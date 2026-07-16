//! Reconciles abandoned or origin-changed remote work through persisted cleanup authority.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::jobs::{
    DetachedRemoteCancellationRecord, JobLedgerError, PreparedRemoteJobRecord, RecordingJobRecord,
    RecordingJobStatus,
};

use super::{
    records::{sqlite_integer, validate_opaque_identifier, validate_server_base_url},
    retention::prune_terminal_history,
    row_mapping::{query_job, raw_prepared_remote_job_from_row, stored_unsigned},
    JobLedger, MAX_PREPARED_REQUEST_BYTES,
};

impl JobLedger {
    pub fn list_pending_remote_cancellations(
        &self,
    ) -> Result<Vec<PreparedRemoteJobRecord>, JobLedgerError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT prepared.job_id, prepared.create_request_json, prepared.capture_manifest_path, prepared.capture_manifest_sha256, prepared.server_job_id, prepared.server_base_url, prepared.server_cancellation_acknowledged_at_ms, prepared.create_attempt_base_url FROM prepared_remote_jobs AS prepared JOIN recording_jobs AS job ON job.job_id = prepared.job_id WHERE job.status = 'cancelled' AND job.cancellation_requested = 1 AND prepared.server_job_id IS NOT NULL AND prepared.server_base_url IS NOT NULL AND prepared.server_cancellation_acknowledged_at_ms IS NULL ORDER BY job.updated_at_ms, prepared.job_id",
        )?;
        let rows = statement.query_map([], raw_prepared_remote_job_from_row)?;
        rows.map(|row| {
            row.map_err(JobLedgerError::from)
                .and_then(TryInto::try_into)
        })
        .collect()
    }

    pub fn list_remote_create_attempts(
        &self,
    ) -> Result<Vec<PreparedRemoteJobRecord>, JobLedgerError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT prepared.job_id, prepared.create_request_json, prepared.capture_manifest_path, prepared.capture_manifest_sha256, prepared.server_job_id, prepared.server_base_url, prepared.server_cancellation_acknowledged_at_ms, prepared.create_attempt_base_url FROM prepared_remote_jobs AS prepared JOIN recording_jobs AS job ON job.job_id = prepared.job_id WHERE job.status = 'uploading' AND prepared.server_job_id IS NULL AND prepared.server_base_url IS NULL AND prepared.create_attempt_base_url IS NOT NULL ORDER BY job.updated_at_ms, prepared.job_id",
        )?;
        let rows = statement.query_map([], raw_prepared_remote_job_from_row)?;
        rows.map(|row| {
            row.map_err(JobLedgerError::from)
                .and_then(TryInto::try_into)
        })
        .collect()
    }

    pub fn list_cancelled_remote_create_attempts(
        &self,
    ) -> Result<Vec<PreparedRemoteJobRecord>, JobLedgerError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT prepared.job_id, prepared.create_request_json, prepared.capture_manifest_path, prepared.capture_manifest_sha256, prepared.server_job_id, prepared.server_base_url, prepared.server_cancellation_acknowledged_at_ms, prepared.create_attempt_base_url FROM prepared_remote_jobs AS prepared JOIN recording_jobs AS job ON job.job_id = prepared.job_id WHERE job.status = 'cancelled' AND job.cancellation_requested = 1 AND prepared.server_job_id IS NULL AND prepared.server_base_url IS NULL AND prepared.create_attempt_base_url IS NOT NULL ORDER BY job.updated_at_ms, prepared.job_id",
        )?;
        let rows = statement.query_map([], raw_prepared_remote_job_from_row)?;
        rows.map(|row| {
            row.map_err(JobLedgerError::from)
                .and_then(TryInto::try_into)
        })
        .collect()
    }

    pub fn detach_changed_remote_binding(
        &self,
        configured_origin: Option<&str>,
        updated_at_ms: u64,
        cleanup_owned_spool: impl FnOnce(&str) -> Result<(), String>,
    ) -> Result<Option<String>, JobLedgerError> {
        if let Some(origin) = configured_origin {
            validate_server_base_url(origin)?;
        }
        let updated_at_ms = sqlite_integer(updated_at_ms, "updated_at_ms")?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let candidate: Option<(String, String, String, String)> = transaction
            .query_row(
                "SELECT prepared.job_id, prepared.server_base_url, prepared.server_job_id, prepared.create_request_json FROM prepared_remote_jobs AS prepared JOIN recording_jobs AS job ON job.job_id = prepared.job_id WHERE prepared.server_job_id IS NOT NULL AND prepared.server_base_url IS NOT NULL AND prepared.server_cancellation_acknowledged_at_ms IS NULL AND job.status IN ('uploading', 'server_processing', 'saving', 'failed') AND (?1 IS NULL OR prepared.server_base_url <> ?1) ORDER BY job.updated_at_ms, prepared.job_id LIMIT 1",
                [configured_origin],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((job_id, server_base_url, server_job_id, create_request_json)) = candidate else {
            transaction.commit()?;
            return Ok(None);
        };
        validate_server_base_url(&server_base_url)?;
        validate_opaque_identifier(&server_job_id, 128, "server job ID")?;
        if !(2..=MAX_PREPARED_REQUEST_BYTES).contains(&create_request_json.len()) {
            return Err(JobLedgerError::InvalidRecord(
                "changed-origin cancellation request is outside the persisted contract",
            ));
        }
        let current: RecordingJobRecord = query_job(&transaction, &job_id)?
            .ok_or_else(|| JobLedgerError::NotFound(job_id.clone()))?
            .try_into()?;
        enqueue_detached_cancellation(
            &transaction,
            &server_base_url,
            &server_job_id,
            &create_request_json,
            updated_at_ms,
        )?;
        cleanup_owned_spool(&job_id).map_err(JobLedgerError::OwnedSpoolCleanup)?;
        let next_attempt_count = if current.status == RecordingJobStatus::Failed {
            current.attempt_count
        } else {
            current
                .attempt_count
                .checked_add(1)
                .ok_or(JobLedgerError::OutOfRange {
                    field: "attempt_count",
                    value: u64::MAX,
                })?
        };
        let next_attempt_count = sqlite_integer(next_attempt_count, "attempt_count")?;
        let changed = transaction.execute(
            "UPDATE recording_jobs SET status = 'failed', attempt_count = ?1, next_attempt_at_ms = NULL, error_code = 'REMOTE_ORIGIN_CHANGED', error_message = 'The private-server origin changed before this request could finish. Retry the recording to start a new server job.', updated_at_ms = ?2 WHERE job_id = ?3",
            params![next_attempt_count, updated_at_ms, job_id],
        )?;
        if changed != 1 {
            return Err(JobLedgerError::InvalidRecord(
                "changed-origin job was not durably failed",
            ));
        }
        transaction.execute("DELETE FROM job_chunks WHERE job_id = ?1", [&job_id])?;
        transaction.execute(
            "DELETE FROM prepared_remote_jobs WHERE job_id = ?1",
            [&job_id],
        )?;
        transaction.commit()?;
        Ok(Some(job_id))
    }

    pub fn fail_abandoned_remote_create_attempt(
        &self,
        job_id: &str,
        server_base_url: &str,
        updated_at_ms: u64,
        cleanup_owned_spool: impl FnOnce() -> Result<(), String>,
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
                to: RecordingJobStatus::Failed,
            });
        }
        let attempt: (Option<String>, Option<String>, Option<String>) = transaction
            .query_row(
                "SELECT server_job_id, server_base_url, create_attempt_base_url FROM prepared_remote_jobs WHERE job_id = ?1",
                [job_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(JobLedgerError::InvalidRecord(
                "abandoned create attempt has no prepared remote state",
            ))?;
        if attempt.0.is_some()
            || attempt.1.is_some()
            || attempt.2.as_deref() != Some(server_base_url)
        {
            return Err(JobLedgerError::InvalidRecord(
                "abandoned create attempt no longer matches its cleanup origin",
            ));
        }
        cleanup_owned_spool().map_err(JobLedgerError::OwnedSpoolCleanup)?;
        let next_attempt_count =
            current
                .attempt_count
                .checked_add(1)
                .ok_or(JobLedgerError::OutOfRange {
                    field: "attempt_count",
                    value: u64::MAX,
                })?;
        let next_attempt_count = sqlite_integer(next_attempt_count, "attempt_count")?;
        let changed = transaction.execute(
            "UPDATE recording_jobs SET status = 'failed', attempt_count = ?1, next_attempt_at_ms = NULL, error_code = 'REMOTE_ORIGIN_CHANGED', error_message = 'The private-server origin changed before this request could finish. Retry the recording to start a new server job.', updated_at_ms = ?2 WHERE job_id = ?3 AND status = 'uploading'",
            params![next_attempt_count, updated_at_ms, job_id],
        )?;
        if changed != 1 {
            return Err(JobLedgerError::InvalidRecord(
                "abandoned remote create attempt was not durably failed",
            ));
        }
        transaction.execute("DELETE FROM job_chunks WHERE job_id = ?1", [job_id])?;
        transaction.execute(
            "DELETE FROM prepared_remote_jobs WHERE job_id = ?1",
            [job_id],
        )?;
        let updated = query_job(&transaction, job_id)?.expect("abandoned create job exists");
        transaction.commit()?;
        updated.try_into()
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
}

pub(super) fn enqueue_detached_cancellation(
    connection: &Connection,
    server_base_url: &str,
    server_job_id: &str,
    create_request_json: &str,
    queued_at_ms: i64,
) -> Result<(), JobLedgerError> {
    validate_server_base_url(server_base_url)?;
    validate_opaque_identifier(server_job_id, 128, "server job ID")?;
    if !(2..=MAX_PREPARED_REQUEST_BYTES).contains(&create_request_json.len()) {
        return Err(JobLedgerError::InvalidRecord(
            "detached cancellation request is outside the persisted contract",
        ));
    }
    let inserted = connection.execute(
        "INSERT OR IGNORE INTO detached_remote_cancellations (server_base_url, server_job_id, create_request_json, queued_at_ms) VALUES (?1, ?2, ?3, ?4)",
        params![
            server_base_url,
            server_job_id,
            create_request_json,
            queued_at_ms,
        ],
    )?;
    if inserted == 0 {
        let existing: String = connection.query_row(
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
    Ok(())
}
