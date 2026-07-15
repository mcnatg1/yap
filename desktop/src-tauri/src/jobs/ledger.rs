use crate::jobs::{
    migrations,
    model::{transition_policy, TransitionPolicy},
    DetachedRemoteCancellationRecord, JobChunkRecord, JobLedgerError, NewJobChunk,
    NewPreparedRemoteJob, NewRecordingJob, PreparedRemoteJobRecord, RecordingJobRecord,
    RecordingJobStatus, RecordingRoute, SessionMode, SessionOrigin, SourceOwnership,
};
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

const JOB_COLUMNS: &str = "job_id, session_mode, session_origin, source_path, source_ownership, output_path, display_name, status, route, attempt_count, next_attempt_at_ms, cancellation_requested, capture_commit_path, capture_manifest_sha256, error_code, error_message, created_at_ms, updated_at_ms, expires_at_ms";
const MAX_TERMINAL_JOB_HISTORY: usize = 500;
const MAX_PREPARED_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_PREPARED_CHUNKS: usize = 4096;

struct StoredChunkAcknowledgement {
    acknowledged_at_ms: Option<i64>,
    acknowledged_object_id: Option<String>,
    content_byte_length: i64,
    content_sha256: String,
    upload_offset: i64,
}

pub struct JobLedger {
    pub(super) connection: Mutex<Connection>,
}

impl JobLedger {
    pub fn open_default() -> Result<Self, JobLedgerError> {
        Self::open(crate::paths::app_data_dir().join("jobs.sqlite3"))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, JobLedgerError> {
        let mut connection = migrations::open_file(path.as_ref())?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        prune_terminal_history(&transaction, None)?;
        transaction.commit()?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    #[cfg(test)]
    pub(super) fn open_in_memory() -> Result<Self, JobLedgerError> {
        let mut connection = migrations::open_in_memory()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        prune_terminal_history(&transaction, None)?;
        transaction.commit()?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn insert_job(&self, job: &NewRecordingJob) -> Result<RecordingJobRecord, JobLedgerError> {
        self.insert_job_with_chunks(job, &[])
    }

    pub fn insert_jobs(
        &self,
        jobs: &[NewRecordingJob],
    ) -> Result<Vec<RecordingJobRecord>, JobLedgerError> {
        let jobs = jobs
            .iter()
            .map(ValidatedJob::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for job in &jobs {
            transaction.execute(
                "INSERT INTO recording_jobs (job_id, session_mode, session_origin, source_path, source_ownership, output_path, display_name, status, route, attempt_count, next_attempt_at_ms, cancellation_requested, capture_commit_path, capture_manifest_sha256, error_code, error_message, created_at_ms, updated_at_ms, expires_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
                params![
                    job.job_id,
                    job.session_mode,
                    job.session_origin,
                    job.source_path,
                    job.source_ownership,
                    job.output_path,
                    job.display_name,
                    job.status,
                    job.route,
                    job.attempt_count,
                    job.next_attempt_at_ms,
                    job.cancellation_requested,
                    job.capture_commit_path,
                    job.capture_manifest_sha256,
                    job.error_code,
                    job.error_message,
                    job.created_at_ms,
                    job.updated_at_ms,
                    job.expires_at_ms,
                ],
            )?;
        }
        let records = jobs
            .iter()
            .map(|job| {
                query_job(&transaction, &job.job_id)?
                    .expect("inserted job exists")
                    .try_into()
            })
            .collect::<Result<Vec<_>, JobLedgerError>>()?;
        transaction.commit()?;
        Ok(records)
    }

    pub fn insert_job_with_chunks(
        &self,
        job: &NewRecordingJob,
        chunks: &[NewJobChunk],
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        // Validation and integer conversion deliberately finish before the lock and transaction.
        let job = ValidatedJob::try_from(job)?;
        let chunks = chunks
            .iter()
            .map(ValidatedChunk::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO recording_jobs (job_id, session_mode, session_origin, source_path, source_ownership, output_path, display_name, status, route, attempt_count, next_attempt_at_ms, cancellation_requested, capture_commit_path, capture_manifest_sha256, error_code, error_message, created_at_ms, updated_at_ms, expires_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                job.job_id,
                job.session_mode,
                job.session_origin,
                job.source_path,
                job.source_ownership,
                job.output_path,
                job.display_name,
                job.status,
                job.route,
                job.attempt_count,
                job.next_attempt_at_ms,
                job.cancellation_requested,
                job.capture_commit_path,
                job.capture_manifest_sha256,
                job.error_code,
                job.error_message,
                job.created_at_ms,
                job.updated_at_ms,
                job.expires_at_ms,
            ],
        )?;
        for chunk in &chunks {
            transaction.execute(
                "INSERT INTO job_chunks (job_id, owner_namespace, session_id, track_id, sequence_start, sequence_end, content_sha256, artifact_path, upload_offset, acknowledged_object_id, acknowledged_at_ms, content_byte_length) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    job.job_id,
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
        transaction.commit()?;
        drop(connection);
        self.get_job(&job.job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job.job_id))
    }

    pub fn get_job(&self, job_id: &str) -> Result<Option<RecordingJobRecord>, JobLedgerError> {
        let connection = self.lock()?;
        query_job(&connection, job_id)?
            .map(TryInto::try_into)
            .transpose()
    }

    pub fn find_recoverable_imported_job_by_source(
        &self,
        source_path: &Path,
    ) -> Result<Option<RecordingJobRecord>, JobLedgerError> {
        let source_path = path_text(source_path, "source_path")?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {JOB_COLUMNS} FROM recording_jobs WHERE session_origin = 'imported_file' AND source_path = ?1 AND status NOT IN ('complete', 'partial', 'cancelled') ORDER BY created_at_ms, job_id LIMIT 1"
        ))?;
        statement
            .query_row([source_path], raw_job_from_row)
            .optional()?
            .map(TryInto::try_into)
            .transpose()
    }

    pub fn list_recoverable_jobs(&self) -> Result<Vec<RecordingJobRecord>, JobLedgerError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {JOB_COLUMNS} FROM recording_jobs WHERE status NOT IN ('complete', 'partial', 'cancelled') ORDER BY created_at_ms, job_id"
        ))?;
        let rows = statement.query_map([], raw_job_from_row)?;
        rows.map(|row| {
            row.map_err(JobLedgerError::from)
                .and_then(TryInto::try_into)
        })
        .collect()
    }

