use crate::jobs::{
    migrations,
    model::{transition_policy, TransitionPolicy},
    JobChunkRecord, JobLedgerError, NewJobChunk, NewRecordingJob, RecordingJobRecord,
    RecordingJobStatus, RecordingRoute, SessionMode, SessionOrigin, SourceOwnership,
};
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

const JOB_COLUMNS: &str = "job_id, session_mode, session_origin, source_path, source_ownership, output_path, display_name, status, route, attempt_count, next_attempt_at_ms, cancellation_requested, capture_commit_path, capture_manifest_sha256, error_code, error_message, created_at_ms, updated_at_ms, expires_at_ms";

pub struct JobLedger {
    pub(super) connection: Mutex<Connection>,
}

impl JobLedger {
    pub fn open_default() -> Result<Self, JobLedgerError> {
        Self::open(crate::paths::app_data_dir().join("jobs.sqlite3"))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, JobLedgerError> {
        Ok(Self {
            connection: Mutex::new(migrations::open_file(path.as_ref())?),
        })
    }

    #[cfg(test)]
    pub(super) fn open_in_memory() -> Result<Self, JobLedgerError> {
        Ok(Self {
            connection: Mutex::new(migrations::open_in_memory()?),
        })
    }

    pub fn insert_job(&self, job: &NewRecordingJob) -> Result<RecordingJobRecord, JobLedgerError> {
        self.insert_job_with_chunks(job, &[])
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
                "INSERT INTO job_chunks (job_id, owner_namespace, session_id, track_id, sequence_start, sequence_end, content_sha256, artifact_path, upload_offset, acknowledged_object_id, acknowledged_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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

    pub fn list_chunks(&self, job_id: &str) -> Result<Vec<JobChunkRecord>, JobLedgerError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT job_id, owner_namespace, session_id, track_id, sequence_start, sequence_end, content_sha256, artifact_path, upload_offset, acknowledged_object_id, acknowledged_at_ms FROM job_chunks WHERE job_id = ?1 ORDER BY track_id, sequence_start, sequence_end",
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
            })
        })?;
        rows.map(|row| {
            row.map_err(JobLedgerError::from)
                .and_then(TryInto::try_into)
        })
        .collect()
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
        transaction.commit()?;
        updated.try_into()
    }

    pub fn retry(
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
        if transition_policy(current.status, RecordingJobStatus::Preflighting)
            != TransitionPolicy::Retry
        {
            return Err(JobLedgerError::InvalidTransition {
                from: current.status,
                to: RecordingJobStatus::Preflighting,
            });
        }
        let attempt_count =
            current
                .attempt_count
                .checked_add(1)
                .ok_or(JobLedgerError::OutOfRange {
                    field: "attempt_count",
                    value: u64::MAX,
                })?;
        let attempt_count = sqlite_integer(attempt_count, "attempt_count")?;
        transaction.execute(
            "UPDATE recording_jobs SET status = 'preflighting', attempt_count = ?1, next_attempt_at_ms = NULL, updated_at_ms = ?2 WHERE job_id = ?3",
            params![attempt_count, updated_at_ms, job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("retried job exists");
        transaction.commit()?;
        updated.try_into()
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
        transaction.commit()?;
        updated.try_into()
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, JobLedgerError> {
        self.connection
            .lock()
            .map_err(|_| JobLedgerError::LockPoisoned)
    }
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
    artifact_path: String,
    upload_offset: i64,
    acknowledged_object_id: Option<String>,
    acknowledged_at_ms: Option<i64>,
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

        let chunk_cases: [(&str, ChunkMutation); 4] = [
            ("seq-start", |chunk: &mut NewJobChunk| {
                chunk.sequence_start = u64::MAX
            }),
            ("seq-end", |chunk: &mut NewJobChunk| {
                chunk.sequence_end = u64::MAX
            }),
            ("offset", |chunk: &mut NewJobChunk| {
                chunk.upload_offset = u64::MAX
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
    fn restart_database_contains_metadata_only_across_every_table() {
        let dir = temp_dir("content-audit");
        let path = dir.join("jobs.sqlite3");
        let source = dir.join("source.wav");
        let output = dir.join("output.txt");
        let artifact = dir.join("chunk.flac");
        let wav_bytes = b"RIFF\x00\x01YAP_PRIVATE_WAV_BYTES";
        let transcript = "YAP_PRIVATE_TRANSCRIPT_SENTENCE";
        let credentials = "Bearer YAP_PRIVATE_CREDENTIAL";
        let embedding = "[0.123456789,-0.987654321,0.314159265]";
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
        assert_eq!(table_names, ["job_chunks", "recording_jobs"]);
        for table in table_names {
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
                            assert!(!text.contains(credentials));
                            assert!(!text.contains(embedding));
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
