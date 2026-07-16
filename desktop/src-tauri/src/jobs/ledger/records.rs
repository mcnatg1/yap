//! Validates and normalizes caller-owned values before they cross the SQLite boundary.

use std::path::Path;

use crate::jobs::{
    JobLedgerError, NewJobChunk, NewPreparedRemoteJob, NewRecordingJob, RecordingRoute,
    SessionOrigin,
};

use super::{MAX_PREPARED_CHUNKS, MAX_PREPARED_REQUEST_BYTES};

pub(super) struct ValidatedJob {
    pub(super) job_id: String,
    pub(super) session_mode: &'static str,
    pub(super) session_origin: &'static str,
    pub(super) source_path: Option<String>,
    pub(super) source_ownership: &'static str,
    pub(super) output_path: Option<String>,
    pub(super) display_name: String,
    pub(super) status: &'static str,
    pub(super) route: Option<&'static str>,
    pub(super) attempt_count: i64,
    pub(super) next_attempt_at_ms: Option<i64>,
    pub(super) cancellation_requested: i64,
    pub(super) capture_commit_path: Option<String>,
    pub(super) capture_manifest_sha256: Option<String>,
    pub(super) error_code: Option<String>,
    pub(super) error_message: Option<String>,
    pub(super) created_at_ms: i64,
    pub(super) updated_at_ms: i64,
    pub(super) expires_at_ms: Option<i64>,
}

impl TryFrom<&NewRecordingJob> for ValidatedJob {
    type Error = JobLedgerError;

    fn try_from(job: &NewRecordingJob) -> Result<Self, Self::Error> {
        if job.job_id.trim().is_empty() {
            return Err(JobLedgerError::InvalidRecord("job_id must not be empty"));
        }
        if job.display_name.trim().is_empty() {
            return Err(JobLedgerError::InvalidRecord(
                "display_name must not be empty",
            ));
        }
        if job.session_origin == SessionOrigin::ImportedFile && job.source_path.is_none() {
            return Err(JobLedgerError::InvalidRecord(
                "imported recording jobs require source_path",
            ));
        }
        Ok(Self {
            job_id: job.job_id.clone(),
            session_mode: job.session_mode.as_db(),
            session_origin: job.session_origin.as_db(),
            source_path: optional_path_text(job.source_path.as_deref(), "source_path")?,
            source_ownership: job.source_ownership.as_db(),
            output_path: optional_path_text(job.output_path.as_deref(), "output_path")?,
            display_name: job.display_name.clone(),
            status: job.status.as_db(),
            route: job.route.map(RecordingRoute::as_db),
            attempt_count: sqlite_integer(job.attempt_count, "attempt_count")?,
            next_attempt_at_ms: optional_sqlite_integer(
                job.next_attempt_at_ms,
                "next_attempt_at_ms",
            )?,
            cancellation_requested: i64::from(job.cancellation_requested),
            capture_commit_path: optional_path_text(
                job.capture_commit_path.as_deref(),
                "capture_commit_path",
            )?,
            capture_manifest_sha256: job.capture_manifest_sha256.clone(),
            error_code: job.error_code.clone(),
            error_message: job.error_message.clone(),
            created_at_ms: sqlite_integer(job.created_at_ms, "created_at_ms")?,
            updated_at_ms: sqlite_integer(job.updated_at_ms, "updated_at_ms")?,
            expires_at_ms: optional_sqlite_integer(job.expires_at_ms, "expires_at_ms")?,
        })
    }
}

pub(super) struct ValidatedChunk {
    pub(super) owner_namespace: String,
    pub(super) session_id: String,
    pub(super) track_id: String,
    pub(super) sequence_start: i64,
    pub(super) sequence_end: i64,
    pub(super) content_sha256: String,
    pub(super) content_byte_length: i64,
    pub(super) artifact_path: String,
    pub(super) upload_offset: i64,
    pub(super) acknowledged_object_id: Option<String>,
    pub(super) acknowledged_at_ms: Option<i64>,
}

impl TryFrom<&NewJobChunk> for ValidatedChunk {
    type Error = JobLedgerError;

    fn try_from(chunk: &NewJobChunk) -> Result<Self, Self::Error> {
        let sequence_start = sqlite_integer(chunk.sequence_start, "sequence_start")?;
        let sequence_end = sqlite_integer(chunk.sequence_end, "sequence_end")?;
        if sequence_end < sequence_start {
            return Err(JobLedgerError::InvalidRecord(
                "chunk sequence_end must be at least sequence_start",
            ));
        }
        Ok(Self {
            owner_namespace: chunk.owner_namespace.clone(),
            session_id: chunk.session_id.clone(),
            track_id: chunk.track_id.clone(),
            sequence_start,
            sequence_end,
            content_sha256: chunk.content_sha256.clone(),
            content_byte_length: sqlite_integer(chunk.content_byte_length, "content_byte_length")?,
            artifact_path: path_text(&chunk.artifact_path, "artifact_path")?,
            upload_offset: sqlite_integer(chunk.upload_offset, "upload_offset")?,
            acknowledged_object_id: chunk.acknowledged_object_id.clone(),
            acknowledged_at_ms: optional_sqlite_integer(
                chunk.acknowledged_at_ms,
                "acknowledged_at_ms",
            )?,
        })
    }
}

