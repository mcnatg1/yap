//! Owns legal job transitions, retry preflight, and durable cancellation intent.

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::jobs::{
    model::{transition_policy, TransitionPolicy},
    JobLedgerError, RecordingJobRecord, RecordingJobStatus,
};

use super::{
    records::{optional_sqlite_integer, sqlite_integer},
    remote_recovery::enqueue_detached_cancellation,
    retention::prune_terminal_history,
    row_mapping::query_job,
    JobLedger,
};

impl JobLedger {
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
                    enqueue_detached_cancellation(
                        &transaction,
                        &server_base_url,
                        &server_job_id,
                        &create_request_json,
                        updated_at_ms,
                    )?;
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
}
