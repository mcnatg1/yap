use super::*;

impl StreamingRecording {
    pub fn finalize(&mut self) -> Result<RecordingFinalizeResult, String> {
        if let Some(result) = &self.finalized {
            return Ok(result.clone());
        }
        if self.journal.sink_degraded && self.failure.is_none() {
            self.abort("recording sequence metadata is degraded".into());
        }
        if self.failure.is_some() {
            return Ok(self.partial_result());
        }

        let manifest = match self.finalize_inner() {
            Ok(manifest) => manifest,
            Err(error) => {
                self.failure = Some(error);
                return Ok(self.partial_result());
            }
        };
        let result = RecordingFinalizeResult {
            session_id: self.paths.session_id.clone(),
            status: CaptureStatus::Complete,
            committed: Some(CommittedCapture {
                manifest,
                directory: self.paths.directory.clone(),
            }),
            partial_lineage: None,
            error: None,
            sidecar_receipt: self.sidecar_receipt.clone(),
        };
        self.finalized = Some(result.clone());
        Ok(result)
    }

    fn finalize_inner(&mut self) -> Result<CaptureCommitManifest, String> {
        let mut audio = self
            .audio
            .take()
            .ok_or_else(|| "Live recording audio is unavailable".to_string())?;
        self.hit_fault(CommitFaultPoint::WavHeaderPatch)?;
        write_wav_header(&mut audio, self.data_bytes)?;
        self.hit_fault(CommitFaultPoint::AudioSync)?;
        audio
            .sync_all()
            .map_err(|error| format!("Failed to sync finalized live audio: {error}"))?;

        self.persist_journal()?;

        self.hit_fault(CommitFaultPoint::FinalArtifactRename)?;
        let wav_part = self.paths.wav_part.clone();
        let wav = self.paths.wav.clone();
        let mut published_audio = self.publish_owned(
            &wav_part,
            &wav,
            &audio,
            "finalize live audio",
            PublicationArtifact::Audio,
            CommitFaultPoint::AudioStagingCleanup,
        )?;
        let audio_bytes = published_audio
            .metadata()
            .map_err(|error| format!("Failed to inspect finalized live audio: {error}"))?
            .len();
        let audio_sha256 = sha256_open_file(&mut published_audio)?;
        drop(published_audio);
        drop(audio);

        let sidecar = CaptureSidecar {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id: self.paths.session_id.clone(),
            audio_file: self.paths.wav_file_name(),
            audio_sha256,
            audio_bytes,
            tracks: self.journal.tracks.values().cloned().collect(),
            track_configurations: self.journal.track_configurations.clone(),
            clock_mappings: self.journal.clock_mappings.clone(),
            timeline_gaps: self.journal.timeline_gaps.clone(),
            sequence_coverage: self.journal.sequence_coverage.clone(),
            sequence_gaps: self.journal.sequence_gaps.clone(),
            sequence_gap_overflow: self.journal.sequence_gap_overflow.clone(),
            sink_degraded: self.journal.sink_degraded,
            directory_sync_supported: sync_parent_directory(&self.paths.directory),
            session_metadata: Some(self.session_metadata.clone()),
        };
        validate_audio_metadata_presence(sidecar.audio_bytes, &sidecar.tracks)?;
        validate_timeline_control_metadata(
            &sidecar.session_id,
            &sidecar.tracks,
            &sidecar.track_configurations,
            &sidecar.clock_mappings,
            &sidecar.timeline_gaps,
        )?;
        validate_sequence_metadata(
            &sidecar.tracks,
            &sidecar.sequence_coverage,
            &sidecar.sequence_gaps,
            sidecar.sequence_gap_overflow.as_ref(),
            sidecar.sink_degraded,
        )?;
        let sidecar_file =
            write_json_file_open(&self.paths.sidecar_part, &sidecar, "capture sidecar")?;
        self.hit_fault(CommitFaultPoint::SidecarSync)?;
        sidecar_file
            .sync_all()
            .map_err(|error| format!("Failed to sync capture sidecar: {error}"))?;
        let sidecar_part = self.paths.sidecar_part.clone();
        let sidecar_path = self.paths.sidecar.clone();
        let published_sidecar = self.publish_owned(
            &sidecar_part,
            &sidecar_path,
            &sidecar_file,
            "publish capture sidecar",
            PublicationArtifact::CompleteSidecar,
            CommitFaultPoint::SidecarStagingCleanup,
        )?;
        let sidecar_receipt = receipt_from_published_sidecar(
            published_sidecar,
            self.paths.sidecar_file_name(),
            self.paths.sidecar.clone(),
            &sidecar,
        )?;
        drop(sidecar_file);
        self.sidecar_receipt = Some(sidecar_receipt.clone());
        #[cfg(test)]
        if let Some(hook) = self.after_sidecar_publish.take() {
            hook(&self.paths);
        }
        self.revalidate_sidecar_receipt()?;

        let manifest = CaptureCommitManifest {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id: self.paths.session_id.clone(),
            status: CaptureStatus::Complete,
            audio_file: self.paths.wav_file_name(),
            audio_sha256: sidecar.audio_sha256,
            audio_bytes,
            capture_sidecar_file: self.paths.sidecar_file_name(),
            capture_sidecar_sha256: sidecar_receipt.sha256,
            committed_at_utc: now_utc()?,
            session_metadata: Some(self.session_metadata.clone()),
        };
        manifest.validate()?;
        let commit_file =
            write_json_file_open(&self.paths.commit_part, &manifest, "capture commit")?;
        self.hit_fault(CommitFaultPoint::CommitSync)?;
        commit_file
            .sync_all()
            .map_err(|error| format!("Failed to sync capture commit: {error}"))?;
        self.hit_fault(CommitFaultPoint::CommitRename)?;
        self.revalidate_sidecar_receipt()?;
        let commit_part = self.paths.commit_part.clone();
        let commit = self.paths.commit.clone();
        let mut published_commit = self.publish_owned(
            &commit_part,
            &commit,
            &commit_file,
            "publish capture commit",
            PublicationArtifact::Commit,
            CommitFaultPoint::CommitStagingCleanup,
        )?;
        let manifest = manifest_from_published_commit(&mut published_commit, &manifest)?;
        self.revalidate_sidecar_receipt()?;
        self.remove_owned_journal_after_commit();
        let _ = sync_parent_directory(&self.paths.directory);
        Ok(manifest)
    }

