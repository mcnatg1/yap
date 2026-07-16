use super::*;

#[derive(Debug, Clone)]
pub(super) struct RecordingPaths {
    pub(super) directory: PathBuf,
    pub(super) session_id: SessionId,
    pub(super) wav_part: PathBuf,
    pub(super) journal_part: PathBuf,
    pub(super) wav: PathBuf,
    pub(super) sidecar: PathBuf,
    pub(super) sidecar_part: PathBuf,
    pub(super) partial_sidecar: PathBuf,
    pub(super) partial_sidecar_part: PathBuf,
    pub(super) commit: PathBuf,
    pub(super) commit_part: PathBuf,
}

impl RecordingPaths {
    pub(super) fn new(directory: &Path, session_id: SessionId) -> Self {
        let prefix = format!("live-{session_id}");
        Self {
            directory: directory.to_path_buf(),
            session_id,
            wav_part: directory.join(format!("{prefix}.wav.part")),
            journal_part: directory.join(format!("{prefix}.capture.journal.part")),
            wav: directory.join(format!("{prefix}.wav")),
            sidecar: directory.join(format!("{prefix}.capture.json")),
            sidecar_part: directory.join(format!("{prefix}.capture.json.part")),
            partial_sidecar: directory.join(format!("{prefix}.capture.partial.json")),
            partial_sidecar_part: directory.join(format!("{prefix}.capture.partial.json.part")),
            commit: directory.join(format!("{prefix}.commit.json")),
            commit_part: directory.join(format!("{prefix}.commit.json.part")),
        }
    }

    pub(super) fn wav_file_name(&self) -> String {
        self.wav.file_name().unwrap().to_string_lossy().into_owned()
    }

    pub(super) fn sidecar_file_name(&self) -> String {
        self.sidecar
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    pub(super) fn partial_sidecar_file_name(&self) -> String {
        self.partial_sidecar
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[cfg(test)]
    pub(super) fn path_for_publication(
        &self,
        artifact: PublicationArtifact,
        barrier: PublicationBarrier,
    ) -> PathBuf {
        match (artifact, barrier) {
            (PublicationArtifact::CompleteSidecar, PublicationBarrier::BeforeHardLink) => {
                self.sidecar_part.clone()
            }
            (PublicationArtifact::CompleteSidecar, PublicationBarrier::AfterHardLink) => {
                self.sidecar.clone()
            }
            (PublicationArtifact::PartialSidecar, PublicationBarrier::BeforeHardLink) => {
                self.partial_sidecar_part.clone()
            }
            (PublicationArtifact::PartialSidecar, PublicationBarrier::AfterHardLink) => {
                self.partial_sidecar.clone()
            }
            (PublicationArtifact::Commit, PublicationBarrier::BeforeHardLink) => {
                self.commit_part.clone()
            }
            (PublicationArtifact::Commit, PublicationBarrier::AfterHardLink) => self.commit.clone(),
            (PublicationArtifact::Audio, PublicationBarrier::BeforeHardLink) => {
                self.wav_part.clone()
            }
            (PublicationArtifact::Audio, PublicationBarrier::AfterHardLink) => self.wav.clone(),
        }
    }
}

/// The durable claim passed directly from allocation into the recording worker.
/// Its audio handle is the only authority permitted to write the reserved WAV.
#[derive(Debug)]
pub(crate) struct RecordingReservation {
    pub(super) paths: RecordingPaths,
    pub(super) audio: File,
    pub(super) identity: FileIdentity,
    pub(super) session_metadata: SessionMetadata,
    #[cfg(test)]
    pub(super) handle_drop_signal: Option<ReservationHandleDropSignal>,
}

impl RecordingReservation {
    pub(crate) fn session_id(&self) -> &SessionId {
        &self.paths.session_id
    }

    pub(crate) fn with_session_metadata(
        mut self,
        metadata: SessionMetadata,
    ) -> Result<Self, String> {
        validate_capture_metadata(&metadata, self.session_id())?;
        self.session_metadata = metadata;
        Ok(self)
    }

    #[cfg(test)]
    pub(super) fn wav_part(&self) -> &Path {
        &self.paths.wav_part
    }

    #[cfg(test)]
    pub(super) fn watch_handle_drop_for_test(
        &mut self,
        dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        self.handle_drop_signal = Some(ReservationHandleDropSignal(dropped));
    }
}

#[cfg(test)]
#[derive(Debug)]
pub(super) struct ReservationHandleDropSignal(std::sync::Arc<std::sync::atomic::AtomicBool>);

#[cfg(test)]
impl Drop for ReservationHandleDropSignal {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Allocates a persistent recording identity by atomically reserving its first
/// on-disk artifact. Runtime-local counters are deliberately not part of this ID.
pub(crate) fn allocate_recording_session(directory: &Path) -> Result<RecordingReservation, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("Failed to create live recordings folder: {error}"))?;
    let mut reservation = None;
    session::allocate_recording(|session_id| {
        let claimed = reserve_wav_part(directory, session_id)?;
        reservation = Some(claimed);
        Ok(())
    })?;
    reservation.ok_or_else(|| "recording allocation returned without a reservation".to_string())
}

pub(super) fn reserve_wav_part(
    directory: &Path,
    session_id: &SessionId,
) -> std::io::Result<RecordingReservation> {
    reserve_wav_part_with_before_claim(directory, session_id, || {})
}

pub(super) fn reserve_wav_part_with_before_claim<F>(
    directory: &Path,
    session_id: &SessionId,
    before_claim: F,
) -> std::io::Result<RecordingReservation>
where
    F: FnOnce(),
{
    let paths = RecordingPaths::new(directory, session_id.clone());
    let prefix = format!("live-{session_id}.");
    let conflicting_artifact = fs::read_dir(directory)?.any(|entry| {
        let Ok(entry) = entry else {
            return true;
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.starts_with(&prefix)
    });
    if conflicting_artifact {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "recording artifact prefix already exists",
        ));
    }
    before_claim();
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&paths.wav_part)?;
    file.sync_all()?;
    let identity = file_identity(&file).map_err(std::io::Error::other)?;
    let session_metadata =
        default_session_metadata(session_id.clone()).map_err(std::io::Error::other)?;
    Ok(RecordingReservation {
        paths,
        audio: file,
        identity,
        session_metadata,
        #[cfg(test)]
        handle_drop_signal: None,
    })
}
