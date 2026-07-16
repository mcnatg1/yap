//! Persists prepared remote work and binds it to its exact server origin and job identifier.

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::jobs::{
    JobLedgerError, NewPreparedRemoteJob, PreparedRemoteJobRecord, RecordingJobRecord,
    RecordingJobStatus, RecordingRoute,
};

use super::{
    records::{
        sqlite_integer, validate_opaque_identifier, validate_server_base_url,
        ValidatedPreparedRemoteJob,
    },
    row_mapping::{query_job, raw_prepared_remote_job_from_row},
    JobLedger,
};

impl JobLedger {
    pub fn attach_prepared_remote_job(
        &self,
        job_id: &str,
        prepared: &NewPreparedRemoteJob,
        updated_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        let prepared = ValidatedPreparedRemoteJob::try_from(prepared)?;
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: RecordingJobRecord = query_job(&transaction, job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?
            .try_into()?;
        if current.status != RecordingJobStatus::Preprocessing
            || current.route != Some(RecordingRoute::ServerBatch)
        {
            return Err(JobLedgerError::InvalidTransition {
                from: current.status,
                to: RecordingJobStatus::Uploading,
            });
        }
        if transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM prepared_remote_jobs WHERE job_id = ?1)",
            [job_id],
            |row| row.get::<_, bool>(0),
        )? {
            return Err(JobLedgerError::InvalidRecord(
                "recording job already has prepared remote state",
            ));
        }
        let existing_chunks: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM job_chunks WHERE job_id = ?1",
            [job_id],
            |row| row.get(0),
        )?;
        if existing_chunks != 0 {
            return Err(JobLedgerError::InvalidRecord(
                "recording job already has prepared chunks",
            ));
        }

        transaction.execute(
            "INSERT INTO prepared_remote_jobs (job_id, create_request_json, capture_manifest_path, capture_manifest_sha256) VALUES (?1, ?2, ?3, ?4)",
            params![
                job_id,
                prepared.create_request_json,
                prepared.capture_manifest_path,
                prepared.capture_manifest_sha256,
            ],
        )?;
        for chunk in &prepared.chunks {
            transaction.execute(
                "INSERT INTO job_chunks (job_id, owner_namespace, session_id, track_id, sequence_start, sequence_end, content_sha256, artifact_path, upload_offset, acknowledged_object_id, acknowledged_at_ms, content_byte_length) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    job_id,
                    chunk.owner_namespace,
                    chunk.session_id,
                    chunk.track_id,
                    chunk.sequence_start,
                    chunk.sequence_end,
                    chunk.content_sha256,
                    chunk.artifact_path,
                    chunk.upload_offset,
                    chunk.acknowledged_object_id,
                    chunk.acknowledged_at_ms,
                    chunk.content_byte_length,
                ],
            )?;
        }
        let changed = transaction.execute(
            "UPDATE recording_jobs SET status = 'uploading', capture_manifest_sha256 = ?1, updated_at_ms = ?2 WHERE job_id = ?3 AND status = 'preprocessing' AND route = 'server_batch'",
            params![prepared.capture_manifest_sha256, updated_at_ms, job_id],
        )?;
        if changed != 1 {
            return Err(JobLedgerError::InvalidRecord(
                "recording job preparation lost its state transition",
            ));
        }
        let updated = query_job(&transaction, job_id)?.expect("prepared job exists");
        transaction.commit()?;
        updated.try_into()
    }

    pub fn get_prepared_remote_job(
        &self,
        job_id: &str,
    ) -> Result<Option<PreparedRemoteJobRecord>, JobLedgerError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT job_id, create_request_json, capture_manifest_path, capture_manifest_sha256, server_job_id, server_base_url, server_cancellation_acknowledged_at_ms, create_attempt_base_url FROM prepared_remote_jobs WHERE job_id = ?1",
                [job_id],
                raw_prepared_remote_job_from_row,
            )
            .optional()?
            .map(TryInto::try_into)
            .transpose()
    }

    pub fn begin_remote_create_attempt(
        &self,
        job_id: &str,
        server_base_url: &str,
        updated_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        validate_server_base_url(server_base_url)?;
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: RecordingJobRecord = query_job(&transaction, job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?
            .try_into()?;
        if current.status != RecordingJobStatus::Uploading {
            return Err(JobLedgerError::InvalidTransition {
                from: current.status,
                to: RecordingJobStatus::Uploading,
            });
        }
        let existing: (Option<String>, Option<String>, Option<String>) = transaction
            .query_row(
                "SELECT server_job_id, server_base_url, create_attempt_base_url FROM prepared_remote_jobs WHERE job_id = ?1",
                [job_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(JobLedgerError::InvalidRecord(
                "recording job has no prepared remote state",
            ))?;
        match (
            existing.0.as_deref(),
            existing.1.as_deref(),
            existing.2.as_deref(),
        ) {
            (None, None, Some(attempt)) if attempt == server_base_url => return Ok(current),
            (None, None, None) => {}
            _ => {
                return Err(JobLedgerError::InvalidRecord(
                    "recording job already has a different server create attempt or binding",
                ));
            }
        }
        let changed = transaction.execute(
            "UPDATE prepared_remote_jobs SET create_attempt_base_url = ?1 WHERE job_id = ?2 AND server_job_id IS NULL AND server_base_url IS NULL AND create_attempt_base_url IS NULL",
            params![server_base_url, job_id],
        )?;
        if changed != 1 {
            return Err(JobLedgerError::InvalidRecord(
                "remote create attempt was not durably recorded",
            ));
        }
        transaction.execute(
            "UPDATE recording_jobs SET updated_at_ms = ?1 WHERE job_id = ?2",
            params![updated_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("create-attempt job exists");
        transaction.commit()?;
        updated.try_into()
    }

    pub fn record_server_job_id(
        &self,
        job_id: &str,
        server_job_id: &str,
        server_base_url: &str,
        updated_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        validate_opaque_identifier(server_job_id, 128, "server job ID")?;
        validate_server_base_url(server_base_url)?;
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: RecordingJobRecord = query_job(&transaction, job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?
            .try_into()?;
        let binding_is_factual = current.status == RecordingJobStatus::Uploading
            || (current.status == RecordingJobStatus::Cancelled && current.cancellation_requested);
        if !binding_is_factual {
            return Err(JobLedgerError::InvalidRecord(
                "server response cannot bind this recording job state",
            ));
        }
        let existing: (Option<String>, Option<String>, Option<String>) = transaction
            .query_row(
                "SELECT server_job_id, server_base_url, create_attempt_base_url FROM prepared_remote_jobs WHERE job_id = ?1",
                [job_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(JobLedgerError::InvalidRecord(
                "recording job has no prepared remote state",
            ))?;
        match (
            existing.0.as_deref(),
            existing.1.as_deref(),
            existing.2.as_deref(),
        ) {
            (Some(existing_job), Some(existing_url), None)
                if existing_job == server_job_id && existing_url == server_base_url =>
            {
                return Ok(current);
            }
            (Some(_), Some(_), None) => {
                return Err(JobLedgerError::InvalidRecord(
                    "recording job is already bound to a different server job or origin",
                ));
            }
            (None, None, Some(attempt)) if attempt == server_base_url => {}
            (None, None, Some(_)) => {
                return Err(JobLedgerError::InvalidRecord(
                    "server response origin differs from its durable create attempt",
                ));
            }
            (None, None, None) => {
                return Err(JobLedgerError::InvalidRecord(
                    "server response has no durable create attempt",
                ));
            }
            _ => {
                return Err(JobLedgerError::InvalidRecord(
                    "recording job has an inconsistent server binding",
                ));
            }
        }
        let changed = transaction.execute(
            "UPDATE prepared_remote_jobs SET server_job_id = ?1, server_base_url = ?2, create_attempt_base_url = NULL WHERE job_id = ?3 AND server_job_id IS NULL AND server_base_url IS NULL AND create_attempt_base_url = ?2",
            params![server_job_id, server_base_url, job_id],
        )?;
        if changed != 1 {
            return Err(JobLedgerError::InvalidRecord(
                "server response binding was not durably recorded",
            ));
        }
        transaction.execute(
            "UPDATE recording_jobs SET updated_at_ms = ?1 WHERE job_id = ?2",
            params![updated_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("server-bound job exists");
        transaction.commit()?;
        updated.try_into()
    }
}
