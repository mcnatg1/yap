use std::{fmt, path::PathBuf};

#[derive(Debug)]
pub enum JobLedgerError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    CorruptValue {
        field: &'static str,
        value: String,
    },
    OutOfRange {
        field: &'static str,
        value: u64,
    },
    InvalidPath {
        field: &'static str,
        path: PathBuf,
    },
    InvalidRecord(&'static str),
    PragmaNotApplied {
        pragma: &'static str,
        requested: &'static str,
        actual: String,
    },
    UnsupportedSchema(i64),
    NotFound(String),
    InvalidTransition {
        from: RecordingJobStatus,
        to: RecordingJobStatus,
    },
    RetryRequired,
    CancellationRequired,
    DismissRequired,
    LockPoisoned,
}

impl fmt::Display for JobLedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "job ledger database error: {error}"),
            Self::Io(error) => write!(formatter, "job ledger filesystem error: {error}"),
            Self::CorruptValue { field, value } => {
                write!(formatter, "job ledger has invalid {field} value {value:?}")
            }
            Self::OutOfRange { field, value } => {
                write!(
                    formatter,
                    "{field} value {value} exceeds SQLite's signed integer range"
                )
            }
            Self::InvalidPath { field, path } => {
                write!(
                    formatter,
                    "{field} must be an absolute UTF-8 path: {}",
                    path.display()
                )
            }
            Self::InvalidRecord(message) => formatter.write_str(message),
            Self::PragmaNotApplied {
                pragma,
                requested,
                actual,
            } => write!(
                formatter,
                "job ledger requested {requested} for {pragma}, but SQLite applied {actual}"
            ),
            Self::UnsupportedSchema(version) => {
                write!(
                    formatter,
                    "job ledger uses unsupported schema version {version}"
                )
            }
            Self::NotFound(job_id) => write!(formatter, "recording job {job_id:?} was not found"),
            Self::InvalidTransition { from, to } => {
                write!(
                    formatter,
                    "recording job cannot transition from {from:?} to {to:?}"
                )
            }
            Self::RetryRequired => {
                formatter.write_str("retry transitions must use JobLedger::retry")
            }
            Self::CancellationRequired => formatter
                .write_str("cancellation transitions must use JobLedger::request_cancellation"),
            Self::DismissRequired => {
                formatter.write_str("dismiss transitions must use JobLedger::dismiss_failed")
            }
            Self::LockPoisoned => formatter.write_str("job ledger connection lock is poisoned"),
        }
    }
}

