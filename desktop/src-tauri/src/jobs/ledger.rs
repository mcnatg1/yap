//! Durable authority for recording-job metadata and state transitions.
//! Domain modules extend `JobLedger`; this facade owns connection setup and core record access.

mod lifecycle;
mod records;
mod remote_progress;
mod remote_recovery;
mod remote_state;
mod retention;
mod row_mapping;

use self::records::{path_text, ValidatedChunk, ValidatedJob};
use self::retention::prune_terminal_history;
use self::row_mapping::{query_job, raw_job_from_row, RawChunk};
use crate::jobs::{
    migrations, JobChunkRecord, JobLedgerError, NewJobChunk, NewRecordingJob, RecordingJobRecord,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

const JOB_COLUMNS: &str = "job_id, session_mode, session_origin, source_path, source_ownership, output_path, display_name, status, route, attempt_count, next_attempt_at_ms, cancellation_requested, capture_commit_path, capture_manifest_sha256, error_code, error_message, created_at_ms, updated_at_ms, expires_at_ms";
const MAX_TERMINAL_JOB_HISTORY: usize = 500;
const MAX_PREPARED_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_PREPARED_CHUNKS: usize = 4096;

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

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, JobLedgerError> {
        self.connection
            .lock()
            .map_err(|_| JobLedgerError::LockPoisoned)
    }
}

#[cfg(test)]
mod tests;
