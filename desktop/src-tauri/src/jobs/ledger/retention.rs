//! Owns failure metadata, expiry, bounded terminal history, and durable spool cleanup intent.

use rusqlite::{params, Connection, TransactionBehavior};

use crate::jobs::{
    model::{transition_policy, TransitionPolicy},
    JobLedgerError, RecordingJobRecord, RecordingJobStatus,
};

use super::{
    records::{optional_sqlite_integer, sqlite_integer, validate_opaque_identifier},
    row_mapping::query_job,
    JobLedger, MAX_TERMINAL_JOB_HISTORY,
};

impl JobLedger {
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

    pub fn has_remote_reconciliation_work(&self) -> Result<bool, JobLedgerError> {
        let connection = self.lock()?;
        let pending: i64 = connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM remote_spool_cleanup
                UNION ALL
                SELECT 1 FROM detached_remote_cancellations
                UNION ALL
                SELECT 1
                FROM prepared_remote_jobs AS prepared
                JOIN recording_jobs AS job ON job.job_id = prepared.job_id
                WHERE
                    (job.status = 'cancelled'
                        AND job.cancellation_requested = 1
                        AND prepared.server_cancellation_acknowledged_at_ms IS NULL
                        AND (prepared.server_job_id IS NOT NULL OR prepared.create_attempt_base_url IS NOT NULL))
                    OR (job.status = 'uploading' AND prepared.create_attempt_base_url IS NOT NULL)
                    OR (job.status = 'failed'
                        AND prepared.server_job_id IS NOT NULL
                        AND prepared.server_cancellation_acknowledged_at_ms IS NULL)
            )",
            [],
            |row| row.get(0),
        )?;
        Ok(pending != 0)
    }

    pub fn acknowledge_remote_spool_cleanup(&self, job_id: &str) -> Result<bool, JobLedgerError> {
        let connection = self.lock()?;
        Ok(connection.execute(
            "DELETE FROM remote_spool_cleanup WHERE job_id = ?1",
            [job_id],
        )? > 0)
    }
}

pub(super) fn prune_terminal_history(
    connection: &Connection,
    protected_job_id: Option<&str>,
) -> Result<usize, JobLedgerError> {
    let terminal = "status IN ('complete', 'partial', 'cancelled') AND NOT (cancellation_requested = 1 AND EXISTS (SELECT 1 FROM prepared_remote_jobs AS pending_cancel WHERE pending_cancel.job_id = recording_jobs.job_id AND (pending_cancel.server_job_id IS NOT NULL OR pending_cancel.create_attempt_base_url IS NOT NULL) AND pending_cancel.server_cancellation_acknowledged_at_ms IS NULL))";
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