    pub fn list_jobs(&self) -> Result<Vec<RecordingJobRecord>, JobLedgerError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {JOB_COLUMNS} FROM recording_jobs ORDER BY created_at_ms, job_id"
        ))?;
        let rows = statement.query_map([], raw_job_from_row)?;
        rows.map(|row| {
            row.map_err(JobLedgerError::from)
                .and_then(TryInto::try_into)
        })
        .collect()
    }

    pub fn list_chunks(&self, job_id: &str) -> Result<Vec<JobChunkRecord>, JobLedgerError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT job_id, owner_namespace, session_id, track_id, sequence_start, sequence_end, content_sha256, artifact_path, upload_offset, acknowledged_object_id, acknowledged_at_ms, content_byte_length FROM job_chunks WHERE job_id = ?1 ORDER BY track_id, sequence_start, sequence_end",
        )?;
        let rows = statement.query_map([job_id], |row| {
            Ok(RawChunk {
                job_id: row.get(0)?,
                owner_namespace: row.get(1)?,
                session_id: row.get(2)?,
                track_id: row.get(3)?,
                sequence_start: row.get(4)?,
                sequence_end: row.get(5)?,
                content_sha256: row.get(6)?,
                artifact_path: row.get(7)?,
                upload_offset: row.get(8)?,
                acknowledged_object_id: row.get(9)?,
                acknowledged_at_ms: row.get(10)?,
                content_byte_length: row.get(11)?,
            })
        })?;
        rows.map(|row| {
            row.map_err(JobLedgerError::from)
                .and_then(TryInto::try_into)
        })
        .collect()
    }

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
                "SELECT job_id, create_request_json, capture_manifest_path, capture_manifest_sha256, server_job_id, server_base_url, server_cancellation_acknowledged_at_ms FROM prepared_remote_jobs WHERE job_id = ?1",
                [job_id],
                raw_prepared_remote_job_from_row,
            )
            .optional()?
            .map(TryInto::try_into)
            .transpose()
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
        if current.status != RecordingJobStatus::Uploading {
            return Err(JobLedgerError::InvalidTransition {
                from: current.status,
                to: RecordingJobStatus::Uploading,
            });
        }
        let existing: (Option<String>, Option<String>) = transaction
            .query_row(
                "SELECT server_job_id, server_base_url FROM prepared_remote_jobs WHERE job_id = ?1",
                [job_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(JobLedgerError::InvalidRecord(
                "recording job has no prepared remote state",
            ))?;
        match (existing.0.as_deref(), existing.1.as_deref()) {
            (Some(existing_job), Some(existing_url))
                if existing_job == server_job_id && existing_url == server_base_url =>
            {
                return Ok(current);
            }
            (Some(_), Some(_)) => {
                return Err(JobLedgerError::InvalidRecord(
                    "recording job is already bound to a different server job or origin",
                ));
            }
            (None, None) => {}
            _ => {
                return Err(JobLedgerError::InvalidRecord(
                    "recording job has an incomplete server binding",
                ));
            }
        }
        transaction.execute(
            "UPDATE prepared_remote_jobs SET server_job_id = ?1, server_base_url = ?2 WHERE job_id = ?3 AND server_job_id IS NULL AND server_base_url IS NULL",
            params![server_job_id, server_base_url, job_id],
        )?;
        transaction.execute(
            "UPDATE recording_jobs SET updated_at_ms = ?1 WHERE job_id = ?2",
            params![updated_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("server-bound job exists");
        transaction.commit()?;
        updated.try_into()
    }

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

    pub fn transition(
        &self,
        job_id: &str,
        to: RecordingJobStatus,
        updated_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw = query_job(&transaction, job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?;
        let current: RecordingJobRecord = raw.try_into()?;
        match transition_policy(current.status, to) {
            TransitionPolicy::Ordinary => {}
            TransitionPolicy::Retry => return Err(JobLedgerError::RetryRequired),
            TransitionPolicy::Cancellation => return Err(JobLedgerError::CancellationRequired),
            TransitionPolicy::Dismiss => return Err(JobLedgerError::DismissRequired),
            TransitionPolicy::Forbidden => {
                return Err(JobLedgerError::InvalidTransition {
                    from: current.status,
                    to,
                });
            }
        }
        transaction.execute(
            "UPDATE recording_jobs SET status = ?1, updated_at_ms = ?2 WHERE job_id = ?3",
            params![to.as_db(), updated_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("updated job exists");
        if matches!(
            to,
            RecordingJobStatus::Complete
                | RecordingJobStatus::Partial
                | RecordingJobStatus::Cancelled
        ) {
            prune_terminal_history(&transaction, Some(job_id))?;
        }
        transaction.commit()?;
        updated.try_into()
    }

    pub fn accept_to_queued_server(
        &self,
        job_id: &str,
        updated_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let expires_at_ms = sqlite_integer(expires_at_ms, "expires_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw = query_job(&transaction, job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?;
        let current: RecordingJobRecord = raw.try_into()?;
        if current.status != RecordingJobStatus::Accepted
            || transition_policy(current.status, RecordingJobStatus::Preflighting)
                != TransitionPolicy::Ordinary
            || transition_policy(
                RecordingJobStatus::Preflighting,
                RecordingJobStatus::QueuedServer,
            ) != TransitionPolicy::Ordinary
        {
            return Err(JobLedgerError::InvalidTransition {
                from: current.status,
                to: RecordingJobStatus::QueuedServer,
            });
        }
        transaction.execute(
            "UPDATE recording_jobs SET status = 'queued_server', route = 'server_batch', updated_at_ms = ?1, expires_at_ms = ?2 WHERE job_id = ?3",
            params![updated_at_ms, expires_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("accepted queued job exists");
        transaction.commit()?;
        updated.try_into()
    }

    pub fn retry(
        &self,
        job_id: &str,
        updated_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        self.retry_with_expiry(job_id, updated_at_ms, None)
    }

    pub fn retry_with_expiry(
        &self,
        job_id: &str,
        updated_at_ms: u64,
        expires_at_ms: Option<u64>,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        self.retry_to_status(
            job_id,
            updated_at_ms,
            expires_at_ms,
            RecordingJobStatus::Preflighting,
        )
    }

    pub fn retry_to_queued_server(
        &self,
        job_id: &str,
        updated_at_ms: u64,
        expires_at_ms: Option<u64>,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        self.retry_to_status(
            job_id,
            updated_at_ms,
            expires_at_ms,
            RecordingJobStatus::QueuedServer,
        )
    }

    fn retry_to_status(
        &self,
        job_id: &str,
        updated_at_ms: u64,
        expires_at_ms: Option<u64>,
        final_status: RecordingJobStatus,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let expires_at_ms = optional_sqlite_integer(expires_at_ms, "expires_at_ms")?;
        loop {
            let mut connection = self.lock()?;
            let raw = query_job(&connection, job_id)?
                .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?;
            let current: RecordingJobRecord = raw.try_into()?;
            if transition_policy(current.status, RecordingJobStatus::Preflighting)
                != TransitionPolicy::Retry
            {
                return Err(JobLedgerError::InvalidTransition {
                    from: current.status,
                    to: RecordingJobStatus::Preflighting,
                });
            }
            let expected_attempt_count = sqlite_integer(current.attempt_count, "attempt_count")?;
            let next_attempt_count =
                current
                    .attempt_count
                    .checked_add(1)
                    .ok_or(JobLedgerError::OutOfRange {
                        field: "attempt_count",
                        value: u64::MAX,
                    })?;
            let next_attempt_count = sqlite_integer(next_attempt_count, "attempt_count")?;

            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if final_status == RecordingJobStatus::QueuedServer {
                let detached_binding: Option<(String, String, String)> = transaction
                    .query_row(
                        "SELECT server_base_url, server_job_id, create_request_json FROM prepared_remote_jobs WHERE job_id = ?1 AND server_job_id IS NOT NULL AND server_base_url IS NOT NULL AND server_cancellation_acknowledged_at_ms IS NULL",
                        [job_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                if let Some((server_base_url, server_job_id, create_request_json)) =
                    detached_binding
                {
                    let inserted = transaction.execute(
                        "INSERT OR IGNORE INTO detached_remote_cancellations (server_base_url, server_job_id, create_request_json, queued_at_ms) VALUES (?1, ?2, ?3, ?4)",
                        params![
                            server_base_url,
                            server_job_id,
                            create_request_json,
                            updated_at_ms,
                        ],
                    )?;
                    if inserted == 0 {
                        let existing: String = transaction.query_row(
                            "SELECT create_request_json FROM detached_remote_cancellations WHERE server_base_url = ?1 AND server_job_id = ?2",
                            params![server_base_url, server_job_id],
                            |row| row.get(0),
                        )?;
                        if existing != create_request_json {
                            return Err(JobLedgerError::InvalidRecord(
                                "detached server cancellation conflicts with an existing outbox entry",
                            ));
                        }
                    }
                }
                transaction.execute("DELETE FROM job_chunks WHERE job_id = ?1", [job_id])?;
                transaction.execute(
                    "DELETE FROM prepared_remote_jobs WHERE job_id = ?1",
                    [job_id],
                )?;
            }
            let changed = transaction.execute(
                "UPDATE recording_jobs SET status = ?1, attempt_count = ?2, next_attempt_at_ms = NULL, cancellation_requested = 0, output_path = NULL, capture_commit_path = NULL, capture_manifest_sha256 = NULL, error_code = NULL, error_message = NULL, updated_at_ms = ?3, expires_at_ms = COALESCE(?4, expires_at_ms) WHERE job_id = ?5 AND status = ?6 AND attempt_count = ?7 AND attempt_count < ?8",
                params![
                    final_status.as_db(),
                    next_attempt_count,
                    updated_at_ms,
                    expires_at_ms,
                    job_id,
                    current.status.as_db(),
                    expected_attempt_count,
                    i64::MAX,
                ],
            )?;
            if changed == 0 {
                transaction.rollback()?;
                continue;
            }
            let updated = query_job(&transaction, job_id)?.expect("retried job exists");
            transaction.commit()?;
            return updated.try_into();
        }
    }

    pub fn request_cancellation(
        &self,
        job_id: &str,
        updated_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw = query_job(&transaction, job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?;
        let current: RecordingJobRecord = raw.try_into()?;
        if transition_policy(current.status, RecordingJobStatus::Cancelled)
            != TransitionPolicy::Cancellation
        {
            return Err(JobLedgerError::InvalidTransition {
                from: current.status,
                to: RecordingJobStatus::Cancelled,
            });
        }
        transaction.execute(
            "UPDATE recording_jobs SET status = 'cancelled', cancellation_requested = 1, updated_at_ms = ?1 WHERE job_id = ?2",
            params![updated_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("cancelled job exists");
        prune_terminal_history(&transaction, Some(job_id))?;
        transaction.commit()?;
        updated.try_into()
    }

    pub fn list_pending_remote_cancellations(
        &self,
    ) -> Result<Vec<PreparedRemoteJobRecord>, JobLedgerError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT prepared.job_id, prepared.create_request_json, prepared.capture_manifest_path, prepared.capture_manifest_sha256, prepared.server_job_id, prepared.server_base_url, prepared.server_cancellation_acknowledged_at_ms FROM prepared_remote_jobs AS prepared JOIN recording_jobs AS job ON job.job_id = prepared.job_id WHERE job.status = 'cancelled' AND job.cancellation_requested = 1 AND prepared.server_job_id IS NOT NULL AND prepared.server_base_url IS NOT NULL AND prepared.server_cancellation_acknowledged_at_ms IS NULL ORDER BY job.updated_at_ms, prepared.job_id",
        )?;
        let rows = statement.query_map([], raw_prepared_remote_job_from_row)?;
        rows.map(|row| {
            row.map_err(JobLedgerError::from)
                .and_then(TryInto::try_into)
        })
        .collect()
    }

    pub fn list_detached_remote_cancellations(
        &self,
    ) -> Result<Vec<DetachedRemoteCancellationRecord>, JobLedgerError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT server_base_url, server_job_id, create_request_json, queued_at_ms FROM detached_remote_cancellations ORDER BY queued_at_ms, server_base_url, server_job_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (server_base_url, server_job_id, create_request_json, queued_at_ms) = row?;
            validate_server_base_url(&server_base_url)?;
            validate_opaque_identifier(&server_job_id, 128, "server job ID")?;
            if !(2..=MAX_PREPARED_REQUEST_BYTES).contains(&create_request_json.len()) {
                return Err(JobLedgerError::InvalidRecord(
                    "detached cancellation request is outside the persisted contract",
                ));
            }
            Ok(DetachedRemoteCancellationRecord {
                server_base_url,
                server_job_id,
                create_request_json,
                queued_at_ms: stored_unsigned(queued_at_ms, "queued_at_ms")?,
            })
        })
        .collect()
    }

    pub fn acknowledge_detached_remote_cancellation(
        &self,
        server_base_url: &str,
        server_job_id: &str,
    ) -> Result<(), JobLedgerError> {
        validate_server_base_url(server_base_url)?;
        validate_opaque_identifier(server_job_id, 128, "server job ID")?;
        let connection = self.lock()?;
        connection.execute(
            "DELETE FROM detached_remote_cancellations WHERE server_base_url = ?1 AND server_job_id = ?2",
            params![server_base_url, server_job_id],
        )?;
        Ok(())
    }

    pub fn acknowledge_server_cancellation(
        &self,
        job_id: &str,
        server_job_id: &str,
        acknowledged_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        validate_opaque_identifier(server_job_id, 128, "server job ID")?;
        let acknowledged_at_ms = sqlite_integer(acknowledged_at_ms, "acknowledged_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: RecordingJobRecord = query_job(&transaction, job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?
            .try_into()?;
        if current.status != RecordingJobStatus::Cancelled || !current.cancellation_requested {
            return Err(JobLedgerError::InvalidRecord(
                "server cancellation acknowledgement requires a cancelled remote job",
            ));
        }
        let binding: (String, Option<i64>) = transaction
            .query_row(
                "SELECT server_job_id, server_cancellation_acknowledged_at_ms FROM prepared_remote_jobs WHERE job_id = ?1 AND server_job_id IS NOT NULL",
                [job_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(JobLedgerError::InvalidRecord(
                "cancelled remote job has no server binding",
            ))?;
        if binding.0 != server_job_id {
            return Err(JobLedgerError::InvalidRecord(
                "server cancellation acknowledgement conflicts with the bound job",
            ));
        }
        if binding.1.is_some() {
            return Ok(current);
        }
        transaction.execute(
            "UPDATE prepared_remote_jobs SET server_cancellation_acknowledged_at_ms = ?1 WHERE job_id = ?2 AND server_job_id = ?3 AND server_cancellation_acknowledged_at_ms IS NULL",
            params![acknowledged_at_ms, job_id, server_job_id],
        )?;
        transaction.execute(
            "UPDATE recording_jobs SET updated_at_ms = ?1 WHERE job_id = ?2",
            params![acknowledged_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("cancelled remote job exists");
        prune_terminal_history(&transaction, Some(job_id))?;
        transaction.commit()?;
        updated.try_into()
    }

    pub fn record_remote_error(
        &self,
        job_id: &str,
        error_code: &str,
        error_message: &str,
        retry_at_ms: Option<u64>,
        updated_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        validate_opaque_identifier(error_code, 64, "remote error code")?;
        if error_message.is_empty()
            || error_message.len() > 512
            || error_message
                .chars()
                .any(|character| character.is_control() && !character.is_whitespace())
        {
            return Err(JobLedgerError::InvalidRecord(
                "remote error message is outside the ledger contract",
            ));
        }
        let retry_at_ms = optional_sqlite_integer(retry_at_ms, "next_attempt_at_ms")?;
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: RecordingJobRecord = query_job(&transaction, job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?
            .try_into()?;
        if !matches!(
            current.status,
            RecordingJobStatus::Preprocessing
                | RecordingJobStatus::Uploading
                | RecordingJobStatus::ServerProcessing
                | RecordingJobStatus::Saving
        ) {
            return Err(JobLedgerError::InvalidRecord(
                "remote errors can only be recorded for an active remote job",
            ));
        }
        let next_attempt_count =
            current
                .attempt_count
                .checked_add(1)
                .ok_or(JobLedgerError::OutOfRange {
                    field: "attempt_count",
                    value: u64::MAX,
                })?;
        let next_attempt_count = sqlite_integer(next_attempt_count, "attempt_count")?;
        let status = if retry_at_ms.is_some() {
            current.status.as_db()
        } else {
            RecordingJobStatus::Failed.as_db()
        };
        transaction.execute(
            "UPDATE recording_jobs SET status = ?1, attempt_count = ?2, next_attempt_at_ms = ?3, error_code = ?4, error_message = ?5, updated_at_ms = ?6 WHERE job_id = ?7",
            params![
                status,
                next_attempt_count,
                retry_at_ms,
                error_code,
                error_message,
                updated_at_ms,
                job_id,
            ],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("errored remote job exists");
        transaction.commit()?;
        updated.try_into()
    }

    pub fn dismiss_failed(
        &self,
        job_id: &str,
        updated_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw = query_job(&transaction, job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.into()))?;
        let current: RecordingJobRecord = raw.try_into()?;
        if transition_policy(current.status, RecordingJobStatus::Cancelled)
            != TransitionPolicy::Dismiss
        {
            return Err(JobLedgerError::InvalidTransition {
                from: current.status,
                to: RecordingJobStatus::Cancelled,
            });
        }
        transaction.execute(
            "UPDATE recording_jobs SET status = 'cancelled', cancellation_requested = 1, updated_at_ms = ?1 WHERE job_id = ?2 AND status = 'failed'",
            params![updated_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("dismissed job exists");
        prune_terminal_history(&transaction, Some(job_id))?;
        transaction.commit()?;
        updated.try_into()
    }

    pub fn fail_source_validation(
        &self,
        job_id: &str,
        error_code: &str,
        updated_at_ms: u64,
    ) -> Result<RecordingJobRecord, JobLedgerError> {
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if query_job(&transaction, job_id)?.is_none() {
            return Err(JobLedgerError::NotFound(job_id.into()));
        }
        transaction.execute(
            "UPDATE recording_jobs SET status = 'failed', error_code = ?1, error_message = NULL, updated_at_ms = ?2 WHERE job_id = ?3",
            params![error_code, updated_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("failed job exists");
        transaction.commit()?;
        updated.try_into()
    }

    pub fn expire_pending_jobs(&self, now_ms: u64) -> Result<usize, JobLedgerError> {
        let now_ms = sqlite_integer(now_ms, "updated_at_ms")?;
        let connection = self.lock()?;
        Ok(connection.execute(
            "UPDATE recording_jobs SET status = 'failed', error_code = 'PENDING_EXPIRED', error_message = NULL, updated_at_ms = ?1 WHERE expires_at_ms IS NOT NULL AND expires_at_ms <= ?1 AND status IN ('accepted', 'preflighting', 'blocked_setup_required', 'blocked_server_unavailable', 'blocked_sign_in_required', 'queued_local_fallback', 'queued_server')",
            [now_ms],
        )?)
    }

    pub fn enforce_remote_retention(
        &self,
        now_ms: u64,
    ) -> Result<(Vec<String>, usize), JobLedgerError> {
        let now_ms = sqlite_integer(now_ms, "updated_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job_ids = {
            let mut statement = transaction.prepare(
                "SELECT job_id FROM recording_jobs WHERE route = 'server_batch' AND expires_at_ms IS NOT NULL AND expires_at_ms <= ?1 ORDER BY created_at_ms, job_id",
            )?;
            let rows = statement
                .query_map([now_ms], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let cancelled = transaction.execute(
            "UPDATE recording_jobs SET status = 'cancelled', cancellation_requested = 1, next_attempt_at_ms = NULL, updated_at_ms = ?1 WHERE route = 'server_batch' AND expires_at_ms IS NOT NULL AND expires_at_ms <= ?1 AND status IN ('preprocessing', 'uploading', 'server_processing', 'saving')",
            [now_ms],
        )?;
        let expired_completed = transaction.execute(
            "UPDATE recording_jobs SET status = 'cancelled', cancellation_requested = 0, next_attempt_at_ms = NULL, output_path = NULL, updated_at_ms = ?1 WHERE route = 'server_batch' AND expires_at_ms IS NOT NULL AND expires_at_ms <= ?1 AND status IN ('complete', 'partial')",
            [now_ms],
        )?;
        prune_terminal_history(&transaction, None)?;
        transaction.commit()?;
        Ok((job_ids, cancelled.saturating_add(expired_completed)))
    }

    pub fn list_pending_remote_spool_cleanup(&self) -> Result<Vec<String>, JobLedgerError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT job_id FROM remote_spool_cleanup ORDER BY queued_at_ms, job_id")?;
        let job_ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(job_ids)
    }

    pub fn acknowledge_remote_spool_cleanup(&self, job_id: &str) -> Result<bool, JobLedgerError> {
        let connection = self.lock()?;
        Ok(connection.execute(
            "DELETE FROM remote_spool_cleanup WHERE job_id = ?1",
            [job_id],
        )? > 0)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, JobLedgerError> {
        self.connection
            .lock()
            .map_err(|_| JobLedgerError::LockPoisoned)
    }
}

fn prune_terminal_history(
    connection: &Connection,
    protected_job_id: Option<&str>,
) -> Result<usize, JobLedgerError> {
    let terminal = "status IN ('complete', 'partial', 'cancelled') AND NOT (cancellation_requested = 1 AND EXISTS (SELECT 1 FROM prepared_remote_jobs AS pending_cancel WHERE pending_cancel.job_id = recording_jobs.job_id AND pending_cancel.server_job_id IS NOT NULL AND pending_cancel.server_cancellation_acknowledged_at_ms IS NULL))";
    let deleted = if let Some(protected_job_id) = protected_job_id {
        let candidates = format!(
            "{terminal} AND job_id <> ?1 AND job_id NOT IN (SELECT job_id FROM recording_jobs WHERE {terminal} AND job_id <> ?1 ORDER BY updated_at_ms DESC, job_id DESC LIMIT ?2)"
        );
        let parameters = params![protected_job_id, (MAX_TERMINAL_JOB_HISTORY - 1) as i64];
        connection.execute(
            &format!(
                "INSERT OR IGNORE INTO remote_spool_cleanup (job_id, queued_at_ms) SELECT job_id, updated_at_ms FROM recording_jobs WHERE route = 'server_batch' AND {candidates}"
            ),
            parameters,
        )?;
        connection.execute(
            &format!("DELETE FROM recording_jobs WHERE {candidates}"),
            params![protected_job_id, (MAX_TERMINAL_JOB_HISTORY - 1) as i64],
        )?
    } else {
        let candidates = format!(
            "{terminal} AND job_id NOT IN (SELECT job_id FROM recording_jobs WHERE {terminal} ORDER BY updated_at_ms DESC, job_id DESC LIMIT ?1)"
        );
        connection.execute(
            &format!(
                "INSERT OR IGNORE INTO remote_spool_cleanup (job_id, queued_at_ms) SELECT job_id, updated_at_ms FROM recording_jobs WHERE route = 'server_batch' AND {candidates}"
            ),
            [MAX_TERMINAL_JOB_HISTORY as i64],
        )?;
        connection.execute(
            &format!("DELETE FROM recording_jobs WHERE {candidates}"),
            [MAX_TERMINAL_JOB_HISTORY as i64],
        )?
    };
    Ok(deleted)
}

struct ValidatedJob {
    job_id: String,
    session_mode: &'static str,
    session_origin: &'static str,
    source_path: Option<String>,
    source_ownership: &'static str,
    output_path: Option<String>,
    display_name: String,
    status: &'static str,
    route: Option<&'static str>,
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

struct ValidatedChunk {
    owner_namespace: String,
    session_id: String,
    track_id: String,
    sequence_start: i64,
    sequence_end: i64,
    content_sha256: String,
    content_byte_length: i64,
    artifact_path: String,
    upload_offset: i64,
    acknowledged_object_id: Option<String>,
    acknowledged_at_ms: Option<i64>,
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

struct ValidatedPreparedRemoteJob {
    create_request_json: String,
    capture_manifest_path: String,
    capture_manifest_sha256: String,
    chunks: Vec<ValidatedChunk>,
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

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn validate_opaque_identifier(
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

fn validate_server_base_url(value: &str) -> Result<(), JobLedgerError> {
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

fn path_text(path: &Path, field: &'static str) -> Result<String, JobLedgerError> {
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

fn optional_path_text(
    path: Option<&Path>,
    field: &'static str,
) -> Result<Option<String>, JobLedgerError> {
    path.map(|path| path_text(path, field)).transpose()
}

fn sqlite_integer(value: u64, field: &'static str) -> Result<i64, JobLedgerError> {
    i64::try_from(value).map_err(|_| JobLedgerError::OutOfRange { field, value })
}

fn optional_sqlite_integer(
    value: Option<u64>,
    field: &'static str,
) -> Result<Option<i64>, JobLedgerError> {
    value.map(|value| sqlite_integer(value, field)).transpose()
}

struct RawJob {
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

fn raw_job_from_row(row: &Row<'_>) -> rusqlite::Result<RawJob> {
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

fn query_job(connection: &Connection, job_id: &str) -> Result<Option<RawJob>, JobLedgerError> {
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

struct RawChunk {
    job_id: String,
    owner_namespace: String,
    session_id: String,
    track_id: String,
    sequence_start: i64,
    sequence_end: i64,
    content_sha256: String,
    content_byte_length: i64,
    artifact_path: String,
    upload_offset: i64,
    acknowledged_object_id: Option<String>,
    acknowledged_at_ms: Option<i64>,
}

struct RawPreparedRemoteJob {
    job_id: String,
    create_request_json: String,
    capture_manifest_path: String,
    capture_manifest_sha256: String,
    server_job_id: Option<String>,
    server_base_url: Option<String>,
    server_cancellation_acknowledged_at_ms: Option<i64>,
}

fn raw_prepared_remote_job_from_row(row: &Row<'_>) -> rusqlite::Result<RawPreparedRemoteJob> {
    Ok(RawPreparedRemoteJob {
        job_id: row.get(0)?,
        create_request_json: row.get(1)?,
        capture_manifest_path: row.get(2)?,
        capture_manifest_sha256: row.get(3)?,
        server_job_id: row.get(4)?,
        server_base_url: row.get(5)?,
        server_cancellation_acknowledged_at_ms: row.get(6)?,
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

fn stored_unsigned(value: i64, field: &'static str) -> Result<u64, JobLedgerError> {
    u64::try_from(value).map_err(|_| JobLedgerError::CorruptValue {
        field,
        value: value.to_string(),
    })
}

fn stored_optional_unsigned(
    value: Option<i64>,
    field: &'static str,
) -> Result<Option<u64>, JobLedgerError> {
    value.map(|value| stored_unsigned(value, field)).transpose()
}

fn stored_bool(value: i64, field: &'static str) -> Result<bool, JobLedgerError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(JobLedgerError::CorruptValue {
            field,
            value: value.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::model::{transition_policy, TransitionPolicy};
    use rusqlite::types::ValueRef;
    use std::{
        fs,
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc, Barrier,
        },
        thread,
    };

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn persisted_unknown_enum_is_reported_as_corruption() {
        let ledger = JobLedger::open_in_memory().unwrap();
        ledger.insert_job(&imported_job("bad-enum")).unwrap();
        {
            let connection = ledger.connection.lock().unwrap();
            connection
                .execute_batch("PRAGMA ignore_check_constraints = ON;")
                .unwrap();
            connection.execute(
                "UPDATE recording_jobs SET status = 'invented_ui_state' WHERE job_id = 'bad-enum'",
                [],
            ).unwrap();
        }
        assert!(matches!(
            ledger.get_job("bad-enum"),
            Err(JobLedgerError::CorruptValue {
                field: "status",
                ..
            })
        ));
    }

    #[test]
    fn durable_remote_origins_use_the_same_numeric_loopback_contract() {
        assert!(validate_server_base_url("http://127.0.0.1:18765").is_ok());
        assert!(validate_server_base_url("http://[::1]:18765").is_ok());
        assert!(validate_server_base_url("http://localhost:18765").is_err());
        assert!(validate_server_base_url("http://127.0.0.1:18765/alternate").is_err());
    }

    #[test]
    fn restart_recovers_nonterminal_jobs_and_chunks() {
        let dir = temp_dir("restart");
        let path = dir.join("jobs.sqlite3");
        let source = dir.join("interview.wav");
        fs::write(&source, b"RIFF-restart-fixture").unwrap();
        let mut job = imported_job_at("restart-job", source.clone());
        job.status = RecordingJobStatus::QueuedServer;
        job.route = Some(RecordingRoute::ServerBatch);
        let chunk = chunk_at(dir.join("chunk-0.flac"));
        {
            let ledger = JobLedger::open(&path).unwrap();
            ledger.insert_job_with_chunks(&job, &[chunk]).unwrap();
        }

        let ledger = JobLedger::open(&path).unwrap();
        let recovered = ledger.list_recoverable_jobs().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].job_id, "restart-job");
        assert_eq!(recovered[0].source_path.as_deref(), Some(source.as_path()));
        assert_eq!(ledger.list_chunks("restart-job").unwrap().len(), 1);
        drop(ledger);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prepared_remote_job_is_attached_atomically_and_survives_restart() {
        let dir = temp_dir("prepared-remote-restart");
        let database_path = dir.join("jobs.sqlite3");
        let source_path = dir.join("interview.wav");
        let manifest_path = dir.join("spool/job-remote/capture-manifest.json");
        let chunk_path = dir.join("spool/job-remote/track-1-0-9.pcm");
        fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
        fs::write(&source_path, b"RIFF-restart-fixture").unwrap();
        fs::write(&manifest_path, b"{}").unwrap();
        fs::write(&chunk_path, b"prepared audio bytes").unwrap();
        let create_request_json = r#"{"displayName":"interview.wav","route":"server_batch"}"#;

        {
            let ledger = JobLedger::open(&database_path).unwrap();
            let mut job = imported_job_at("job-remote", source_path.clone());
            job.status = RecordingJobStatus::QueuedServer;
            job.route = Some(RecordingRoute::ServerBatch);
            ledger.insert_job(&job).unwrap();
            ledger
                .transition("job-remote", RecordingJobStatus::Preprocessing, 101)
                .unwrap();
            let prepared = NewPreparedRemoteJob {
                create_request_json: create_request_json.into(),
                capture_manifest_path: manifest_path.clone(),
                capture_manifest_sha256:
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
                chunks: vec![chunk_at(chunk_path.clone())],
            };

            let attached = ledger
                .attach_prepared_remote_job("job-remote", &prepared, 102)
                .unwrap();

            assert_eq!(attached.status, RecordingJobStatus::Uploading);
            assert!(ledger
                .attach_prepared_remote_job("job-remote", &prepared, 103)
                .is_err());
            assert_eq!(ledger.list_chunks("job-remote").unwrap().len(), 1);
        }

        let ledger = JobLedger::open(&database_path).unwrap();
        let recovered = ledger
            .get_prepared_remote_job("job-remote")
            .unwrap()
            .unwrap();
        assert_eq!(recovered.create_request_json, create_request_json);
        assert_eq!(recovered.capture_manifest_path, manifest_path);
        assert_eq!(recovered.server_job_id, None);
        assert_eq!(
            ledger.get_job("job-remote").unwrap().unwrap().status,
            RecordingJobStatus::Uploading
        );
        assert_eq!(
            ledger.list_chunks("job-remote").unwrap()[0].artifact_path,
            chunk_path
        );
        drop(ledger);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn remote_create_chunk_ack_and_commit_are_idempotent_and_restart_safe() {
        let dir = temp_dir("remote-progress-restart");
        let database_path = dir.join("jobs.sqlite3");
        let source_path = dir.join("interview.wav");
        let manifest_path = dir.join("spool/job-progress/capture-manifest.json");
        let chunk_path = dir.join("spool/job-progress/track-1-0-9.pcm");
        fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
        fs::write(&source_path, b"RIFF-restart-fixture").unwrap();
        fs::write(&manifest_path, b"{}").unwrap();
        fs::write(&chunk_path, b"prepared audio bytes").unwrap();

        {
            let ledger = JobLedger::open(&database_path).unwrap();
            let mut job = imported_job_at("job-progress", source_path);
            job.status = RecordingJobStatus::QueuedServer;
            job.route = Some(RecordingRoute::ServerBatch);
            ledger.insert_job(&job).unwrap();
            ledger
                .transition("job-progress", RecordingJobStatus::Preprocessing, 101)
                .unwrap();
            ledger
                .attach_prepared_remote_job(
                    "job-progress",
                    &NewPreparedRemoteJob {
                        create_request_json:
                            r#"{"displayName":"interview.wav","route":"server_batch"}"#.into(),
                        capture_manifest_path: manifest_path,
                        capture_manifest_sha256:
                            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                                .into(),
                        chunks: vec![chunk_at(chunk_path)],
                    },
                    102,
                )
                .unwrap();

            ledger
                .record_server_job_id(
                    "job-progress",
                    "job-server-1",
                    "http://127.0.0.1:18765",
                    103,
                )
                .unwrap();
            ledger
                .record_server_job_id(
                    "job-progress",
                    "job-server-1",
                    "http://127.0.0.1:18765",
                    104,
                )
                .unwrap();
            assert!(ledger
                .record_server_job_id(
                    "job-progress",
                    "job-server-conflict",
                    "http://127.0.0.1:18765",
                    105,
                )
                .is_err());
            assert!(ledger
                .record_server_job_id(
                    "job-progress",
                    "job-server-1",
                    "http://127.0.0.1:18766",
                    105,
                )
                .is_err());
            assert!(ledger
                .mark_remote_job_committed("job-progress", 106)
                .is_err());

            ledger
                .acknowledge_remote_chunk(
                    "job-progress",
                    "microphone",
                    0,
                    9,
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    107,
                )
                .unwrap();
            ledger
                .acknowledge_remote_chunk(
                    "job-progress",
                    "microphone",
                    0,
                    9,
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    108,
                )
                .unwrap();
            ledger
                .mark_remote_job_committed("job-progress", 109)
                .unwrap();
        }

        let ledger = JobLedger::open(&database_path).unwrap();
        assert_eq!(
            ledger.get_job("job-progress").unwrap().unwrap().status,
            RecordingJobStatus::ServerProcessing
        );
        assert_eq!(
            ledger
                .get_prepared_remote_job("job-progress")
                .unwrap()
                .unwrap()
                .server_job_id
                .as_deref(),
            Some("job-server-1")
        );
        let chunks = ledger.list_chunks("job-progress").unwrap();
        assert_eq!(chunks[0].content_byte_length, 20);
        assert_eq!(chunks[0].upload_offset, chunks[0].content_byte_length);
        assert_eq!(chunks[0].acknowledged_at_ms, Some(107));
        let cancelled = ledger.request_cancellation("job-progress", 110).unwrap();
        assert_eq!(cancelled.status, RecordingJobStatus::Cancelled);
        let pending = ledger.list_pending_remote_cancellations().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].server_job_id.as_deref(), Some("job-server-1"));
        assert_eq!(
            pending[0].server_base_url.as_deref(),
            Some("http://127.0.0.1:18765")
        );
        ledger
            .acknowledge_server_cancellation("job-progress", "job-server-1", 111)
            .unwrap();
        assert!(ledger
            .list_pending_remote_cancellations()
            .unwrap()
            .is_empty());
        drop(ledger);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_remote_retry_detaches_the_old_binding_into_the_cancellation_outbox() {
        let dir = temp_dir("remote-retry-reset");
        let source_path = dir.join("source.wav");
        let manifest_path = dir.join("spool/job-retry/capture-manifest.json");
        let chunk_path = dir.join("spool/job-retry/track-1-0-9.pcm");
        fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
        fs::write(&source_path, b"RIFF-retry-fixture").unwrap();
        fs::write(&manifest_path, b"{}").unwrap();
        fs::write(&chunk_path, b"prepared audio bytes").unwrap();
        let ledger = JobLedger::open_in_memory().unwrap();
        let mut job = imported_job_at("job-retry", source_path);
        job.status = RecordingJobStatus::QueuedServer;
        job.route = Some(RecordingRoute::ServerBatch);
        ledger.insert_job(&job).unwrap();
        ledger
            .transition("job-retry", RecordingJobStatus::Preprocessing, 201)
            .unwrap();
        ledger
            .attach_prepared_remote_job(
                "job-retry",
                &NewPreparedRemoteJob {
                    create_request_json:
                        r#"{"displayName":"interview.wav","route":"server_batch"}"#.into(),
                    capture_manifest_path: manifest_path,
                    capture_manifest_sha256:
                        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
                    chunks: vec![chunk_at(chunk_path)],
                },
                202,
            )
            .unwrap();
        ledger
            .record_server_job_id(
                "job-retry",
                "job-server-retry",
                "http://127.0.0.1:18765",
                203,
            )
            .unwrap();
        let failed = ledger
            .record_remote_error(
                "job-retry",
                "SERVER_CONTRACT_ERROR",
                "The private server returned incompatible job state.",
                None,
                204,
            )
            .unwrap();
        assert_eq!(failed.status, RecordingJobStatus::Failed);

        let retried = ledger
            .retry_to_queued_server("job-retry", 205, Some(604_800_205))
            .unwrap();
        assert_eq!(retried.status, RecordingJobStatus::QueuedServer);
        assert_eq!(retried.error_code, None);
        assert_eq!(retried.capture_manifest_sha256, None);
        assert!(ledger
            .get_prepared_remote_job("job-retry")
            .unwrap()
            .is_none());
        assert!(ledger.list_chunks("job-retry").unwrap().is_empty());
        let cancellations = ledger.list_detached_remote_cancellations().unwrap();
        assert_eq!(cancellations.len(), 1);
        assert_eq!(cancellations[0].server_job_id, "job-server-retry");
        assert_eq!(cancellations[0].server_base_url, "http://127.0.0.1:18765");
        ledger
            .acknowledge_detached_remote_cancellation("http://127.0.0.1:18765", "job-server-retry")
            .unwrap();
        assert!(ledger
            .list_detached_remote_cancellations()
            .unwrap()
            .is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_callers_can_read_the_mutex_owned_connection() {
        let ledger = Arc::new(JobLedger::open_in_memory().unwrap());
        ledger.insert_job(&imported_job("concurrent-job")).unwrap();
        let barrier = Arc::new(Barrier::new(9));
        let readers: Vec<_> = (0..8)
            .map(|_| {
                let ledger = Arc::clone(&ledger);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..50 {
                        assert_eq!(
                            ledger.get_job("concurrent-job").unwrap().unwrap().job_id,
                            "concurrent-job"
                        );
                    }
                })
            })
            .collect();
        barrier.wait();
        for reader in readers {
            reader.join().unwrap();
        }
    }

    #[test]
    fn job_and_chunk_insert_rolls_back_as_one_transaction() {
        let ledger = JobLedger::open_in_memory().unwrap();
        let chunk = chunk_at(std::env::temp_dir().join("duplicate-chunk.flac"));
        let error = ledger
            .insert_job_with_chunks(&imported_job("rollback-job"), &[chunk.clone(), chunk])
            .unwrap_err();
        assert!(matches!(error, JobLedgerError::Sqlite(_)));
        assert!(ledger.get_job("rollback-job").unwrap().is_none());
    }

    #[test]
    fn multi_job_insert_rolls_back_every_row_when_one_insert_fails() {
        let ledger = JobLedger::open_in_memory().unwrap();
        let first = imported_job("duplicate-job");
        let second = imported_job("duplicate-job");

        assert!(ledger.insert_jobs(&[first, second]).is_err());
        assert!(ledger.list_jobs().unwrap().is_empty());
    }

    #[test]
    fn every_unsigned_sql_value_is_range_checked_without_partial_writes() {
        type JobMutation = fn(&mut NewRecordingJob);
        type ChunkMutation = fn(&mut NewJobChunk);

        let ledger = JobLedger::open_in_memory().unwrap();
        let job_cases: [(&str, JobMutation); 5] = [
            ("attempt", |job: &mut NewRecordingJob| {
                job.attempt_count = u64::MAX
            }),
            ("next", |job: &mut NewRecordingJob| {
                job.next_attempt_at_ms = Some(u64::MAX)
            }),
            ("created", |job: &mut NewRecordingJob| {
                job.created_at_ms = u64::MAX
            }),
            ("updated", |job: &mut NewRecordingJob| {
                job.updated_at_ms = u64::MAX
            }),
            ("expires", |job: &mut NewRecordingJob| {
                job.expires_at_ms = Some(u64::MAX)
            }),
        ];
        for (id, mutate) in job_cases {
            let mut job = imported_job(id);
            mutate(&mut job);
            assert!(matches!(
                ledger.insert_job(&job),
                Err(JobLedgerError::OutOfRange { .. })
            ));
            assert!(ledger.get_job(id).unwrap().is_none());
        }

        let chunk_cases: [(&str, ChunkMutation); 5] = [
            ("seq-start", |chunk: &mut NewJobChunk| {
                chunk.sequence_start = u64::MAX
            }),
            ("seq-end", |chunk: &mut NewJobChunk| {
                chunk.sequence_end = u64::MAX
            }),
            ("offset", |chunk: &mut NewJobChunk| {
                chunk.upload_offset = u64::MAX
            }),
            ("byte-length", |chunk: &mut NewJobChunk| {
                chunk.content_byte_length = u64::MAX
            }),
            ("ack-at", |chunk: &mut NewJobChunk| {
                chunk.acknowledged_at_ms = Some(u64::MAX)
            }),
        ];
        for (id, mutate) in chunk_cases {
            let mut chunk = chunk_at(std::env::temp_dir().join(format!("{id}.flac")));
            mutate(&mut chunk);
            assert!(matches!(
                ledger.insert_job_with_chunks(&imported_job(id), &[chunk]),
                Err(JobLedgerError::OutOfRange { .. })
            ));
            assert!(ledger.get_job(id).unwrap().is_none());
        }
    }

    #[test]
    fn retry_is_transactional_and_never_skips_preflight() {
        let ledger = JobLedger::open_in_memory().unwrap();
        let mut failed = imported_job("retry-job");
        failed.status = RecordingJobStatus::Failed;
        failed.attempt_count = 3;
        ledger.insert_job(&failed).unwrap();

        assert_eq!(
            transition_policy(RecordingJobStatus::Failed, RecordingJobStatus::Uploading),
            TransitionPolicy::Forbidden
        );
        assert!(matches!(
            ledger.transition("retry-job", RecordingJobStatus::Uploading, 200),
            Err(JobLedgerError::InvalidTransition { .. })
        ));
        let retried = ledger.retry("retry-job", 201).unwrap();
        assert_eq!(retried.status, RecordingJobStatus::Preflighting);
        assert_eq!(retried.attempt_count, 4);
        assert_eq!(retried.updated_at_ms, 201);
    }

    #[test]
    fn retry_rejects_max_counter_before_waiting_for_a_writer_transaction() {
        let dir = temp_dir("retry-max-before-transaction");
        let path = dir.join("jobs.sqlite3");
        let ledger = Arc::new(JobLedger::open(&path).unwrap());
        let mut failed = imported_job("retry-max");
        failed.status = RecordingJobStatus::Failed;
        failed.attempt_count = i64::MAX as u64;
        ledger.insert_job(&failed).unwrap();

        let writer = rusqlite::Connection::open(&path).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let retrying_ledger = Arc::clone(&ledger);
        let retry = thread::spawn(move || {
            result_tx
                .send(retrying_ledger.retry("retry-max", 202))
                .unwrap();
        });

        let early_result = result_rx.recv_timeout(std::time::Duration::from_millis(200));
        let was_early = early_result.is_ok();
        writer.execute_batch("ROLLBACK").unwrap();
        let result = match early_result {
            Ok(result) => result,
            Err(_) => result_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
        };
        retry.join().unwrap();
        assert!(
            was_early,
            "retry opened a writer transaction before rejecting i64::MAX"
        );
        assert!(matches!(
            result,
            Err(JobLedgerError::OutOfRange {
                field: "attempt_count",
                value,
            }) if value == i64::MAX as u64 + 1
        ));

        drop(writer);
        drop(ledger);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_retry_connections_increment_once_without_a_stale_overwrite() {
        let dir = temp_dir("concurrent-retry");
        let path = dir.join("jobs.sqlite3");
        let first = JobLedger::open(&path).unwrap();
        let mut failed = imported_job("concurrent-retry");
        failed.status = RecordingJobStatus::Failed;
        failed.attempt_count = 7;
        first.insert_job(&failed).unwrap();
        let second = JobLedger::open(&path).unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let retries: Vec<_> = [first, second]
            .into_iter()
            .map(|ledger| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    ledger.retry("concurrent-retry", 203)
                })
            })
            .collect();

        barrier.wait();
        let results: Vec<_> = retries
            .into_iter()
            .map(|retry| retry.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(JobLedgerError::InvalidTransition { .. })))
                .count(),
            1
        );

        let observer = JobLedger::open(&path).unwrap();
        let record = observer.get_job("concurrent-retry").unwrap().unwrap();
        assert_eq!(record.status, RecordingJobStatus::Preflighting);
        assert_eq!(record.attempt_count, 8);
        drop(observer);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancellation_updates_metadata_but_never_deletes_an_external_source() {
        let dir = temp_dir("external-cancel");
        let source = dir.join("user-owned.wav");
        fs::write(&source, b"RIFF-user-owned").unwrap();
        let ledger = JobLedger::open_in_memory().unwrap();
        ledger
            .insert_job(&imported_job_at("cancel-job", source.clone()))
            .unwrap();

        let cancelled = ledger.request_cancellation("cancel-job", 300).unwrap();
        assert_eq!(cancelled.status, RecordingJobStatus::Cancelled);
        assert!(cancelled.cancellation_requested);
        assert!(source.exists());
        drop(ledger);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_dismissal_uses_its_central_policy_and_preserves_failure_provenance() {
        let dir = temp_dir("external-dismiss");
        let source = dir.join("user-owned-failed.wav");
        fs::write(&source, b"RIFF-user-owned-failed").unwrap();
        let ledger = JobLedger::open_in_memory().unwrap();
        let mut failed = imported_job_at("dismiss-job", source.clone());
        failed.status = RecordingJobStatus::Failed;
        failed.error_code = Some("PLAYBACK_AUTHORITY_FAILED".into());
        failed.error_message = Some("playback authority could not be established".into());
        ledger.insert_job(&failed).unwrap();

        assert!(matches!(
            ledger.transition("dismiss-job", RecordingJobStatus::Cancelled, 300),
            Err(JobLedgerError::DismissRequired)
        ));
        assert!(matches!(
            ledger.request_cancellation("dismiss-job", 301),
            Err(JobLedgerError::InvalidTransition { .. })
        ));
        let dismissed = ledger.dismiss_failed("dismiss-job", 302).unwrap();

        assert_eq!(dismissed.status, RecordingJobStatus::Cancelled);
        assert!(dismissed.cancellation_requested);
        assert_eq!(dismissed.source_path.as_deref(), Some(source.as_path()));
        assert_eq!(
            dismissed.error_code.as_deref(),
            Some("PLAYBACK_AUTHORITY_FAILED")
        );
        assert_eq!(
            dismissed.error_message.as_deref(),
            Some("playback authority could not be established")
        );
        assert!(source.exists());
        drop(ledger);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn terminal_history_is_bounded_without_pruning_recoverable_or_current_jobs() {
        let ledger = JobLedger::open_in_memory().unwrap();
        ledger.insert_job(&imported_job("active-survivor")).unwrap();
        let mut failed = imported_job("failed-survivor");
        failed.status = RecordingJobStatus::Failed;
        ledger.insert_job(&failed).unwrap();

        for index in 0..MAX_TERMINAL_JOB_HISTORY {
            let id = format!("terminal-{index:04}");
            let mut job = imported_job(&id);
            if index == 0 {
                job.route = Some(RecordingRoute::ServerBatch);
            }
            if index == 0 {
                ledger
                    .insert_job_with_chunks(
                        &job,
                        &[chunk_at(std::env::temp_dir().join("old.flac"))],
                    )
                    .unwrap();
            } else {
                ledger.insert_job(&job).unwrap();
            }
            ledger
                .request_cancellation(&id, 1_000 + index as u64)
                .unwrap();
        }

        ledger
            .insert_job(&imported_job("protected-current"))
            .unwrap();
        ledger.request_cancellation("protected-current", 1).unwrap();

        let connection = ledger.connection.lock().unwrap();
        let terminal_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM recording_jobs WHERE status IN ('complete', 'partial', 'cancelled')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let pruned_chunk_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM job_chunks WHERE job_id = 'terminal-0000'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);

        assert_eq!(terminal_count, MAX_TERMINAL_JOB_HISTORY as i64);
        assert!(ledger.get_job("terminal-0000").unwrap().is_none());
        assert!(ledger.get_job("protected-current").unwrap().is_some());
        assert!(ledger.get_job("active-survivor").unwrap().is_some());
        assert_eq!(
            ledger.get_job("failed-survivor").unwrap().unwrap().status,
            RecordingJobStatus::Failed
        );
        assert_eq!(pruned_chunk_count, 0);
        assert_eq!(
            ledger.list_pending_remote_spool_cleanup().unwrap(),
            ["terminal-0000"]
        );
        assert!(ledger
            .acknowledge_remote_spool_cleanup("terminal-0000")
            .unwrap());
        assert!(ledger
            .list_pending_remote_spool_cleanup()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn reopening_legacy_database_prunes_preexisting_terminal_overflow_and_chunks() {
        let dir = temp_dir("terminal-reopen-prune");
        let path = dir.join("jobs.sqlite3");
        {
            let ledger = JobLedger::open(&path).unwrap();
            ledger.insert_job(&imported_job("active-survivor")).unwrap();
            let mut failed = imported_job("failed-survivor");
            failed.status = RecordingJobStatus::Failed;
            ledger.insert_job(&failed).unwrap();

            ledger
                .insert_job_with_chunks(
                    &imported_job("terminal-0000"),
                    &[chunk_at(dir.join("old-terminal.flac"))],
                )
                .unwrap();
            let overflow = (1..=MAX_TERMINAL_JOB_HISTORY)
                .map(|index| imported_job(&format!("terminal-{index:04}")))
                .collect::<Vec<_>>();
            ledger.insert_jobs(&overflow).unwrap();

            let mut connection = ledger.connection.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            transaction
                .execute(
                    "UPDATE recording_jobs SET route = 'server_batch' WHERE job_id = 'terminal-0000'",
                    [],
                )
                .unwrap();
            for index in 0..=MAX_TERMINAL_JOB_HISTORY {
                transaction
                    .execute(
                        "UPDATE recording_jobs SET status = 'cancelled', updated_at_ms = ?1 WHERE job_id = ?2",
                        params![1_000 + index as i64, format!("terminal-{index:04}")],
                    )
                    .unwrap();
            }
            transaction.commit().unwrap();
        }

        let reopened = JobLedger::open(&path).unwrap();
        let connection = reopened.connection.lock().unwrap();
        let terminal_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM recording_jobs WHERE status IN ('complete', 'partial', 'cancelled')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let pruned_chunk_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM job_chunks WHERE job_id = 'terminal-0000'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);

        assert_eq!(terminal_count, MAX_TERMINAL_JOB_HISTORY as i64);
        assert!(reopened.get_job("terminal-0000").unwrap().is_none());
        assert!(reopened.get_job("terminal-0500").unwrap().is_some());
        assert!(reopened.get_job("active-survivor").unwrap().is_some());
        assert_eq!(
            reopened.get_job("failed-survivor").unwrap().unwrap().status,
            RecordingJobStatus::Failed
        );
        assert_eq!(pruned_chunk_count, 0);
        assert_eq!(
            reopened.list_pending_remote_spool_cleanup().unwrap(),
            ["terminal-0000"]
        );
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restart_database_has_exact_metadata_surface_and_no_payload_content() {
        let dir = temp_dir("content-audit");
        let path = dir.join("jobs.sqlite3");
        let source = dir.join("source.wav");
        let output = dir.join("output.txt");
        let artifact = dir.join("chunk.flac");
        let wav_bytes = b"RIFF\x00\x01YAP_PRIVATE_WAV_BYTES";
        let transcript = "YAP_PRIVATE_TRANSCRIPT_SENTENCE";
        fs::write(&source, wav_bytes).unwrap();
        fs::write(&output, transcript).unwrap();
        fs::write(&artifact, b"encoded audio bytes").unwrap();
        let mut job = imported_job_at("audit-job", source);
        job.output_path = Some(output);
        {
            let ledger = JobLedger::open(&path).unwrap();
            ledger
                .insert_job_with_chunks(&job, &[chunk_at(artifact)])
                .unwrap();
        }

        let connection = rusqlite::Connection::open(&path).unwrap();
        let table_names: Vec<String> = {
            let mut statement = connection.prepare(
                "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
            ).unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(
            table_names,
            [
                "detached_remote_cancellations",
                "job_chunks",
                "prepared_remote_jobs",
                "recording_jobs",
                "remote_spool_cleanup",
            ]
        );
        let expected_columns = [
            (
                "detached_remote_cancellations",
                &[
                    ("server_base_url", "TEXT"),
                    ("server_job_id", "TEXT"),
                    ("create_request_json", "TEXT"),
                    ("queued_at_ms", "INTEGER"),
                ][..],
            ),
            (
                "job_chunks",
                &[
                    ("job_id", "TEXT"),
                    ("owner_namespace", "TEXT"),
                    ("session_id", "TEXT"),
                    ("track_id", "TEXT"),
                    ("sequence_start", "INTEGER"),
                    ("sequence_end", "INTEGER"),
                    ("content_sha256", "TEXT"),
                    ("artifact_path", "TEXT"),
                    ("upload_offset", "INTEGER"),
                    ("acknowledged_object_id", "TEXT"),
                    ("acknowledged_at_ms", "INTEGER"),
                    ("content_byte_length", "INTEGER"),
                ][..],
            ),
            (
                "prepared_remote_jobs",
                &[
                    ("job_id", "TEXT"),
                    ("create_request_json", "TEXT"),
                    ("capture_manifest_path", "TEXT"),
                    ("capture_manifest_sha256", "TEXT"),
                    ("server_job_id", "TEXT"),
                    ("server_base_url", "TEXT"),
                    ("server_cancellation_acknowledged_at_ms", "INTEGER"),
                ][..],
            ),
            (
                "recording_jobs",
                &[
                    ("job_id", "TEXT"),
                    ("session_mode", "TEXT"),
                    ("session_origin", "TEXT"),
                    ("source_path", "TEXT"),
                    ("source_ownership", "TEXT"),
                    ("output_path", "TEXT"),
                    ("display_name", "TEXT"),
                    ("status", "TEXT"),
                    ("route", "TEXT"),
                    ("attempt_count", "INTEGER"),
                    ("next_attempt_at_ms", "INTEGER"),
                    ("cancellation_requested", "INTEGER"),
                    ("capture_commit_path", "TEXT"),
                    ("capture_manifest_sha256", "TEXT"),
                    ("error_code", "TEXT"),
                    ("error_message", "TEXT"),
                    ("created_at_ms", "INTEGER"),
                    ("updated_at_ms", "INTEGER"),
                    ("expires_at_ms", "INTEGER"),
                ][..],
            ),
            (
                "remote_spool_cleanup",
                &[("job_id", "TEXT"), ("queued_at_ms", "INTEGER")][..],
            ),
        ];
        for (table, expected) in expected_columns {
            let actual: Vec<(String, String)> = {
                let mut statement = connection
                    .prepare(&format!("PRAGMA table_info(\"{table}\")"))
                    .unwrap();
                statement
                    .query_map([], |row| Ok((row.get(1)?, row.get(2)?)))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap()
            };
            assert_eq!(
                actual,
                expected
                    .iter()
                    .map(|(name, kind)| ((*name).into(), (*kind).into()))
                    .collect::<Vec<(String, String)>>(),
                "{table} added an unapproved payload, credential, or embedding storage surface"
            );

            let mut statement = connection
                .prepare(&format!("SELECT * FROM \"{table}\""))
                .unwrap();
            let column_count = statement.column_count();
            let mut rows = statement.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                for column in 0..column_count {
                    match row.get_ref(column).unwrap() {
                        ValueRef::Text(value) | ValueRef::Blob(value) => {
                            assert!(!value
                                .windows(wav_bytes.len())
                                .any(|window| window == wav_bytes));
                            let text = String::from_utf8_lossy(value);
                            assert!(!text.contains(transcript));
                        }
                        ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) => {}
                    }
                }
            }
        }
        drop(connection);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn relative_paths_are_rejected_before_persistence() {
        let ledger = JobLedger::open_in_memory().unwrap();
        let mut job = imported_job("relative-path");
        job.source_path = Some("relative.wav".into());
        assert!(matches!(
            ledger.insert_job(&job),
            Err(JobLedgerError::InvalidPath { .. })
        ));
        assert!(ledger.get_job("relative-path").unwrap().is_none());
    }

    fn imported_job(id: &str) -> NewRecordingJob {
        imported_job_at(id, std::env::temp_dir().join(format!("{id}.wav")))
    }

    fn imported_job_at(id: &str, source_path: std::path::PathBuf) -> NewRecordingJob {
        NewRecordingJob {
            job_id: id.into(),
            session_mode: SessionMode::Meeting,
            session_origin: SessionOrigin::ImportedFile,
            source_path: Some(source_path),
            source_ownership: SourceOwnership::External,
            output_path: None,
            display_name: format!("{id}.wav"),
            status: RecordingJobStatus::Accepted,
            route: None,
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

    fn chunk_at(artifact_path: std::path::PathBuf) -> NewJobChunk {
        NewJobChunk {
            owner_namespace: "local:test-install".into(),
            session_id: "session-1".into(),
            track_id: "microphone".into(),
            sequence_start: 0,
            sequence_end: 9,
            content_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .into(),
            content_byte_length: 20,
            artifact_path,
            upload_offset: 0,
            acknowledged_object_id: None,
            acknowledged_at_ms: None,
        }
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("yap-ledger-{label}-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