impl std::error::Error for JobLedgerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for JobLedgerError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for JobLedgerError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRecordingJob {
    pub job_id: String,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub source_path: Option<PathBuf>,
    pub source_ownership: SourceOwnership,
    pub output_path: Option<PathBuf>,
    pub display_name: String,
    pub status: RecordingJobStatus,
    pub route: Option<RecordingRoute>,
    pub attempt_count: u64,
    pub next_attempt_at_ms: Option<u64>,
    pub cancellation_requested: bool,
    pub capture_commit_path: Option<PathBuf>,
    pub capture_manifest_sha256: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingJobRecord {
    pub job_id: String,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub source_path: Option<PathBuf>,
    pub source_ownership: SourceOwnership,
    pub output_path: Option<PathBuf>,
    pub display_name: String,
    pub status: RecordingJobStatus,
    pub route: Option<RecordingRoute>,
    pub attempt_count: u64,
    pub next_attempt_at_ms: Option<u64>,
    pub cancellation_requested: bool,
    pub capture_commit_path: Option<PathBuf>,
    pub capture_manifest_sha256: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJobChunk {
    pub owner_namespace: String,
    pub session_id: String,
    pub track_id: String,
    pub sequence_start: u64,
    pub sequence_end: u64,
    pub content_sha256: String,
    pub content_byte_length: u64,
    pub artifact_path: PathBuf,
    pub upload_offset: u64,
    pub acknowledged_object_id: Option<String>,
    pub acknowledged_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobChunkRecord {
    pub job_id: String,
    pub owner_namespace: String,
    pub session_id: String,
    pub track_id: String,
    pub sequence_start: u64,
    pub sequence_end: u64,
    pub content_sha256: String,
    pub content_byte_length: u64,
    pub artifact_path: PathBuf,
    pub upload_offset: u64,
    pub acknowledged_object_id: Option<String>,
    pub acknowledged_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPreparedRemoteJob {
    pub create_request_json: String,
    pub capture_manifest_path: PathBuf,
    pub capture_manifest_sha256: String,
    pub chunks: Vec<NewJobChunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRemoteJobRecord {
    pub job_id: String,
    pub create_request_json: String,
    pub capture_manifest_path: PathBuf,
    pub capture_manifest_sha256: String,
    pub server_job_id: Option<String>,
    pub server_base_url: Option<String>,
    pub server_cancellation_acknowledged_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachedRemoteCancellationRecord {
    pub server_base_url: String,
    pub server_job_id: String,
    pub create_request_json: String,
    pub queued_at_ms: u64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PipelineStageStatus {
    NotStarted,
    Queued,
    Running,
    Done,
    Error,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingPipelineState {
    pub intake: PipelineStageStatus,
    pub preprocessing: PipelineStageStatus,
    pub transcription: PipelineStageStatus,
    pub alignment: PipelineStageStatus,
    pub diarization: PipelineStageStatus,
    pub postprocessing: PipelineStageStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingJobView {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playback_path: Option<String>,
    pub name: String,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub status: RecordingJobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<RecordingRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub pipeline: RecordingPipelineState,
}

impl RecordingJobView {
    pub fn from_record(record: &RecordingJobRecord) -> Self {
        Self {
            id: record.job_id.clone(),
            source_path: record
                .source_path
                .as_ref()
                .map(|path| path.display().to_string()),
            playback_path: None,
            name: record.display_name.clone(),
            session_mode: record.session_mode,
            session_origin: record.session_origin,
            status: record.status,
            route: record.route,
            output_path: record
                .output_path
                .as_ref()
                .map(|path| path.display().to_string()),
            error: record
                .error_message
                .clone()
                .or_else(|| record.error_code.clone()),
            pipeline: pipeline_for(record.status, record.route),
        }
    }
}

fn pipeline_for(
    status: RecordingJobStatus,
    route: Option<RecordingRoute>,
) -> RecordingPipelineState {
    use PipelineStageStatus as P;
    use RecordingJobStatus as S;
    let mut pipeline = RecordingPipelineState {
        intake: P::Done,
        preprocessing: P::NotStarted,
        transcription: P::NotStarted,
        alignment: P::NotStarted,
        diarization: P::NotStarted,
        postprocessing: P::NotStarted,
    };
    match status {
        S::QueuedLocalFallback => pipeline.preprocessing = P::Skipped,
        S::Preprocessing => pipeline.preprocessing = P::Running,
        S::Uploading => pipeline.preprocessing = P::Done,
        S::ServerProcessing => {
            pipeline.preprocessing = P::Done;
            pipeline.transcription = P::Running;
        }
        S::LocalTranscribing => {
            pipeline.preprocessing = P::Skipped;
            pipeline.transcription = P::Running;
        }
        S::Saving => {
            pipeline.preprocessing = completed_preprocessing(route);
            pipeline.transcription = P::Done;
            pipeline.postprocessing = P::Running;
        }
        S::DiarizationQueued => {
            pipeline.preprocessing = P::Done;
            pipeline.transcription = P::Done;
            pipeline.alignment = P::Done;
            pipeline.diarization = P::Queued;
        }
        S::DiarizationRunning => {
            pipeline.preprocessing = P::Done;
            pipeline.transcription = P::Done;
            pipeline.alignment = P::Done;
            pipeline.diarization = P::Running;
        }
        S::Complete => {
            pipeline.preprocessing = completed_preprocessing(route);
            pipeline.transcription = P::Done;
            pipeline.postprocessing = P::Done;
        }
        S::Partial => {
            pipeline.preprocessing = completed_preprocessing(route);
            pipeline.transcription = P::Done;
        }
        S::Failed => pipeline.preprocessing = completed_preprocessing(route),
        S::Accepted
        | S::Preflighting
        | S::BlockedSetupRequired
        | S::BlockedServerUnavailable
        | S::BlockedSignInRequired
        | S::QueuedServer
        | S::Cancelled => {}
    }
    pipeline
}

fn completed_preprocessing(route: Option<RecordingRoute>) -> PipelineStageStatus {
    match route {
        Some(RecordingRoute::LocalFallback) => PipelineStageStatus::Skipped,
        Some(RecordingRoute::ServerBatch | RecordingRoute::ServerLive) => PipelineStageStatus::Done,
        None => PipelineStageStatus::NotStarted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_enums_round_trip_and_reject_unknown_values() {
        for status in RecordingJobStatus::ALL {
            assert_eq!(RecordingJobStatus::from_db(status.as_db()).unwrap(), status);
        }
        for mode in SessionMode::ALL {
            assert_eq!(SessionMode::from_db(mode.as_db()).unwrap(), mode);
        }
        for origin in SessionOrigin::ALL {
            assert_eq!(SessionOrigin::from_db(origin.as_db()).unwrap(), origin);
        }
        for route in RecordingRoute::ALL {
            assert_eq!(RecordingRoute::from_db(route.as_db()).unwrap(), route);
        }
        assert!(matches!(
            RecordingJobStatus::from_db("invented_ui_state"),
            Err(JobLedgerError::CorruptValue {
                field: "status",
                ..
            })
        ));
        assert!(SessionMode::from_db("podcast").is_err());
        assert!(SessionOrigin::from_db("cloud_upload").is_err());
        assert!(RecordingRoute::from_db("direct").is_err());
    }

    #[test]
    fn transition_policy_is_pure_and_cancellation_is_narrow() {
        assert_eq!(
            transition_policy(
                RecordingJobStatus::Accepted,
                RecordingJobStatus::Preflighting
            ),
            TransitionPolicy::Ordinary
        );
        assert_eq!(
            transition_policy(RecordingJobStatus::Failed, RecordingJobStatus::Preflighting),
            TransitionPolicy::Retry
        );
        assert_eq!(
            transition_policy(RecordingJobStatus::Failed, RecordingJobStatus::Uploading),
            TransitionPolicy::Forbidden
        );
        assert_eq!(
            transition_policy(RecordingJobStatus::Failed, RecordingJobStatus::Cancelled),
            TransitionPolicy::Dismiss
        );

        for status in [
            RecordingJobStatus::Accepted,
            RecordingJobStatus::BlockedSetupRequired,
            RecordingJobStatus::BlockedServerUnavailable,
            RecordingJobStatus::BlockedSignInRequired,
            RecordingJobStatus::QueuedLocalFallback,
            RecordingJobStatus::QueuedServer,
            RecordingJobStatus::Preprocessing,
            RecordingJobStatus::Uploading,
            RecordingJobStatus::ServerProcessing,
            RecordingJobStatus::Saving,
        ] {
            assert_eq!(
                transition_policy(status, RecordingJobStatus::Cancelled),
                TransitionPolicy::Cancellation,
                "{status:?} should be cancellable"
            );
        }
        for status in [
            RecordingJobStatus::Preflighting,
            RecordingJobStatus::LocalTranscribing,
            RecordingJobStatus::DiarizationQueued,
            RecordingJobStatus::DiarizationRunning,
            RecordingJobStatus::Complete,
            RecordingJobStatus::Partial,
            RecordingJobStatus::Cancelled,
        ] {
            assert_eq!(
                transition_policy(status, RecordingJobStatus::Cancelled),
                TransitionPolicy::Forbidden,
                "{status:?} must not be cancellable"
            );
        }
    }

    #[test]
    fn view_projection_uses_the_camel_case_typescript_contract() {
        let record = fixture_record();
        let value = serde_json::to_value(RecordingJobView::from_record(&record)).unwrap();

        assert_eq!(value["id"], "job-view");
        assert_eq!(
            value["sourcePath"],
            record.source_path.unwrap().display().to_string()
        );
        assert_eq!(value["name"], "Board meeting");
        assert_eq!(value["sessionMode"], "meeting");
        assert_eq!(value["sessionOrigin"], "importedFile");
        assert_eq!(value["status"], "queued_server");
        assert_eq!(value["route"], "serverBatch");
        assert!(value.get("job_id").is_none());
        assert!(value.get("attempt_count").is_none());
    }

    #[test]
    fn every_durable_status_projects_only_proven_pipeline_progress() {
        use PipelineStageStatus::{
            Done as D, NotStarted as N, Queued as Q, Running as R, Skipped as S,
        };
        use RecordingJobStatus as J;
        use RecordingRoute::{LocalFallback as Local, ServerBatch as Server};

        let pipeline = |preprocessing, transcription, alignment, diarization, postprocessing| {
            RecordingPipelineState {
                intake: D,
                preprocessing,
                transcription,
                alignment,
                diarization,
                postprocessing,
            }
        };
        let cases = [
            (J::Accepted, None, pipeline(N, N, N, N, N)),
            (J::Preflighting, None, pipeline(N, N, N, N, N)),
            (J::BlockedSetupRequired, None, pipeline(N, N, N, N, N)),
            (
                J::BlockedServerUnavailable,
                Some(Server),
                pipeline(N, N, N, N, N),
            ),
            (
                J::BlockedSignInRequired,
                Some(Server),
                pipeline(N, N, N, N, N),
            ),
            (J::QueuedLocalFallback, Some(Local), pipeline(S, N, N, N, N)),
            (J::QueuedServer, Some(Server), pipeline(N, N, N, N, N)),
            (J::Preprocessing, Some(Server), pipeline(R, N, N, N, N)),
            (J::Uploading, Some(Server), pipeline(D, N, N, N, N)),
            (J::ServerProcessing, Some(Server), pipeline(D, R, N, N, N)),
            (J::LocalTranscribing, Some(Local), pipeline(S, R, N, N, N)),
            (J::Saving, Some(Server), pipeline(D, D, N, N, R)),
            (J::DiarizationQueued, Some(Server), pipeline(D, D, D, Q, N)),
            (J::DiarizationRunning, Some(Server), pipeline(D, D, D, R, N)),
            (J::Complete, Some(Server), pipeline(D, D, N, N, D)),
            (J::Partial, Some(Server), pipeline(D, D, N, N, N)),
            (J::Failed, Some(Server), pipeline(D, N, N, N, N)),
            (J::Cancelled, Some(Server), pipeline(N, N, N, N, N)),
            (J::Saving, Some(Local), pipeline(S, D, N, N, R)),
            (J::Complete, Some(Local), pipeline(S, D, N, N, D)),
        ];

        for (status, route, expected) in cases {
            let mut record = fixture_record();
            record.status = status;
            record.route = route;

            assert_eq!(
                RecordingJobView::from_record(&record).pipeline,
                expected,
                "unexpected projection for {status:?} with {route:?}"
            );
        }
    }

    fn fixture_record() -> RecordingJobRecord {
        RecordingJobRecord {
            job_id: "job-view".into(),
            session_mode: SessionMode::Meeting,
            session_origin: SessionOrigin::ImportedFile,
            source_path: Some(std::env::temp_dir().join("meeting.wav")),
            source_ownership: SourceOwnership::External,
            output_path: None,
            display_name: "Board meeting".into(),
            status: RecordingJobStatus::QueuedServer,
            route: Some(RecordingRoute::ServerBatch),
            attempt_count: 0,
            next_attempt_at_ms: None,
            cancellation_requested: false,
            capture_commit_path: None,
            capture_manifest_sha256: None,
            error_code: None,
            error_message: None,
            created_at_ms: 100,
            updated_at_ms: 100,
            expires_at_ms: None,
        }
    }
}
