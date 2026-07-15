use super::status::{
    RecordingJobStatus, RecordingRoute, SessionMode, SessionOrigin, SourceOwnership,
};
use std::path::PathBuf;

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
    pub create_attempt_base_url: Option<String>,
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
