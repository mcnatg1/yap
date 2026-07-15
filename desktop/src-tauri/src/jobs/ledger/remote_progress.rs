//! Persists upload acknowledgements, commit state, and immutable remote-result publication.

use std::path::Path;

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::jobs::{JobLedgerError, RecordingJobRecord, RecordingJobStatus};

use super::{
    records::{path_text, sqlite_integer, valid_sha256, validate_opaque_identifier},
    retention::prune_terminal_history,
    row_mapping::query_job,
    JobLedger,
};

struct StoredChunkAcknowledgement {
    acknowledged_at_ms: Option<i64>,
    acknowledged_object_id: Option<String>,
    content_byte_length: i64,
    content_sha256: String,
    upload_offset: i64,
}

impl JobLedger {
    #[allow(clippy::too_many_arguments)]
    pub fn acknowledge_remote_chunk(
        &self,
        job_id: &str,
        track_id: &str,
        sequence_start: u64,
        sequence_end: u64,
        content_sha256: &str,
        acknowledged_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        validate_opaque_identifier(track_id, 64, "track ID")?;
        if !valid_sha256(content_sha256) {
            return Err(JobLedgerError::InvalidRecord(
                "chunk acknowledgement requires a lowercase SHA-256 digest",
            ));
        }
        let sequence_start = sqlite_integer(sequence_start, "sequence_start")?;
        let sequence_end = sqlite_integer(sequence_end, "sequence_end")?;
        let acknowledged_at_ms = sqlite_integer(acknowledged_at_ms, "acknowledged_at_ms")?;
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
        let server_job_id: String = transaction
            .query_row(
                "SELECT server_job_id FROM prepared_remote_jobs WHERE job_id = ?1 AND server_job_id IS NOT NULL",
                [job_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(JobLedgerError::InvalidRecord(
                "recording job has not been created on the server",
            ))?;
        let chunk: Option<StoredChunkAcknowledgement> = transaction
            .query_row(
                "SELECT content_sha256, content_byte_length, upload_offset, acknowledged_object_id, acknowledged_at_ms FROM job_chunks WHERE job_id = ?1 AND track_id = ?2 AND sequence_start = ?3 AND sequence_end = ?4",
                params![job_id, track_id, sequence_start, sequence_end],
                |row| {
                    Ok(StoredChunkAcknowledgement {
                        content_sha256: row.get(0)?,
                        content_byte_length: row.get(1)?,
                        upload_offset: row.get(2)?,
                        acknowledged_object_id: row.get(3)?,
                        acknowledged_at_ms: row.get(4)?,
                    })
                },
            )
            .optional()?;
        let Some(chunk) = chunk else {
            return Err(JobLedgerError::InvalidRecord(
                "chunk acknowledgement does not match a prepared replay range",
            ));
        };
        if chunk.content_sha256 != content_sha256 || chunk.content_byte_length <= 0 {
            return Err(JobLedgerError::InvalidRecord(
                "chunk acknowledgement conflicts with prepared content",
            ));
        }
        if chunk.acknowledged_at_ms.is_some() {
            if chunk.upload_offset == chunk.content_byte_length
                && chunk.acknowledged_object_id.as_deref() == Some(server_job_id.as_str())
            {
                return Ok(current);
            }
            return Err(JobLedgerError::InvalidRecord(
                "stored chunk acknowledgement is inconsistent",
            ));
        }
        transaction.execute(
            "UPDATE job_chunks SET upload_offset = content_byte_length, acknowledged_object_id = ?1, acknowledged_at_ms = ?2 WHERE job_id = ?3 AND track_id = ?4 AND sequence_start = ?5 AND sequence_end = ?6 AND acknowledged_at_ms IS NULL",
            params![
                server_job_id,
                acknowledged_at_ms,
                job_id,
                track_id,
                sequence_start,
                sequence_end,
            ],
        )?;
        transaction.execute(
            "UPDATE recording_jobs SET updated_at_ms = ?1 WHERE job_id = ?2",
            params![acknowledged_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("acknowledged job exists");
        transaction.commit()?;
        updated.try_into()
    }

    pub fn mark_remote_job_committed(
        &self,
        job_id: &str,
        updated_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: RecordingJobRecord = query_job(&transaction, job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?
            .try_into()?;
        if current.status == RecordingJobStatus::ServerProcessing {
            return Ok(current);
        }
        if current.status != RecordingJobStatus::Uploading {
            return Err(JobLedgerError::InvalidTransition {
                from: current.status,
                to: RecordingJobStatus::ServerProcessing,
            });
        }
        let has_server_job: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM prepared_remote_jobs WHERE job_id = ?1 AND server_job_id IS NOT NULL)",
            [job_id],
            |row| row.get(0),
        )?;
        let (chunk_count, incomplete_count): (i64, i64) = transaction.query_row(
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN content_byte_length <= 0 OR upload_offset <> content_byte_length OR acknowledged_at_ms IS NULL THEN 1 ELSE 0 END), 0) FROM job_chunks WHERE job_id = ?1",
            [job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if !has_server_job || chunk_count == 0 || incomplete_count != 0 {
            return Err(JobLedgerError::InvalidRecord(
                "recording job cannot commit before every prepared chunk is acknowledged",
            ));
        }
        transaction.execute(
            "UPDATE recording_jobs SET status = 'server_processing', updated_at_ms = ?1 WHERE job_id = ?2 AND status = 'uploading'",
            params![updated_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("committed job exists");
        transaction.commit()?;
        updated.try_into()
    }

    pub fn begin_remote_result_saving(
        &self,
        job_id: &str,
        updated_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: RecordingJobRecord = query_job(&transaction, job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?
            .try_into()?;
        if current.status == RecordingJobStatus::Saving {
            return Ok(current);
        }
        if current.status != RecordingJobStatus::ServerProcessing {
            return Err(JobLedgerError::InvalidTransition {
                from: current.status,
                to: RecordingJobStatus::Saving,
            });
        }
        let has_server_job: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM prepared_remote_jobs WHERE job_id = ?1 AND server_job_id IS NOT NULL)",
            [job_id],
            |row| row.get(0),
        )?;
        if !has_server_job {
            return Err(JobLedgerError::InvalidRecord(
                "remote result cannot be saved without a bound server job",
            ));
        }
        transaction.execute(
            "UPDATE recording_jobs SET status = 'saving', updated_at_ms = ?1 WHERE job_id = ?2 AND status = 'server_processing'",
            params![updated_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("saving job exists");
        transaction.commit()?;
        updated.try_into()
    }

    pub fn complete_remote_result(
        &self,
        job_id: &str,
        output_path: &Path,
        result_expires_at_ms: u64,
        updated_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        let output_path = path_text(output_path, "output_path")?;
        let result_expires_at_ms = sqlite_integer(result_expires_at_ms, "result_expires_at_ms")?;
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        if result_expires_at_ms <= updated_at_ms {
            return Err(JobLedgerError::InvalidRecord(
                "remote result retention must end after completion",
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: RecordingJobRecord = query_job(&transaction, job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?
            .try_into()?;
        if current.status == RecordingJobStatus::Complete {
            if current.output_path.as_deref() == Some(Path::new(&output_path))
                && current.expires_at_ms == Some(result_expires_at_ms as u64)
            {
                return Ok(current);
            }
            return Err(JobLedgerError::InvalidRecord(
                "completed remote job is bound to a different result artifact",
            ));
        }
        if current.status != RecordingJobStatus::Saving {
            return Err(JobLedgerError::InvalidTransition {
                from: current.status,
                to: RecordingJobStatus::Complete,
            });
        }
        let has_server_job: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM prepared_remote_jobs WHERE job_id = ?1 AND server_job_id IS NOT NULL)",
            [job_id],
            |row| row.get(0),
        )?;
        if !has_server_job {
            return Err(JobLedgerError::InvalidRecord(
                "remote result cannot complete without a bound server job",
            ));
        }
        transaction.execute(
            "UPDATE recording_jobs SET status = 'complete', output_path = ?1, error_code = NULL, error_message = NULL, updated_at_ms = ?2, expires_at_ms = ?3 WHERE job_id = ?4 AND status = 'saving'",
            params![output_path, updated_at_ms, result_expires_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("completed remote job exists");
        prune_terminal_history(&transaction, Some(job_id))?;
        transaction.commit()?;
        updated.try_into()
    }
}