pub(super) struct ValidatedPreparedRemoteJob {
    pub(super) create_request_json: String,
    pub(super) capture_manifest_path: String,
    pub(super) capture_manifest_sha256: String,
    pub(super) chunks: Vec<ValidatedChunk>,
}

impl TryFrom<&NewPreparedRemoteJob> for ValidatedPreparedRemoteJob {
    type Error = JobLedgerError;

    fn try_from(prepared: &NewPreparedRemoteJob) -> Result<Self, Self::Error> {
        if prepared.create_request_json.len() < 2
            || prepared.create_request_json.len() > MAX_PREPARED_REQUEST_BYTES
            || !serde_json::from_str::<serde_json::Value>(&prepared.create_request_json)
                .is_ok_and(|value| value.is_object())
        {
            return Err(JobLedgerError::InvalidRecord(
                "prepared create request must be a bounded JSON object",
            ));
        }
        if !valid_sha256(&prepared.capture_manifest_sha256) {
            return Err(JobLedgerError::InvalidRecord(
                "prepared capture manifest requires a lowercase SHA-256 digest",
            ));
        }
        if prepared.chunks.is_empty() || prepared.chunks.len() > MAX_PREPARED_CHUNKS {
            return Err(JobLedgerError::InvalidRecord(
                "prepared remote job has an invalid chunk count",
            ));
        }
        let chunks = prepared
            .chunks
            .iter()
            .map(ValidatedChunk::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        if chunks.iter().any(|chunk| {
            chunk.content_byte_length <= 0
                || chunk.upload_offset != 0
                || chunk.acknowledged_object_id.is_some()
                || chunk.acknowledged_at_ms.is_some()
        }) {
            return Err(JobLedgerError::InvalidRecord(
                "new prepared chunks cannot already be acknowledged",
            ));
        }
        let owner_namespace = &chunks[0].owner_namespace;
        let session_id = &chunks[0].session_id;
        if chunks.iter().any(|chunk| {
            &chunk.owner_namespace != owner_namespace || &chunk.session_id != session_id
        }) {
            return Err(JobLedgerError::InvalidRecord(
                "prepared chunks must share one owner and session",
            ));
        }
        Ok(Self {
            create_request_json: prepared.create_request_json.clone(),
            capture_manifest_path: path_text(
                &prepared.capture_manifest_path,
                "capture_manifest_path",
            )?,
            capture_manifest_sha256: prepared.capture_manifest_sha256.clone(),
            chunks,
        })
    }
}

pub(super) fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub(super) fn validate_opaque_identifier(
    value: &str,
    maximum: usize,
    _label: &'static str,
) -> Result<(), JobLedgerError> {
    if value.is_empty()
        || value.len() > maximum
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(JobLedgerError::InvalidRecord(
            "remote identifier is outside the server contract",
        ));
    }
    Ok(())
}

pub(super) fn validate_server_base_url(value: &str) -> Result<(), JobLedgerError> {
    let normalized = crate::server_connector::batch::validate_development_batch_base_url(value)
        .map_err(|_| {
            JobLedgerError::InvalidRecord(
                "server base URL is outside the development transport contract",
            )
        })?;
    if normalized != value {
        return Err(JobLedgerError::InvalidRecord(
            "server base URL is outside the development transport contract",
        ));
    }
    Ok(())
}

pub(super) fn path_text(path: &Path, field: &'static str) -> Result<String, JobLedgerError> {
    if !path.is_absolute() {
        return Err(JobLedgerError::InvalidPath {
            field,
            path: path.to_owned(),
        });
    }
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| JobLedgerError::InvalidPath {
            field,
            path: path.to_owned(),
        })
}

pub(super) fn optional_path_text(
    path: Option<&Path>,
    field: &'static str,
) -> Result<Option<String>, JobLedgerError> {
    path.map(|path| path_text(path, field)).transpose()
}

pub(super) fn sqlite_integer(value: u64, field: &'static str) -> Result<i64, JobLedgerError> {
    i64::try_from(value).map_err(|_| JobLedgerError::OutOfRange { field, value })
}

pub(super) fn optional_sqlite_integer(
    value: Option<u64>,
    field: &'static str,
) -> Result<Option<i64>, JobLedgerError> {
    value.map(|value| sqlite_integer(value, field)).transpose()
}
