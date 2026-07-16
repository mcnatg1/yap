use super::*;

pub struct StreamingRecording {
    pub(super) paths: RecordingPaths,
    pub(super) session_metadata: SessionMetadata,
    pub(super) audio: Option<File>,
    #[cfg(test)]
    pub(super) _reservation_handle_drop_signal: Option<ReservationHandleDropSignal>,
    pub(super) journal_file: Option<File>,
    pub(super) journal: CaptureJournal,
    pub(super) journal_durable: DurableJournalState,
    pub(super) journal_bytes: u64,
    pub(super) journal_growth_stopped: bool,
    pub(super) journal_terminal_written: bool,
    pub(super) data_bytes: u64,
    pub(super) samples_since_sync: u64,
    pub(super) sync_interval_samples: u64,
    pub(super) data_limit: u64,
    pub(super) failure: Option<String>,
    pub(super) sidecar_receipt: Option<PublicationReceipt>,
    pub(super) finalized: Option<RecordingFinalizeResult>,
    #[cfg(test)]
    pub(super) fault: Option<CommitFaultPoint>,
    #[cfg(test)]
    pub(super) after_sidecar_publish: Option<SidecarPublishHook>,
    #[cfg(test)]
    pub(super) publication_hook: Option<PublicationHook>,
    #[cfg(test)]
    pub(super) append_write_attempts: Option<Arc<std::sync::atomic::AtomicUsize>>,
    #[cfg(test)]
    pub(super) journal_write_attempts: Option<Arc<std::sync::atomic::AtomicUsize>>,
}

pub(super) fn default_session_metadata(session_id: SessionId) -> Result<SessionMetadata, String> {
    SessionMetadata::new(
        session_id,
        SessionMode::Dictation,
        SessionOrigin::LiveCapture,
        TriggerMode::PushToTalk,
        std::time::SystemTime::now(),
        None,
        None,
        None,
        Vec::new(),
        None,
    )
}
