use super::*;

impl StreamingRecording {
    pub fn create(directory: &Path, session_id: SessionId) -> Result<Self, String> {
        Self::create_inner(directory, session_id, None)
    }

    #[cfg(test)]
    pub(crate) fn create_with_session_metadata(
        directory: &Path,
        metadata: SessionMetadata,
    ) -> Result<Self, String> {
        let session_id = metadata.session_id.clone();
        fs::create_dir_all(directory)
            .map_err(|error| format!("Failed to create live recordings folder: {error}"))?;
        let paths = RecordingPaths::new(directory, session_id);
        let audio = create_new(&paths.wav_part, "recording audio")?;
        Self::create_from_open_audio(paths, audio, None, None, metadata)
    }

    pub(crate) fn create_reserved(reservation: RecordingReservation) -> Result<Self, String> {
        let RecordingReservation {
            paths,
            audio,
            identity,
            session_metadata,
            #[cfg(test)]
            handle_drop_signal,
        } = reservation;
        if file_identity(&audio)? != identity {
            return Err(
                "reserved recording audio handle identity changed before worker adoption".into(),
            );
        }
        Self::create_from_open_audio(
            paths,
            audio,
            #[cfg(test)]
            handle_drop_signal,
            None,
            session_metadata,
        )
    }

    #[cfg(test)]
    pub(crate) fn create_with_fault(
        directory: &Path,
        session_id: SessionId,
        fault: CommitFaultPoint,
    ) -> Result<Self, String> {
        Self::create_inner(directory, session_id, Some(fault))
    }

    #[cfg(test)]
    pub(super) fn create_with_sidecar_hook<F>(
        directory: &Path,
        session_id: SessionId,
        hook: F,
    ) -> Result<Self, String>
    where
        F: FnOnce(&RecordingPaths) + Send + 'static,
    {
        let mut recording = Self::create_inner(directory, session_id, None)?;
        recording.after_sidecar_publish = Some(Box::new(hook));
        Ok(recording)
    }

    #[cfg(test)]
    pub(super) fn create_with_publication_hook<F>(
        directory: &Path,
        session_id: SessionId,
        fault: Option<CommitFaultPoint>,
        hook: F,
    ) -> Result<Self, String>
    where
        F: FnMut(PublicationArtifact, PublicationBarrier, &RecordingPaths) + Send + 'static,
    {
        let mut recording = Self::create_inner(directory, session_id, fault)?;
        recording.publication_hook = Some(Box::new(hook));
        Ok(recording)
    }

    fn create_inner(
        directory: &Path,
        session_id: SessionId,
        #[cfg(test)] fault: Option<CommitFaultPoint>,
        #[cfg(not(test))] _fault: Option<()>,
    ) -> Result<Self, String> {
        fs::create_dir_all(directory)
            .map_err(|error| format!("Failed to create live recordings folder: {error}"))?;
        let paths = RecordingPaths::new(directory, session_id.clone());
        let audio = create_new(&paths.wav_part, "recording audio")?;
        let session_metadata = default_session_metadata(session_id)?;
        Self::create_from_open_audio(
            paths,
            audio,
            #[cfg(test)]
            None,
            #[cfg(test)]
            fault,
            #[cfg(not(test))]
            _fault,
            session_metadata,
        )
    }

    fn create_from_open_audio(
        paths: RecordingPaths,
        mut audio: File,
        #[cfg(test)] reservation_handle_drop_signal: Option<ReservationHandleDropSignal>,
        #[cfg(test)] fault: Option<CommitFaultPoint>,
        #[cfg(not(test))] _fault: Option<()>,
        session_metadata: SessionMetadata,
    ) -> Result<Self, String> {
        if audio
            .metadata()
            .map_err(|error| format!("Failed to inspect recording audio: {error}"))?
            .len()
            != 0
        {
            return Err("Reserved recording audio was unexpectedly non-empty".into());
        }
        write_wav_header(&mut audio, 0)?;
        audio
            .sync_data()
            .map_err(|error| format!("Failed to initialize live audio: {error}"))?;
        let mut journal_file = create_new(&paths.journal_part, "recording journal")?;
        let journal = CaptureJournal::new(paths.session_id.clone());
        let journal_bytes = write_journal_record(
            &mut journal_file,
            &JournalRecord::Header {
                journal: journal.clone(),
            },
        )?;
        journal_file
            .sync_data()
            .map_err(|error| format!("Failed to initialize recording journal: {error}"))?;
        Ok(Self {
            paths,
            session_metadata,
            audio: Some(audio),
            #[cfg(test)]
            _reservation_handle_drop_signal: reservation_handle_drop_signal,
            journal_file: Some(journal_file),
            journal_durable: DurableJournalState::from_journal(&journal),
            journal,
            journal_bytes,
            journal_growth_stopped: false,
            journal_terminal_written: false,
            data_bytes: 0,
            samples_since_sync: 0,
            sync_interval_samples: DEFAULT_SYNC_INTERVAL_SAMPLES,
            data_limit: u64::from(u32::MAX),
            failure: None,
            sidecar_receipt: None,
            finalized: None,
            #[cfg(test)]
            fault,
            #[cfg(test)]
            after_sidecar_publish: None,
            #[cfg(test)]
            publication_hook: None,
            #[cfg(test)]
            append_write_attempts: None,
            #[cfg(test)]
            journal_write_attempts: None,
        })
    }
}
