//! Decodes persisted rows and rejects corrupt values before they enter the domain model.

use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, Row};

use crate::jobs::{
    JobChunkRecord, JobLedgerError, PreparedRemoteJobRecord, RecordingJobRecord,
    RecordingJobStatus, RecordingRoute, SessionMode, SessionOrigin, SourceOwnership,
};

use super::{
    records::{valid_sha256, validate_opaque_identifier, validate_server_base_url},
    JOB_COLUMNS, MAX_PREPARED_REQUEST_BYTES,
};

pub(super) struct RawJob {
    job_id: String,
    session_mode: String,
    session_origin: String,
    source_path: Option<String>,
    source_ownership: String,
    output_path: Option<String>,
    display_name: String,
    status: String,
    route: Option<String>,
    attempt_count: i64,
    next_attempt_at_ms: Option<i64>,
    cancellation_requested: i64,
    capture_commit_path: Option<String>,
    capture_manifest_sha256: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
    expires_at_ms: Option<i64>,
}

pub(super) fn raw_job_from_row(row: &Row<'_>) -> rusqlite::Result<RawJob> {
    Ok(RawJob {
        job_id: row.get(0)?,
        session_mode: row.get(1)?,
        session_origin: row.get(2)?,
        source_path: row.get(3)?,
        source_ownership: row.get(4)?,
        output_path: row.get(5)?,
        display_name: row.get(6)?,
        status: row.get(7)?,
        route: row.get(8)?,
        attempt_count: row.get(9)?,
        next_attempt_at_ms: row.get(10)?,
        cancellation_requested: row.get(11)?,
        capture_commit_path: row.get(12)?,
        capture_manifest_sha256: row.get(13)?,
        error_code: row.get(14)?,
        error_message: row.get(15)?,
        created_at_ms: row.get(16)?,
        updated_at_ms: row.get(17)?,
        expires_at_ms: row.get(18)?,
    })
}

pub(super) fn query_job(
    connection: &Connection,
    job_id: &str,
) -> Result<Option<RawJob>, JobLedgerError> {
    connection
        .query_row(
            &format!("SELECT {JOB_COLUMNS} FROM recording_jobs WHERE job_id = ?1"),
            [job_id],
            raw_job_from_row,
        )
        .optional()
        .map_err(Into::into)
}

impl TryFrom<RawJob> for RecordingJobRecord {
    type Error = JobLedgerError;

    fn try_from(raw: RawJob) -> Result<Self, Self::Error> {
        Ok(Self {
            job_id: raw.job_id,
            session_mode: SessionMode::from_db(&raw.session_mode)?,
            session_origin: SessionOrigin::from_db(&raw.session_origin)?,
            source_path: raw.source_path.map(PathBuf::from),
            source_ownership: SourceOwnership::from_db(&raw.source_ownership)?,
            output_path: raw.output_path.map(PathBuf::from),
            display_name: raw.display_name,
            status: RecordingJobStatus::from_db(&raw.status)?,
            route: raw
                .route
                .as_deref()
                .map(RecordingRoute::from_db)
                .transpose()?,
            attempt_count: stored_unsigned(raw.attempt_count, "attempt_count")?,
            next_attempt_at_ms: stored_optional_unsigned(
                raw.next_attempt_at_ms,
                "next_attempt_at_ms",
            )?,
            cancellation_requested: stored_bool(
                raw.cancellation_requested,
                "cancellation_requested",
            )?,
            capture_commit_path: raw.capture_commit_path.map(PathBuf::from),
            capture_manifest_sha256: raw.capture_manifest_sha256,
            error_code: raw.error_code,
            error_message: raw.error_message,
            created_at_ms: stored_unsigned(raw.created_at_ms, "created_at_ms")?,
            updated_at_ms: stored_unsigned(raw.updated_at_ms, "updated_at_ms")?,
            expires_at_ms: stored_optional_unsigned(raw.expires_at_ms, "expires_at_ms")?,
        })
    }
}

pub(super) struct RawChunk {
    pub(super) job_id: String,
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

pub(super) struct RawPreparedRemoteJob {
    job_id: String,
    create_request_json: String,
    capture_manifest_path: String,
    capture_manifest_sha256: String,
    server_job_id: Option<String>,
    server_base_url: Option<String>,
    server_cancellation_acknowledged_at_ms: Option<i64>,
    create_attempt_base_url: Option<String>,
}

pub(super) fn raw_prepared_remote_job_from_row(
    row: &Row<'_>,
) -> rusqlite::Result<RawPreparedRemoteJob> {
    Ok(RawPreparedRemoteJob {
        job_id: row.get(0)?,
        create_request_json: row.get(1)?,
        capture_manifest_path: row.get(2)?,
        capture_manifest_sha256: row.get(3)?,
        server_job_id: row.get(4)?,
        server_base_url: row.get(5)?,
        server_cancellation_acknowledged_at_ms: row.get(6)?,
        create_attempt_base_url: row.get(7)?,
    })
}

impl TryFrom<RawPreparedRemoteJob> for PreparedRemoteJobRecord {
    type Error = JobLedgerError;

