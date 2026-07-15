use super::error::JobLedgerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingJobStatus {
    Accepted,
    Preflighting,
    BlockedSetupRequired,
    BlockedServerUnavailable,
    BlockedSignInRequired,
    QueuedLocalFallback,
    QueuedServer,
    Preprocessing,
    Uploading,
    ServerProcessing,
    LocalTranscribing,
    Saving,
    DiarizationQueued,
    DiarizationRunning,
    Complete,
    Partial,
    Failed,
    Cancelled,
}

impl RecordingJobStatus {
    pub const ALL: [Self; 18] = [
        Self::Accepted,
        Self::Preflighting,
        Self::BlockedSetupRequired,
        Self::BlockedServerUnavailable,
        Self::BlockedSignInRequired,
        Self::QueuedLocalFallback,
        Self::QueuedServer,
        Self::Preprocessing,
        Self::Uploading,
        Self::ServerProcessing,
        Self::LocalTranscribing,
        Self::Saving,
        Self::DiarizationQueued,
        Self::DiarizationRunning,
        Self::Complete,
        Self::Partial,
        Self::Failed,
        Self::Cancelled,
    ];

    pub const fn as_db(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Preflighting => "preflighting",
            Self::BlockedSetupRequired => "blocked_setup_required",
            Self::BlockedServerUnavailable => "blocked_server_unavailable",
            Self::BlockedSignInRequired => "blocked_sign_in_required",
            Self::QueuedLocalFallback => "queued_local_fallback",
            Self::QueuedServer => "queued_server",
            Self::Preprocessing => "preprocessing",
            Self::Uploading => "uploading",
            Self::ServerProcessing => "server_processing",
            Self::LocalTranscribing => "local_transcribing",
            Self::Saving => "saving",
            Self::DiarizationQueued => "diarization_queued",
            Self::DiarizationRunning => "diarization_running",
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, JobLedgerError> {
        match value {
            "accepted" => Ok(Self::Accepted),
            "preflighting" => Ok(Self::Preflighting),
            "blocked_setup_required" => Ok(Self::BlockedSetupRequired),
            "blocked_server_unavailable" => Ok(Self::BlockedServerUnavailable),
            "blocked_sign_in_required" => Ok(Self::BlockedSignInRequired),
            "queued_local_fallback" => Ok(Self::QueuedLocalFallback),
            "queued_server" => Ok(Self::QueuedServer),
            "preprocessing" => Ok(Self::Preprocessing),
            "uploading" => Ok(Self::Uploading),
            "server_processing" => Ok(Self::ServerProcessing),
            "local_transcribing" => Ok(Self::LocalTranscribing),
            "saving" => Ok(Self::Saving),
            "diarization_queued" => Ok(Self::DiarizationQueued),
            "diarization_running" => Ok(Self::DiarizationRunning),
            "complete" => Ok(Self::Complete),
            "partial" => Ok(Self::Partial),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(corrupt("status", value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    Dictation,
    Meeting,
}

impl SessionMode {
    pub const ALL: [Self; 2] = [Self::Dictation, Self::Meeting];
    pub const fn as_db(self) -> &'static str {
        match self {
            Self::Dictation => "dictation",
            Self::Meeting => "meeting",
        }
    }
    pub fn from_db(value: &str) -> Result<Self, JobLedgerError> {
        match value {
            "dictation" => Ok(Self::Dictation),
            "meeting" => Ok(Self::Meeting),
            _ => Err(corrupt("session_mode", value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionOrigin {
    LiveCapture,
    ImportedFile,
}

impl SessionOrigin {
    pub const ALL: [Self; 2] = [Self::LiveCapture, Self::ImportedFile];
    pub const fn as_db(self) -> &'static str {
        match self {
            Self::LiveCapture => "live_capture",
            Self::ImportedFile => "imported_file",
        }
    }
    pub fn from_db(value: &str) -> Result<Self, JobLedgerError> {
        match value {
            "live_capture" => Ok(Self::LiveCapture),
            "imported_file" => Ok(Self::ImportedFile),
            _ => Err(corrupt("session_origin", value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordingRoute {
    LocalFallback,
    ServerBatch,
    ServerLive,
}

impl RecordingRoute {
    pub const ALL: [Self; 3] = [Self::LocalFallback, Self::ServerBatch, Self::ServerLive];
    pub const fn as_db(self) -> &'static str {
        match self {
            Self::LocalFallback => "local_fallback",
            Self::ServerBatch => "server_batch",
            Self::ServerLive => "server_live",
        }
    }
    pub fn from_db(value: &str) -> Result<Self, JobLedgerError> {
        match value {
            "local_fallback" => Ok(Self::LocalFallback),
            "server_batch" => Ok(Self::ServerBatch),
            "server_live" => Ok(Self::ServerLive),
            _ => Err(corrupt("route", value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceOwnership {
    External,
    YapSpool,
}

impl SourceOwnership {
    pub const fn as_db(self) -> &'static str {
        match self {
            Self::External => "external",
            Self::YapSpool => "yap_spool",
        }
    }
    pub fn from_db(value: &str) -> Result<Self, JobLedgerError> {
        match value {
            "external" => Ok(Self::External),
            "yap_spool" => Ok(Self::YapSpool),
            _ => Err(corrupt("source_ownership", value)),
        }
    }
}

fn corrupt(field: &'static str, value: &str) -> JobLedgerError {
    JobLedgerError::CorruptValue {
        field,
        value: value.into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransitionPolicy {
    Ordinary,
    Retry,
    Cancellation,
    Dismiss,
    Forbidden,
}

pub(crate) const fn transition_policy(
    from: RecordingJobStatus,
    to: RecordingJobStatus,
) -> TransitionPolicy {
    use RecordingJobStatus as S;
    match (from, to) {
        (S::Accepted, S::Preflighting) => TransitionPolicy::Ordinary,
        (
            S::Preflighting,
            S::BlockedSetupRequired
            | S::BlockedServerUnavailable
            | S::BlockedSignInRequired
            | S::QueuedLocalFallback
            | S::QueuedServer,
        ) => TransitionPolicy::Ordinary,
        (S::QueuedLocalFallback, S::LocalTranscribing) => TransitionPolicy::Ordinary,
        (S::QueuedServer, S::Preprocessing) => TransitionPolicy::Ordinary,
        (S::Preprocessing, S::Uploading) => TransitionPolicy::Ordinary,
        (S::Uploading, S::ServerProcessing | S::Failed) => TransitionPolicy::Ordinary,
        (S::ServerProcessing, S::Saving | S::DiarizationQueued | S::Failed) => {
            TransitionPolicy::Ordinary
        }
        (S::LocalTranscribing, S::Saving | S::Failed) => TransitionPolicy::Ordinary,
        (S::Saving, S::Complete | S::Partial | S::Failed) => TransitionPolicy::Ordinary,
        (S::DiarizationQueued, S::DiarizationRunning | S::Failed) => TransitionPolicy::Ordinary,
        (S::DiarizationRunning, S::Complete | S::Partial | S::Failed) => TransitionPolicy::Ordinary,
        (
            S::BlockedSetupRequired
            | S::BlockedServerUnavailable
            | S::BlockedSignInRequired
            | S::Failed,
            S::Preflighting,
        ) => TransitionPolicy::Retry,
        (S::Failed, S::Cancelled) => TransitionPolicy::Dismiss,
        (
            S::Accepted
            | S::BlockedSetupRequired
            | S::BlockedServerUnavailable
            | S::BlockedSignInRequired
            | S::QueuedLocalFallback
            | S::QueuedServer
            | S::Preprocessing
            | S::Uploading
            | S::ServerProcessing
            | S::Saving,
            S::Cancelled,
        ) => TransitionPolicy::Cancellation,
        _ => TransitionPolicy::Forbidden,
    }
}