    // The journal is recovery state. Keep its original handle until publication so a
    // pathname replacement cannot cause us to remove somebody else's file.
    fn remove_owned_journal_after_commit(&mut self) {
        let Some(journal) = self.journal_file.take() else {
            return;
        };
        let name = self
            .paths
            .journal_part
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let result = if name.is_empty() {
            Err("recording journal has no valid file name".to_string())
        } else {
            remove_open_regular_artifact(&self.paths.directory, name, &journal, || {})
        };
        drop(journal);
        if let Err(error) = result {
            crate::stt::log_yap(&format!(
                "Published capture commit, but journal cleanup is pending: {error}"
            ));
        }
    }

    fn partial_result(&mut self) -> RecordingFinalizeResult {
        let partial_lineage = self.publish_partial_lineage();
        let error = match (self.failure.clone(), partial_lineage.as_ref()) {
            (Some(error), Ok(_)) => Some(error),
            (Some(error), Err(lineage_error)) => Some(format!(
                "{error}; failed to publish partial capture lineage: {lineage_error}"
            )),
            (None, Err(lineage_error)) => Some(format!(
                "Failed to publish partial capture lineage: {lineage_error}"
            )),
            (None, Ok(_)) => None,
        };
        let result = RecordingFinalizeResult {
            session_id: self.paths.session_id.clone(),
            status: CaptureStatus::Partial,
            committed: None,
            partial_lineage: partial_lineage.ok(),
            error,
            sidecar_receipt: self.sidecar_receipt.clone(),
        };
        self.finalized = Some(result.clone());
        result
    }

    fn publish_partial_lineage(&mut self) -> Result<PartialCaptureLineage, String> {
        if let Some(receipt) = &self.sidecar_receipt {
            if receipt.revalidate().is_ok() {
                return Ok(receipt.lineage());
            }
            self.sidecar_receipt = None;
        }

        let sidecar = PartialCaptureSidecar {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id: self.paths.session_id.clone(),
            status: CaptureStatus::Partial,
        };
        let partial_sidecar_file = write_json_file_open(
            &self.paths.partial_sidecar_part,
            &sidecar,
            "partial capture sidecar",
        )?;
        partial_sidecar_file
            .sync_all()
            .map_err(|error| format!("Failed to sync partial capture sidecar: {error}"))?;
        let partial_sidecar_part = self.paths.partial_sidecar_part.clone();
        let partial_sidecar = self.paths.partial_sidecar.clone();
        let published_sidecar = self.publish_owned(
            &partial_sidecar_part,
            &partial_sidecar,
            &partial_sidecar_file,
            "publish partial capture sidecar",
            PublicationArtifact::PartialSidecar,
            CommitFaultPoint::SidecarStagingCleanup,
        )?;
        let receipt = receipt_from_published_partial_sidecar(
            published_sidecar,
            self.paths.partial_sidecar_file_name(),
            self.paths.partial_sidecar.clone(),
            &sidecar,
        )?;
        let _ = sync_parent_directory(&self.paths.directory);
        self.sidecar_receipt = Some(receipt.clone());
        Ok(receipt.lineage())
    }

    pub(super) fn abort(&mut self, reason: String) {
        self.failure.get_or_insert(reason);
    }

    pub(super) fn fail<T>(&mut self, error: String) -> Result<T, String> {
        Err(self.failure.get_or_insert(error).clone())
    }
}