    fn try_from(raw: RawPreparedRemoteJob) -> Result<Self, Self::Error> {
        if raw.create_request_json.len() < 2
            || raw.create_request_json.len() > MAX_PREPARED_REQUEST_BYTES
            || !serde_json::from_str::<serde_json::Value>(&raw.create_request_json)
                .is_ok_and(|value| value.is_object())
        {
            return Err(JobLedgerError::CorruptValue {
                field: "create_request_json",
                value: "invalid prepared request".into(),
            });
        }
        if !valid_sha256(&raw.capture_manifest_sha256) {
            return Err(JobLedgerError::CorruptValue {
                field: "capture_manifest_sha256",
                value: raw.capture_manifest_sha256,
            });
        }
        match (raw.server_job_id.as_deref(), raw.server_base_url.as_deref()) {
            (Some(server_job_id), Some(server_base_url)) => {
                validate_opaque_identifier(server_job_id, 128, "server job ID")?;
                validate_server_base_url(server_base_url)?;
            }
            (None, None) => {}
            _ => {
                return Err(JobLedgerError::CorruptValue {
                    field: "server_binding",
                    value: "incomplete server binding".into(),
                });
            }
        }
        if let Some(create_attempt_base_url) = raw.create_attempt_base_url.as_deref() {
            validate_server_base_url(create_attempt_base_url)?;
            if raw.server_job_id.is_some() || raw.server_base_url.is_some() {
                return Err(JobLedgerError::CorruptValue {
                    field: "server_binding",
                    value: "create attempt overlaps a completed server binding".into(),
                });
            }
        }
        let capture_manifest_path = PathBuf::from(&raw.capture_manifest_path);
        if !capture_manifest_path.is_absolute() {
            return Err(JobLedgerError::CorruptValue {
                field: "capture_manifest_path",
                value: raw.capture_manifest_path,
            });
        }
        Ok(Self {
            job_id: raw.job_id,
            create_request_json: raw.create_request_json,
            capture_manifest_path,
            capture_manifest_sha256: raw.capture_manifest_sha256,
            create_attempt_base_url: raw.create_attempt_base_url,
            server_job_id: raw.server_job_id,
            server_base_url: raw.server_base_url,
            server_cancellation_acknowledged_at_ms: stored_optional_unsigned(
                raw.server_cancellation_acknowledged_at_ms,
                "server_cancellation_acknowledged_at_ms",
            )?,
        })
    }
}

impl TryFrom<RawChunk> for JobChunkRecord {
    type Error = JobLedgerError;

    fn try_from(raw: RawChunk) -> Result<Self, Self::Error> {
        Ok(Self {
            job_id: raw.job_id,
            owner_namespace: raw.owner_namespace,
            session_id: raw.session_id,
            track_id: raw.track_id,
            sequence_start: stored_unsigned(raw.sequence_start, "sequence_start")?,
            sequence_end: stored_unsigned(raw.sequence_end, "sequence_end")?,
            content_sha256: raw.content_sha256,
            content_byte_length: stored_unsigned(raw.content_byte_length, "content_byte_length")?,
            artifact_path: PathBuf::from(raw.artifact_path),
            upload_offset: stored_unsigned(raw.upload_offset, "upload_offset")?,
            acknowledged_object_id: raw.acknowledged_object_id,
            acknowledged_at_ms: stored_optional_unsigned(
                raw.acknowledged_at_ms,
                "acknowledged_at_ms",
            )?,
        })
    }
}

pub(super) fn stored_unsigned(value: i64, field: &'static str) -> Result<u64, JobLedgerError> {
    u64::try_from(value).map_err(|_| JobLedgerError::CorruptValue {
        field,
        value: value.to_string(),
    })
}

pub(super) fn stored_optional_unsigned(
    value: Option<i64>,
    field: &'static str,
) -> Result<Option<u64>, JobLedgerError> {
    value.map(|value| stored_unsigned(value, field)).transpose()
}

pub(super) fn stored_bool(value: i64, field: &'static str) -> Result<bool, JobLedgerError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(JobLedgerError::CorruptValue {
            field,
            value: value.to_string(),
        }),
    }
}
