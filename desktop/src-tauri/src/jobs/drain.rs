use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use tauri::{Emitter, Manager};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    audio::session::OwnerNamespace,
    jobs::{
        remote, JobChunkRecord, JobLedger, PreparedRemoteJobRecord, RecordingJobRecord,
        RecordingJobStatus,
    },
    server_connector::batch::{
        ApiError, BatchApiClient, BatchClientError, CaptureChunkReference,
        CommitRecordingJobRequest, CreateRecordingJobRequest, RecordingJob,
        TranscriptResultRevision,
    },
    server_connector::{BatchConnectionLease, ServerConnector},
};

const MAX_AUTOMATIC_REMOTE_ATTEMPTS: u64 = 6;

#[derive(Debug)]
struct DrainStepError {
    detail: String,
    automatic_retry: bool,
    code: &'static str,
    user_message: &'static str,
}

impl DrainStepError {
    fn permanent(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            automatic_retry: false,
            code: "REMOTE_STATE_INVALID",
            user_message: "The private-server job state is incompatible. Retry the recording to start a new server job.",
        }
    }

    fn transient_state(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            automatic_retry: true,
            code: "REMOTE_REQUEST_RETRYING",
            user_message:
                "The private-server request did not complete. Yap will retry automatically.",
        }
    }

    fn terminal_server(error: &ApiError) -> Self {
        Self {
            detail: format!(
                "server job failed with {} (request {}, retryable={})",
                error.code, error.request_id, error.retryable
            ),
            automatic_retry: false,
            code: "REMOTE_SERVER_FAILED",
            user_message: "The private server could not complete this recording. Retry it to start a new server job.",
        }
    }
}

impl std::fmt::Display for DrainStepError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl From<String> for DrainStepError {
    fn from(detail: String) -> Self {
        Self::permanent(detail)
    }
}

impl From<&str> for DrainStepError {
    fn from(detail: &str) -> Self {
        Self::permanent(detail)
    }
}

impl From<BatchClientError> for DrainStepError {
    fn from(error: BatchClientError) -> Self {
        if error.is_retryable() {
            Self::transient_state(error.to_string())
        } else {
            Self::permanent(error.to_string())
        }
    }
}

type DrainResult<T> = Result<T, DrainStepError>;

enum BatchCommitGuard<'a> {
    PersistedCleanup,
    #[cfg(test)]
    Unchecked,
    #[cfg(test)]
    StaleForTest,
    #[cfg(test)]
    StaleAfterForTest {
        remaining_successes: &'a std::sync::atomic::AtomicUsize,
    },
    Lease {
        connector: &'a ServerConnector,
        lease: &'a BatchConnectionLease,
    },
}

impl BatchCommitGuard<'_> {
    fn commit<T>(&self, mutation: impl FnOnce() -> DrainResult<T>) -> DrainResult<T> {
        match self {
            Self::PersistedCleanup => mutation(),
            #[cfg(test)]
            Self::Unchecked => mutation(),
            #[cfg(test)]
            Self::StaleForTest => Err(DrainStepError::transient_state("test stale lease")),
            #[cfg(test)]
            Self::StaleAfterForTest {
                remaining_successes,
            } => {
                let remaining = remaining_successes.load(std::sync::atomic::Ordering::SeqCst);
                if remaining == 0 {
                    Err(DrainStepError::transient_state("test stale lease"))
                } else {
                    remaining_successes.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    mutation()
                }
            }
            Self::Lease { connector, lease } => connector
                .with_current_batch_lease(lease, mutation)
                .map_err(DrainStepError::transient_state)?,
        }
    }

    fn ensure_current(&self) -> DrainResult<()> {
        self.commit(|| Ok(()))
    }
}

pub(crate) struct RemoteJobDrain {
    ledger: JobLedger,
    owner_namespace: OwnerNamespace,
    owned_live_directory: PathBuf,
    remote_jobs_directory: PathBuf,
}

impl RemoteJobDrain {
    pub(crate) fn open_default() -> Result<Self, String> {
        Ok(Self {
            ledger: JobLedger::open_default().map_err(|error| error.to_string())?,
            owner_namespace: crate::install_identity::load_or_create()?,
            owned_live_directory: crate::live::recordings::recordings_dir(),
            remote_jobs_directory: crate::paths::app_data_dir().join("remote-jobs"),
        })
    }

    fn has_pending_work(&self) -> Result<bool, String> {
        let active_job = self
            .ledger
            .list_recoverable_jobs()
            .map_err(|error| error.to_string())?
            .into_iter()
            .any(|job| {
                matches!(
                    job.status,
                    RecordingJobStatus::QueuedServer
                        | RecordingJobStatus::Preprocessing
                        | RecordingJobStatus::Uploading
                        | RecordingJobStatus::ServerProcessing
                        | RecordingJobStatus::Saving
                )
            });
        Ok(active_job
            || self
                .ledger
                .has_remote_reconciliation_work()
                .map_err(|error| error.to_string())?)
    }

    fn enforce_retention(&self, now_ms: u64) -> Result<bool, String> {
        let expired_pending = self
            .ledger
            .expire_pending_jobs(now_ms)
            .map_err(|error| error.to_string())?;
        let (expired_remote_job_ids, changed_remote_jobs) = self
            .ledger
            .enforce_remote_retention(now_ms)
            .map_err(|error| error.to_string())?;
        let mut cleanup_error = None;
        for job_id in expired_remote_job_ids {
            if let Err(error) = remote::reset_unattached_spool(&job_id, &self.remote_jobs_directory)
            {
                cleanup_error.get_or_insert(error);
            }
        }
        if let Some(error) = cleanup_error {
            return Err(error);
        }
        let mut pruned_spools = 0_usize;
        for job_id in self
            .ledger
            .list_pending_remote_spool_cleanup()
            .map_err(|error| error.to_string())?
        {
            remote::reset_unattached_spool(&job_id, &self.remote_jobs_directory)?;
            if self
                .ledger
                .acknowledge_remote_spool_cleanup(&job_id)
                .map_err(|error| error.to_string())?
            {
                pruned_spools = pruned_spools.saturating_add(1);
            }
        }
        Ok(expired_pending > 0 || changed_remote_jobs > 0 || pruned_spools > 0)
    }

    fn fail_preprocessing_candidate(&self, updated_at_ms: u64) {
        let candidate = self.ledger.list_recoverable_jobs().ok().and_then(|jobs| {
            jobs.into_iter()
                .find(|job| job.status == RecordingJobStatus::Preprocessing)
        });
        let Some(candidate) = candidate else {
            return;
        };
        let _ = self.ledger.record_remote_error(
            &candidate.job_id,
            "PREPROCESSING_FAILED",
            "The selected recording could not be prepared for private-server transcription.",
            None,
            updated_at_ms,
        );
    }

    fn schedule_remote_retry(
        &self,
        statuses: &[RecordingJobStatus],
        error: &DrainStepError,
        updated_at_ms: u64,
    ) {
        let candidate = self.ledger.list_recoverable_jobs().ok().and_then(|jobs| {
            jobs.into_iter().find(|job| {
                statuses.contains(&job.status)
                    && job
                        .next_attempt_at_ms
                        .is_none_or(|retry_at| retry_at <= updated_at_ms)
            })
        });
        let Some(candidate) = candidate else {
            return;
        };
        let (retry_at_ms, code, message) =
            remote_retry_plan(error, candidate.attempt_count, updated_at_ms);
        let _ = self.ledger.record_remote_error(
            &candidate.job_id,
            code,
            message,
            retry_at_ms,
            updated_at_ms,
        );
    }
}

fn remote_retry_plan(
    error: &DrainStepError,
    attempt_count: u64,
    updated_at_ms: u64,
) -> (Option<u64>, &'static str, &'static str) {
    let delay_seconds =
        [1_u64, 2, 4, 8, 15, 30][usize::try_from(attempt_count).unwrap_or(usize::MAX).min(5)];
    let retry_at_ms = (error.automatic_retry && attempt_count < MAX_AUTOMATIC_REMOTE_ATTEMPTS)
        .then(|| updated_at_ms.saturating_add(delay_seconds.saturating_mul(1_000)));
    if error.automatic_retry && retry_at_ms.is_none() {
        return (
            None,
            "REMOTE_RETRY_EXHAUSTED",
            "The private-server request did not recover after bounded retries. Retry the recording to start a new server job.",
        );
    }
    (retry_at_ms, error.code, error.user_message)
}

pub(crate) fn start(app: &tauri::AppHandle) {
    let app = app.clone();
    std::mem::drop(tauri::async_runtime::spawn(async move {
        run(app).await;
    }));
}

async fn run(app: tauri::AppHandle) {
    let mut next_retention_check_ms = 0_u64;
    let mut next_pending_error_log_ms = 0_u64;
    loop {
        let loop_now_ms = now_ms();
        if loop_now_ms >= next_retention_check_ms {
            next_retention_check_ms = loop_now_ms.saturating_add(60_000);
            match app.state::<RemoteJobDrain>().enforce_retention(loop_now_ms) {
                Ok(true) => emit_jobs_changed(&app),
                Ok(false) => {}
                Err(error) => crate::stt::log_yap(&format!(
                    "owned remote recording retention remains pending: {error}"
                )),
            }
        }
        let has_work = match app.state::<RemoteJobDrain>().has_pending_work() {
            Ok(has_work) => has_work,
            Err(error) => {
                if loop_now_ms >= next_pending_error_log_ms {
                    next_pending_error_log_ms = loop_now_ms.saturating_add(60_000);
                    crate::stt::log_yap(&format!(
                        "remote job drain state remains unavailable; retrying: {error}"
                    ));
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        if !has_work {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        let connector = app.state::<ServerConnector>();
        let runtime_state = app.state::<crate::runtime::RuntimeOrchestratorState>();
        let now = now_ms();
        match advance_persisted_cancellation_once(
            &app.state::<RemoteJobDrain>().ledger,
            &app.state::<RemoteJobDrain>().remote_jobs_directory,
            &connector,
            now,
        )
        .await
        {
            Ok(true) => {
                emit_jobs_changed(&app);
                continue;
            }
            Ok(false) => {}
            Err(error) => {
                crate::stt::log_yap(&format!(
                    "remote cancellation remains pending after a bounded request: {error}"
                ));
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        }
        if connector.batch_connection_lease().ok().flatten().is_none() {
            connector.refresh_for_job_drain(&app, &runtime_state).await;
        }
        if connector.batch_connection_lease().ok().flatten().is_none() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        let prepare_app = app.clone();
        let prepared = tauri::async_runtime::spawn_blocking(move || {
            let drain = prepare_app.state::<RemoteJobDrain>();
            prepare_next_queued_job(
                &drain.ledger,
                &drain.owned_live_directory,
                &drain.remote_jobs_directory,
                &drain.owner_namespace,
                now,
                SystemTime::now(),
            )
        })
        .await;
        match prepared {
            Ok(Ok(true)) => {
                emit_jobs_changed(&app);
                continue;
            }
            Ok(Ok(false)) => {}
            Ok(Err(error)) => {
                crate::stt::log_yap(&format!("remote preprocessing stopped safely: {error}"));
                app.state::<RemoteJobDrain>()
                    .fail_preprocessing_candidate(now);
                emit_jobs_changed(&app);
                continue;
            }
            Err(error) => {
                crate::stt::log_yap(&format!("remote preprocessing worker failed: {error}"));
                app.state::<RemoteJobDrain>()
                    .fail_preprocessing_candidate(now);
                emit_jobs_changed(&app);
                continue;
            }
        }

        let Some(lease) = connector.batch_connection_lease().ok().flatten() else {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        let drain = app.state::<RemoteJobDrain>();
        match advance_upload_with_lease(
            &drain.ledger,
            &drain.remote_jobs_directory,
            &connector,
            &lease,
            now,
        )
        .await
        {
            Ok(true) => {
                emit_jobs_changed(&app);
                continue;
            }
            Ok(false) => {}
            Err(error) => {
                crate::stt::log_yap(&format!("remote upload step will not commit: {error}"));
                drain.schedule_remote_retry(&[RecordingJobStatus::Uploading], &error, now);
                emit_jobs_changed(&app);
                continue;
            }
        }

        let Some(lease) = connector.batch_connection_lease().ok().flatten() else {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        match advance_processing_with_lease(
            &drain.ledger,
            &drain.remote_jobs_directory,
            &connector,
            &lease,
            now,
        )
        .await
        {
            Ok(true) => emit_jobs_changed(&app),
            Ok(false) => {}
            Err(error) => {
                crate::stt::log_yap(&format!("remote result step will not commit: {error}"));
                drain.schedule_remote_retry(
                    &[
                        RecordingJobStatus::ServerProcessing,
                        RecordingJobStatus::Saving,
                    ],
                    &error,
                    now,
                );
                emit_jobs_changed(&app);
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn emit_jobs_changed(app: &tauri::AppHandle) {
    if let Err(error) = app.emit_to(
        crate::authorization::MAIN_WINDOW_LABEL,
        "recording-jobs-changed",
        (),
    ) {
        crate::stt::log_yap(&format!(
            "recording jobs event failed after background commit: {error}"
        ));
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn prepare_next_queued_job(
    ledger: &JobLedger,
    owned_live_directory: &Path,
    remote_jobs_directory: &Path,
    owner_namespace: &OwnerNamespace,
    updated_at_ms: u64,
    started_at: SystemTime,
) -> Result<bool, String> {
    let candidate = ledger
        .list_recoverable_jobs()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|job| {
            matches!(
                job.status,
                RecordingJobStatus::QueuedServer | RecordingJobStatus::Preprocessing
            ) && job
                .next_attempt_at_ms
                .is_none_or(|retry_at| retry_at <= updated_at_ms)
        });
    let Some(mut candidate) = candidate else {
        return Ok(false);
    };
    if candidate.status == RecordingJobStatus::QueuedServer {
        candidate = ledger
            .transition(
                &candidate.job_id,
                RecordingJobStatus::Preprocessing,
                updated_at_ms,
            )
            .map_err(|error| error.to_string())?;
    }
    if ledger
        .get_prepared_remote_job(&candidate.job_id)
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("preprocessing job already has durable remote state".into());
    }
    let source_path = candidate
        .source_path
        .as_deref()
        .ok_or_else(|| "imported recording has no source path".to_string())?;
    let validated =
        crate::file_actions::validate_recording_job_source_at(source_path, owned_live_directory)
            .map_err(|error| match error {
                crate::file_actions::RecordingJobSourceError::Missing => {
                    "imported recording source is missing".to_string()
                }
                crate::file_actions::RecordingJobSourceError::Unsafe(message) => message,
            })?;
    let mut source = crate::commands::media_protocol::open_unchanged_media_source(
        &validated.canonical_path,
        &validated.fingerprint,
    )?;
    remote::reset_unattached_spool(&candidate.job_id, remote_jobs_directory)?;
    let prepared = remote::prepare_imported_pcm_wav(
        &candidate.job_id,
        &candidate.display_name,
        &mut source,
        remote_jobs_directory,
        owner_namespace,
        started_at,
    )?
    .into_ledger_state()?;
    attach_prepared_remote_job_or_cleanup(
        ledger,
        &candidate.job_id,
        &prepared,
        remote_jobs_directory,
        updated_at_ms,
    )?;
    Ok(true)
}

fn attach_prepared_remote_job_or_cleanup(
    ledger: &JobLedger,
    job_id: &str,
    prepared: &crate::jobs::NewPreparedRemoteJob,
    remote_jobs_directory: &Path,
    updated_at_ms: u64,
) -> Result<(), String> {
    match ledger.attach_prepared_remote_job(job_id, prepared, updated_at_ms) {
        Ok(_) => Ok(()),
        Err(error) => {
            remote::reset_unattached_spool(job_id, remote_jobs_directory).map_err(
                |cleanup_error| {
                    format!(
                        "durable preprocessing commit failed ({error}); owned spool cleanup also failed ({cleanup_error})"
                    )
                },
            )?;
            Err(error.to_string())
        }
    }
}

#[cfg(test)]
async fn advance_upload_once(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    client: &BatchApiClient,
    updated_at_ms: u64,
) -> DrainResult<bool> {
    advance_upload_once_guarded(
        ledger,
        remote_jobs_directory,
        client,
        updated_at_ms,
        &BatchCommitGuard::Unchecked,
    )
    .await
}

async fn advance_upload_with_lease(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    connector: &ServerConnector,
    lease: &BatchConnectionLease,
    updated_at_ms: u64,
) -> DrainResult<bool> {
    advance_upload_once_guarded(
        ledger,
        remote_jobs_directory,
        lease.client(),
        updated_at_ms,
        &BatchCommitGuard::Lease { connector, lease },
    )
    .await
}

async fn advance_upload_once_guarded(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    client: &BatchApiClient,
    updated_at_ms: u64,
    guard: &BatchCommitGuard<'_>,
) -> DrainResult<bool> {
    let candidate = ledger
        .list_recoverable_jobs()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|job| {
            job.status == RecordingJobStatus::Uploading
                && job
                    .next_attempt_at_ms
                    .is_none_or(|retry_at| retry_at <= updated_at_ms)
        });
    let Some(candidate) = candidate else {
        return Ok(false);
    };
    let prepared = ledger
        .get_prepared_remote_job(&candidate.job_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "uploading job has no durable remote state".to_string())?;
    let request = CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json)?;
    let chunks = ledger
        .list_chunks(&candidate.job_id)
        .map_err(|error| error.to_string())?;
    validate_durable_upload_state(&candidate, &prepared, &request, &chunks)?;
    if prepared.server_job_id.is_some()
        && prepared.server_base_url.as_deref() != Some(client.base_url_identity())
    {
        return Err("uploading job is bound to a different server origin".into());
    }
    if prepared.server_job_id.is_none()
        && prepared
            .create_attempt_base_url
            .as_deref()
            .is_some_and(|origin| origin != client.base_url_identity())
    {
        return Err("uploading job has a create attempt at a different server origin".into());
    }

    let Some(server_job_id) = prepared.server_job_id.as_deref() else {
        let idempotency_key = request.create_idempotency_key()?;
        guard.commit(|| {
            ledger
                .begin_remote_create_attempt(
                    &candidate.job_id,
                    client.base_url_identity(),
                    updated_at_ms,
                )
                .map_err(|error| DrainStepError::permanent(error.to_string()))
        })?;
        guard.ensure_current()?;
        let projection = client.create(&idempotency_key, &request).await?;
        validate_job_projection(&projection, &request, None, &["accepted", "uploading"])?;
        ledger
            .record_server_job_id(
                &candidate.job_id,
                &projection.job_id,
                client.base_url_identity(),
                updated_at_ms,
            )
            .map_err(|error| DrainStepError::permanent(error.to_string()))?;
        guard.ensure_current()?;
        return Ok(true);
    };

    if let Some((record, reference)) = chunks
        .iter()
        .zip(&request.chunks)
        .find(|(record, _)| record.acknowledged_at_ms.is_none())
    {
        let body =
            remote::read_prepared_chunk(&record.artifact_path, remote_jobs_directory, reference)?;
        guard.ensure_current()?;
        let receipt = client.upload_chunk(server_job_id, reference, body).await?;
        if receipt.replay_key.schema_version != reference.replay_key.schema_version
            || receipt.replay_key.session_id != reference.replay_key.session_id
            || receipt.replay_key.track_id != reference.replay_key.track_id
            || receipt.replay_key.sequence_start != reference.replay_key.sequence_start
            || receipt.replay_key.sequence_end != reference.replay_key.sequence_end
            || receipt.content_identity.sha256 != reference.content_identity.sha256
            || receipt.content_identity.byte_length != reference.content_identity.byte_length
            || !matches!(receipt.disposition.as_str(), "accepted" | "replayed")
            || receipt.accepted_at_utc.is_empty()
        {
            return Err("server chunk receipt conflicts with durable upload identity".into());
        }
        ledger
            .acknowledge_remote_chunk(
                &candidate.job_id,
                &reference.replay_key.track_id,
                reference.replay_key.sequence_start,
                reference.replay_key.sequence_end,
                &reference.content_identity.sha256,
                updated_at_ms,
            )
            .map_err(|error| DrainStepError::permanent(error.to_string()))?;
        guard.ensure_current()?;
        return Ok(true);
    }

    guard.ensure_current()?;
    let status = client.status(server_job_id).await?;
    validate_job_projection(
        &status,
        &request,
        Some(server_job_id),
        &["accepted", "uploading", "server_processing", "complete"],
    )?;
    if matches!(status.status.as_str(), "accepted" | "uploading") {
        guard.ensure_current()?;
        let committed = client
            .commit(
                server_job_id,
                &CommitRecordingJobRequest {
                    capture_manifest: request.capture_manifest.clone(),
                    chunk_count: request.chunks.len(),
                },
            )
            .await?;
        validate_job_projection(
            &committed,
            &request,
            Some(server_job_id),
            &["server_processing", "complete"],
        )?;
    }
    ledger
        .mark_remote_job_committed(&candidate.job_id, updated_at_ms)
        .map_err(|error| DrainStepError::permanent(error.to_string()))?;
    guard.ensure_current()?;
    Ok(true)
}

async fn advance_persisted_cancellation_once(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    connector: &ServerConnector,
    updated_at_ms: u64,
) -> DrainResult<bool> {
    if cleanup_next_owned_spool_once(ledger, remote_jobs_directory)? {
        return Ok(true);
    }
    if let Some(origin) = next_persisted_cancellation_origin(ledger)? {
        let client = connector
            .persisted_cleanup_client(&origin)
            .map_err(DrainStepError::permanent)?;
        return advance_cancellation_once_guarded(
            ledger,
            remote_jobs_directory,
            &client,
            updated_at_ms,
            &BatchCommitGuard::PersistedCleanup,
        )
        .await;
    }

    if recover_cancelled_create_attempt_once(
        ledger,
        remote_jobs_directory,
        connector,
        updated_at_ms,
    )
    .await?
    {
        return Ok(true);
    }

    let configured_origin = match connector.configured_batch_origin() {
        Ok(origin) => origin,
        Err(_) => return Ok(false),
    };
    if detach_changed_remote_binding_once(
        ledger,
        remote_jobs_directory,
        configured_origin.as_deref(),
        updated_at_ms,
    )? {
        return Ok(true);
    }
    recover_abandoned_create_attempt_once(
        ledger,
        remote_jobs_directory,
        connector,
        configured_origin.as_deref(),
        updated_at_ms,
    )
    .await
}

fn cleanup_next_owned_spool_once(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
) -> DrainResult<bool> {
    let pending = ledger
        .list_pending_remote_spool_cleanup()
        .map_err(|error| error.to_string())?
        .into_iter()
        .next();
    let Some(job_id) = pending else {
        return Ok(false);
    };
    cleanup_owned_spool_and_ack(
        ledger,
        remote_jobs_directory,
        &job_id,
        "owned spool cleanup lost its durable acknowledgement",
    )?;
    Ok(true)
}

fn next_persisted_cancellation_origin(ledger: &JobLedger) -> DrainResult<Option<String>> {
    let pending_origin = ledger
        .list_pending_remote_cancellations()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find_map(|candidate| candidate.server_base_url);
    match pending_origin {
        Some(origin) => Ok(Some(origin)),
        None => Ok(ledger
            .list_detached_remote_cancellations()
            .map_err(|error| error.to_string())?
            .into_iter()
            .next()
            .map(|candidate| candidate.server_base_url)),
    }
}

async fn recover_cancelled_create_attempt_once(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    connector: &ServerConnector,
    updated_at_ms: u64,
) -> DrainResult<bool> {
    let cancelled_attempt = ledger
        .list_cancelled_remote_create_attempts()
        .map_err(|error| error.to_string())?
        .into_iter()
        .next();
    let Some(cancelled_attempt) = cancelled_attempt else {
        return Ok(false);
    };
    let origin = cancelled_attempt
        .create_attempt_base_url
        .as_deref()
        .ok_or_else(|| {
            DrainStepError::permanent("cancelled create cleanup omitted its persisted origin")
        })?;
    let client = connector
        .persisted_cleanup_client(origin)
        .map_err(DrainStepError::permanent)?;
    let (request, projection) =
        recreate_server_job_for_cleanup(&client, &cancelled_attempt).await?;
    ledger
        .record_server_job_id(
            &cancelled_attempt.job_id,
            &projection.job_id,
            origin,
            updated_at_ms,
        )
        .map_err(|error| DrainStepError::permanent(error.to_string()))?;
    cancel_server_job(&client, &projection.job_id, &request).await?;
    remote::reset_unattached_spool(&cancelled_attempt.job_id, remote_jobs_directory)
        .map_err(DrainStepError::permanent)?;
    ledger
        .acknowledge_server_cancellation(
            &cancelled_attempt.job_id,
            &projection.job_id,
            updated_at_ms,
        )
        .map_err(|error| DrainStepError::permanent(error.to_string()))?;
    Ok(true)
}

fn detach_changed_remote_binding_once(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    configured_origin: Option<&str>,
    updated_at_ms: u64,
) -> DrainResult<bool> {
    if let Some(job_id) = ledger
        .detach_changed_remote_binding(configured_origin, updated_at_ms, |job_id| {
            remote::reset_unattached_spool(job_id, remote_jobs_directory)
        })
        .map_err(|error| error.to_string())?
    {
        debug_assert!(!job_id.is_empty());
        return Ok(true);
    }
    Ok(false)
}

async fn recover_abandoned_create_attempt_once(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    connector: &ServerConnector,
    configured_origin: Option<&str>,
    updated_at_ms: u64,
) -> DrainResult<bool> {
    let abandoned = ledger
        .list_remote_create_attempts()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|candidate| candidate.create_attempt_base_url.as_deref() != configured_origin);
    let Some(abandoned) = abandoned else {
        return Ok(false);
    };
    let origin = abandoned
        .create_attempt_base_url
        .as_deref()
        .ok_or_else(|| DrainStepError::permanent("create cleanup omitted its persisted origin"))?;
    let client = connector
        .persisted_cleanup_client(origin)
        .map_err(DrainStepError::permanent)?;
    let (request, projection) = recreate_server_job_for_cleanup(&client, &abandoned).await?;
    cancel_server_job(&client, &projection.job_id, &request).await?;
    ledger
        .fail_abandoned_remote_create_attempt(&abandoned.job_id, origin, updated_at_ms, || {
            remote::reset_unattached_spool(&abandoned.job_id, remote_jobs_directory)
        })
        .map_err(|error| DrainStepError::permanent(error.to_string()))?;
    Ok(true)
}

async fn recreate_server_job_for_cleanup(
    client: &BatchApiClient,
    candidate: &PreparedRemoteJobRecord,
) -> DrainResult<(CreateRecordingJobRequest, RecordingJob)> {
    let request = CreateRecordingJobRequest::decode_persisted(&candidate.create_request_json)?;
    let idempotency_key = request.create_idempotency_key()?;
    let projection = client.create(&idempotency_key, &request).await?;
    validate_job_projection(
        &projection,
        &request,
        None,
        &[
            "accepted",
            "uploading",
            "server_processing",
            "complete",
            "failed",
            "cancelled",
        ],
    )?;
    Ok((request, projection))
}

fn cleanup_owned_spool_and_ack(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    job_id: &str,
    missing_acknowledgement: &'static str,
) -> DrainResult<()> {
    remote::reset_unattached_spool(job_id, remote_jobs_directory)
        .map_err(DrainStepError::permanent)?;
    let acknowledged = ledger
        .acknowledge_remote_spool_cleanup(job_id)
        .map_err(|error| DrainStepError::permanent(error.to_string()))?;
    if !acknowledged {
        return Err(DrainStepError::permanent(missing_acknowledgement));
    }
    Ok(())
}

#[cfg(test)]
async fn advance_cancellation_once(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    client: &BatchApiClient,
    updated_at_ms: u64,
) -> DrainResult<bool> {
    advance_cancellation_once_guarded(
        ledger,
        remote_jobs_directory,
        client,
        updated_at_ms,
        &BatchCommitGuard::Unchecked,
    )
    .await
}

async fn advance_cancellation_once_guarded(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    client: &BatchApiClient,
    updated_at_ms: u64,
    guard: &BatchCommitGuard<'_>,
) -> DrainResult<bool> {
    let candidate = ledger
        .list_pending_remote_cancellations()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|candidate| candidate.server_base_url.as_deref() == Some(client.base_url_identity()));
    if let Some(candidate) = candidate {
        let server_job_id = candidate
            .server_job_id
            .as_deref()
            .ok_or_else(|| "pending cancellation has no bound server job ID".to_string())?;
        let request = CreateRecordingJobRequest::decode_persisted(&candidate.create_request_json)?;
        guard.ensure_current()?;
        cancel_server_job(client, server_job_id, &request).await?;
        remote::reset_unattached_spool(&candidate.job_id, remote_jobs_directory)
            .map_err(DrainStepError::permanent)?;
        guard.commit(|| {
            ledger
                .acknowledge_server_cancellation(&candidate.job_id, server_job_id, updated_at_ms)
                .map_err(|error| DrainStepError::permanent(error.to_string()))
        })?;
        return Ok(true);
    }

    let detached = ledger
        .list_detached_remote_cancellations()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|candidate| candidate.server_base_url == client.base_url_identity());
    let Some(detached) = detached else {
        return Ok(false);
    };
    let request = CreateRecordingJobRequest::decode_persisted(&detached.create_request_json)?;
    guard.ensure_current()?;
    cancel_server_job(client, &detached.server_job_id, &request).await?;
    guard.commit(|| {
        ledger
            .acknowledge_detached_remote_cancellation(
                &detached.server_base_url,
                &detached.server_job_id,
            )
            .map_err(|error| DrainStepError::permanent(error.to_string()))
    })?;
    Ok(true)
}

async fn cancel_server_job(
    client: &BatchApiClient,
    server_job_id: &str,
    request: &CreateRecordingJobRequest,
) -> DrainResult<()> {
    match client.cancel(server_job_id).await {
        Ok(projection) => {
            validate_job_projection(&projection, request, Some(server_job_id), &["cancelled"])?
        }
        Err(BatchClientError::Api { status, code, .. })
            if status == reqwest::StatusCode::NOT_FOUND && code == "JOB_NOT_FOUND" => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(test)]
async fn advance_processing_once(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    client: &BatchApiClient,
    updated_at_ms: u64,
) -> DrainResult<bool> {
    advance_processing_once_guarded(
        ledger,
        remote_jobs_directory,
        client,
        updated_at_ms,
        &BatchCommitGuard::Unchecked,
    )
    .await
}

async fn advance_processing_with_lease(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    connector: &ServerConnector,
    lease: &BatchConnectionLease,
    updated_at_ms: u64,
) -> DrainResult<bool> {
    advance_processing_once_guarded(
        ledger,
        remote_jobs_directory,
        lease.client(),
        updated_at_ms,
        &BatchCommitGuard::Lease { connector, lease },
    )
    .await
}

async fn advance_processing_once_guarded(
    ledger: &JobLedger,
    remote_jobs_directory: &Path,
    client: &BatchApiClient,
    updated_at_ms: u64,
    guard: &BatchCommitGuard<'_>,
) -> DrainResult<bool> {
    let candidate = ledger
        .list_recoverable_jobs()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|job| {
            matches!(
                job.status,
                RecordingJobStatus::ServerProcessing | RecordingJobStatus::Saving
            ) && job
                .next_attempt_at_ms
                .is_none_or(|retry_at| retry_at <= updated_at_ms)
        });
    let Some(candidate) = candidate else {
        return Ok(false);
    };
    let prepared = ledger
        .get_prepared_remote_job(&candidate.job_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "server-processing job has no durable remote state".to_string())?;
    let server_job_id = prepared
        .server_job_id
        .as_deref()
        .ok_or_else(|| "server-processing job has no bound server job ID".to_string())?;
    if prepared.server_base_url.as_deref() != Some(client.base_url_identity()) {
        return Err("server-processing job is bound to a different server origin".into());
    }
    let request = CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json)?;
    let result_expires_at_ms = result_retention_expiry_ms(&request)?;
    let chunks = ledger
        .list_chunks(&candidate.job_id)
        .map_err(|error| error.to_string())?;
    validate_durable_upload_state(&candidate, &prepared, &request, &chunks)?;

    guard.ensure_current()?;
    let projection = client.status(server_job_id).await?;
    validate_job_projection(
        &projection,
        &request,
        Some(server_job_id),
        &["server_processing", "complete", "failed", "cancelled"],
    )?;
    guard.ensure_current()?;
    if projection.status == "server_processing" {
        return Ok(false);
    }
    if projection.status == "failed" {
        let error = projection.error.as_ref().ok_or_else(|| {
            DrainStepError::permanent("failed server projection omitted its typed error")
        })?;
        return Err(DrainStepError::terminal_server(error));
    }
    if projection.status != "complete" {
        return Err(DrainStepError::permanent(format!(
            "server job entered terminal status {} before publishing a result",
            projection.status
        )));
    }

    guard.ensure_current()?;
    let result = client.result(server_job_id).await?;
    validate_result_revision(&result, &request)?;
    guard.commit(|| {
        ledger
            .begin_remote_result_saving(&candidate.job_id, updated_at_ms)
            .map_err(|error| DrainStepError::permanent(error.to_string()))
    })?;
    let output_path =
        remote::publish_remote_result(&candidate.job_id, remote_jobs_directory, &result)?;
    guard.commit(|| {
        ledger
            .complete_remote_result(
                &candidate.job_id,
                &output_path,
                result_expires_at_ms,
                updated_at_ms,
            )
            .map_err(|error| DrainStepError::permanent(error.to_string()))
    })?;
    Ok(true)
}

fn validate_durable_upload_state(
    job: &RecordingJobRecord,
    prepared: &PreparedRemoteJobRecord,
    request: &CreateRecordingJobRequest,
    chunks: &[JobChunkRecord],
) -> Result<(), String> {
    let session_id = request.metadata.session_id.as_str();
    if prepared.job_id != job.job_id
        || prepared.capture_manifest_sha256 != request.capture_manifest.sha256
        || job.capture_manifest_sha256.as_deref() != Some(request.capture_manifest.sha256.as_str())
        || chunks.len() != request.chunks.len()
        || chunks.is_empty()
    {
        return Err("durable upload state conflicts with its prepared request".into());
    }
    for (record, reference) in chunks.iter().zip(&request.chunks) {
        validate_durable_chunk(job, session_id, prepared, record, reference)?;
    }
    Ok(())
}

fn validate_durable_chunk(
    job: &RecordingJobRecord,
    session_id: &str,
    prepared: &PreparedRemoteJobRecord,
    record: &JobChunkRecord,
    reference: &CaptureChunkReference,
) -> Result<(), String> {
    let replay = &reference.replay_key;
    let content = &reference.content_identity;
    if record.job_id != job.job_id
        || record.session_id != session_id
        || record.track_id != replay.track_id
        || record.sequence_start != replay.sequence_start
        || record.sequence_end != replay.sequence_end
        || record.content_sha256 != content.sha256
        || record.content_byte_length != content.byte_length
    {
        return Err("durable chunk state conflicts with its prepared request".into());
    }
    match record.acknowledged_at_ms {
        Some(_) => {
            if record.upload_offset != record.content_byte_length
                || record.acknowledged_object_id.as_deref() != prepared.server_job_id.as_deref()
            {
                return Err("durable chunk acknowledgement is inconsistent".into());
            }
        }
        None => {
            if record.upload_offset != 0 || record.acknowledged_object_id.is_some() {
                return Err("unacknowledged durable chunk contains upload progress".into());
            }
        }
    }
    Ok(())
}

fn validate_job_projection(
    projection: &RecordingJob,
    request: &CreateRecordingJobRequest,
    expected_job_id: Option<&str>,
    allowed_statuses: &[&str],
) -> Result<(), String> {
    let manifest = &projection.capture_manifest;
    let error_is_valid = match (projection.status.as_str(), projection.error.as_ref()) {
        ("failed", Some(error)) => valid_server_job_error(error),
        ("failed", None) => false,
        (_, None) => true,
        (_, Some(_)) => false,
    };
    if expected_job_id.is_some_and(|expected| projection.job_id != expected)
        || projection.job_id.is_empty()
        || projection.session_id != request.metadata.session_id.as_str()
        || projection.display_name != request.display_name
        || projection.session_mode != "meeting"
        || projection.session_origin != "imported_file"
        || projection.route.as_deref() != Some("server_batch")
        || manifest.schema_version != request.capture_manifest.schema_version
        || manifest.session_id != request.capture_manifest.session_id
        || manifest.sha256 != request.capture_manifest.sha256
        || manifest.byte_length != request.capture_manifest.byte_length
        || !allowed_statuses.contains(&projection.status.as_str())
        || !error_is_valid
        || projection.created_at_utc.is_empty()
        || projection.updated_at_utc.is_empty()
    {
        return Err("server job projection conflicts with the prepared recording".into());
    }
    Ok(())
}

fn valid_server_job_error(error: &ApiError) -> bool {
    error.is_valid()
}

fn validate_result_revision(
    result: &TranscriptResultRevision,
    request: &CreateRecordingJobRequest,
) -> Result<(), String> {
    let expected_language = request
        .metadata
        .preferred_languages_bcp47
        .first()
        .ok_or_else(|| "prepared recording has no preferred result language".to_string())?;
    let language = result
        .language
        .as_ref()
        .ok_or_else(|| "server result omitted its language decision".to_string())?;
    let timestamp_valid = result.created_at_utc.ends_with('Z')
        && result.created_at_utc.len() <= 64
        && OffsetDateTime::parse(&result.created_at_utc, &Rfc3339).is_ok();
    let language_valid = language.language_bcp47 == *expected_language
        && language.language_bcp47.len() <= 35
        && language
            .language_bcp47
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && language
            .confidence
            .is_none_or(|confidence| (0.0..=1.0).contains(&confidence));
    let provenance_valid = !result.model_provenance.is_empty()
        && result.model_provenance.len() <= 8
        && result.model_provenance.iter().all(|model| {
            [
                model.model_id.as_str(),
                model.revision.as_str(),
                model.calibration_revision.as_str(),
            ]
            .iter()
            .all(|value| !value.is_empty() && value.len() <= 256)
        });
    if result.session_id != request.metadata.session_id.as_str()
        || result.revision != 1
        || result.authority != "server_authoritative"
        || !timestamp_valid
        || result.capture_manifest_sha256 != request.capture_manifest.sha256
        || result.previous_result_sha256.is_some()
        || result.status != "complete"
        || !language_valid
        || result.transcript.trim().is_empty()
        || result.transcript.len() > 2 * 1024 * 1024 - 1
        || !result.aligned_words.is_empty()
        || !provenance_valid
    {
        return Err("server result revision conflicts with the prepared recording".into());
    }
    Ok(())
}

fn result_retention_expiry_ms(request: &CreateRecordingJobRequest) -> Result<u64, String> {
    let encoded = request
        .metadata
        .retention_expires_at_utc
        .as_deref()
        .filter(|value| value.ends_with('Z'))
        .ok_or_else(|| "prepared meeting job has no UTC result retention expiry".to_string())?;
    let parsed = OffsetDateTime::parse(encoded, &Rfc3339)
        .map_err(|_| "prepared meeting job has an invalid result retention expiry".to_string())?;
    let milliseconds = parsed.unix_timestamp_nanos().div_euclid(1_000_000);
    u64::try_from(milliseconds)
        .map_err(|_| "prepared meeting result retention expiry is out of range".to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File},
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc, Mutex,
        },
        thread,
        time::{Duration, UNIX_EPOCH},
    };

    use crate::{
        audio::session::OwnerNamespace,
        jobs::{
            JobLedger, NewRecordingJob, RecordingJobStatus, RecordingRoute, SessionMode,
            SessionOrigin, SourceOwnership,
        },
        server_connector::{
            batch::{ApiError, BatchApiClient, CreateRecordingJobRequest},
            config::ServerSettings,
            ServerConnector, ServerConnectorBoundary,
        },
    };

    use super::{
        advance_cancellation_once, advance_persisted_cancellation_once,
        advance_processing_once_guarded, advance_upload_once, advance_upload_once_guarded,
        attach_prepared_remote_job_or_cleanup, prepare_next_queued_job, remote_retry_plan,
        validate_result_revision, BatchCommitGuard, DrainStepError, RemoteJobDrain,
    };

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn retention_drain_removes_pruned_private_spools_before_acknowledging_cleanup() {
        let dir = temp_dir("pruned-spool-cleanup");
        let remote_jobs_directory = dir.join("remote-jobs");
        let job_id = "job-0123456789abcdef01234567";
        let owned_spool = remote_jobs_directory.join(job_id);
        fs::create_dir_all(&owned_spool).unwrap();
        fs::write(owned_spool.join("private.pcm"), b"private bytes").unwrap();
        let ledger = JobLedger::open_in_memory().unwrap();
        {
            let connection = ledger.connection.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO remote_spool_cleanup (job_id, queued_at_ms) VALUES (?1, 1)",
                    [job_id],
                )
                .unwrap();
        }
        let drain = RemoteJobDrain {
            ledger,
            owner_namespace: OwnerNamespace::local("i-pruned-spool-test").unwrap(),
            owned_live_directory: dir.join("recordings"),
            remote_jobs_directory,
        };

        assert!(drain.has_pending_work().unwrap());
        assert!(drain.enforce_retention(2).unwrap());
        assert!(!owned_spool.exists());
        assert!(drain
            .ledger
            .list_pending_remote_spool_cleanup()
            .unwrap()
            .is_empty());

        drop(drain);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pending_owned_spool_cleanup_does_not_require_initialized_server_settings() {
        let dir = temp_dir("pending-spool-cleanup");
        let remote_jobs_directory = dir.join("remote-jobs");
        let job_id = "job-0123456789abcdef01234567";
        let owned_spool = remote_jobs_directory.join(job_id);
        fs::create_dir_all(&owned_spool).unwrap();
        fs::write(owned_spool.join("private.pcm"), b"private bytes").unwrap();
        let ledger = JobLedger::open_in_memory().unwrap();
        {
            let connection = ledger.connection.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO remote_spool_cleanup (job_id, queued_at_ms) VALUES (?1, 1)",
                    [job_id],
                )
                .unwrap();
        }
        let connector = ServerConnector::new();

        let cleaned = tauri::async_runtime::block_on(advance_persisted_cancellation_once(
            &ledger,
            &remote_jobs_directory,
            &connector,
            2,
        ))
        .unwrap();

        assert!(cleaned);
        assert!(!owned_spool.exists());
        assert!(ledger
            .list_pending_remote_spool_cleanup()
            .unwrap()
            .is_empty());

        drop(ledger);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn automatic_remote_retries_are_typed_and_bounded() {
        let transient = DrainStepError::transient_state("request timed out");
        let permanent = DrainStepError::permanent("manifest conflicts with durable state");

        assert_eq!(
            remote_retry_plan(&transient, 0, 10_000),
            (
                Some(11_000),
                "REMOTE_REQUEST_RETRYING",
                "The private-server request did not complete. Yap will retry automatically.",
            )
        );
        assert_eq!(
            remote_retry_plan(&transient, 6, 10_000),
            (
                None,
                "REMOTE_RETRY_EXHAUSTED",
                "The private-server request did not recover after bounded retries. Retry the recording to start a new server job.",
            )
        );
        assert_eq!(
            remote_retry_plan(&permanent, 0, 10_000),
            (
                None,
                "REMOTE_STATE_INVALID",
                "The private-server job state is incompatible. Retry the recording to start a new server job.",
            )
        );
    }

    #[test]
    fn terminal_server_diagnostics_do_not_copy_server_controlled_messages() {
        let private_message = "Private transcript and C:/private/audio.wav";
        let error = ApiError {
            code: "ASR_WORKER_FAILED".into(),
            message: private_message.into(),
            retryable: true,
            request_id: "job-abc123".into(),
        };

        let diagnostic = DrainStepError::terminal_server(&error);

        assert!(diagnostic.detail.contains("ASR_WORKER_FAILED"));
        assert!(diagnostic.detail.contains("job-abc123"));
        assert!(!diagnostic.detail.contains(private_message));
    }

    #[test]
    fn queued_wav_is_preprocessed_into_durable_owned_replay_state() {
        let root = temp_dir("prepare");
        let database = root.join("jobs.sqlite3");
        let source = root.join("source.wav");
        let owned_live = root.join("live-recordings");
        let remote_jobs = root.join("remote-jobs");
        fs::create_dir_all(&owned_live).unwrap();
        write_pcm_wav(&source, &vec![0_u8; 320]);
        let original = fs::read(&source).unwrap();
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&NewRecordingJob {
                job_id: "job-drain-prepare".into(),
                session_mode: SessionMode::Meeting,
                session_origin: SessionOrigin::ImportedFile,
                source_path: Some(source.clone()),
                source_ownership: SourceOwnership::External,
                output_path: None,
                display_name: "source.wav".into(),
                status: RecordingJobStatus::QueuedServer,
                route: Some(RecordingRoute::ServerBatch),
                attempt_count: 0,
                next_attempt_at_ms: None,
                cancellation_requested: false,
                capture_commit_path: None,
                capture_manifest_sha256: None,
                error_code: None,
                error_message: None,
                created_at_ms: 1_720_000_000_000,
                updated_at_ms: 1_720_000_000_000,
                expires_at_ms: Some(1_720_604_800_000),
            })
            .unwrap();
        let owner = OwnerNamespace::local("i-drain-test").unwrap();

        assert!(prepare_next_queued_job(
            &ledger,
            &owned_live,
            &remote_jobs,
            &owner,
            1_720_000_000_100,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap());

        let job = ledger.get_job("job-drain-prepare").unwrap().unwrap();
        assert_eq!(job.status, RecordingJobStatus::Uploading);
        let prepared = ledger
            .get_prepared_remote_job("job-drain-prepare")
            .unwrap()
            .unwrap();
        assert!(prepared.capture_manifest_path.is_file());
        assert_eq!(ledger.list_chunks("job-drain-prepare").unwrap().len(), 1);
        assert_eq!(fs::read(source).unwrap(), original);

        drop(ledger);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_cancelled_preprocessing_race_removes_the_unattached_owned_spool() {
        let root = temp_dir("prepare-cancel-race");
        let database = root.join("jobs.sqlite3");
        let source = root.join("source.wav");
        let remote_jobs = root.join("remote-jobs");
        write_pcm_wav(&source, &vec![0_u8; 320]);
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&queued_job("job-prepare-cancel-race", source.clone()))
            .unwrap();
        ledger
            .transition(
                "job-prepare-cancel-race",
                RecordingJobStatus::Preprocessing,
                1_720_000_000_100,
            )
            .unwrap();
        let owner = OwnerNamespace::local("i-drain-test").unwrap();
        let mut source_file = File::open(&source).unwrap();
        let prepared = crate::jobs::remote::prepare_imported_pcm_wav(
            "job-prepare-cancel-race",
            "source.wav",
            &mut source_file,
            &remote_jobs,
            &owner,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap()
        .into_ledger_state()
        .unwrap();
        assert!(remote_jobs.join("job-prepare-cancel-race").is_dir());
        ledger
            .request_cancellation("job-prepare-cancel-race", 1_720_000_000_200)
            .unwrap();

        assert!(attach_prepared_remote_job_or_cleanup(
            &ledger,
            "job-prepare-cancel-race",
            &prepared,
            &remote_jobs,
            1_720_000_000_300,
        )
        .is_err());
        assert!(!remote_jobs.join("job-prepare-cancel-race").exists());
        assert!(source.is_file(), "external source must never be deleted");

        drop(ledger);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prepared_job_creates_uploads_and_commits_through_the_durable_contract() {
        let root = temp_dir("upload");
        let database = root.join("jobs.sqlite3");
        let source = root.join("source.wav");
        let owned_live = root.join("live-recordings");
        let remote_jobs = root.join("remote-jobs");
        fs::create_dir_all(&owned_live).unwrap();
        write_pcm_wav(&source, &vec![0_u8; 320]);
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&queued_job("job-drain-upload", source))
            .unwrap();
        let owner = OwnerNamespace::local("i-drain-test").unwrap();
        prepare_next_queued_job(
            &ledger,
            &owned_live,
            &remote_jobs,
            &owner,
            1_720_000_000_100,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap();
        let prepared = ledger
            .get_prepared_remote_job("job-drain-upload")
            .unwrap()
            .unwrap();
        let request =
            CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json).unwrap();
        let create_idempotency_key = request.create_idempotency_key().unwrap();
        let server_job_id = "job-0123456789abcdef0123456789abcdef";
        let projection = |status: &str| {
            serde_json::json!({
                "jobId": server_job_id,
                "sessionId": request.metadata.session_id.as_str(),
                "displayName": request.display_name,
                "sessionMode": "meeting",
                "sessionOrigin": "imported_file",
                "status": status,
                "route": "server_batch",
                "captureManifest": request.capture_manifest,
                "createdAtUtc": "2026-07-14T21:00:00Z",
                "updatedAtUtc": "2026-07-14T21:00:01Z"
            })
        };
        let chunk = &request.chunks[0];
        let responses = vec![
            (202, projection("accepted")),
            (
                201,
                serde_json::json!({
                    "replayKey": chunk.replay_key,
                    "contentIdentity": chunk.content_identity,
                    "disposition": "accepted",
                    "acceptedAtUtc": "2026-07-14T21:00:01Z"
                }),
            ),
            (200, projection("uploading")),
            (202, projection("server_processing")),
        ];
        let (base_url, observed, server) = start_json_server(responses);
        let client = BatchApiClient::new(
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            &base_url,
        )
        .unwrap();

        tauri::async_runtime::block_on(async {
            assert!(
                advance_upload_once(&ledger, &remote_jobs, &client, 1_720_000_000_200,)
                    .await
                    .unwrap()
            );
            assert!(
                advance_upload_once(&ledger, &remote_jobs, &client, 1_720_000_000_300,)
                    .await
                    .unwrap()
            );
            assert!(
                advance_upload_once(&ledger, &remote_jobs, &client, 1_720_000_000_400,)
                    .await
                    .unwrap()
            );
        });
        server.join().unwrap();

        assert_eq!(
            ledger.get_job("job-drain-upload").unwrap().unwrap().status,
            RecordingJobStatus::ServerProcessing
        );
        let requests = observed.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[0].starts_with("POST /v1/jobs HTTP/1.1"));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains(&format!("idempotency-key: {create_idempotency_key}")));
        assert!(requests[1].starts_with(&format!(
            "PUT /v1/jobs/{server_job_id}/chunks/track-1/0-159 HTTP/1.1"
        )));
        assert!(requests[2].starts_with(&format!("GET /v1/jobs/{server_job_id} HTTP/1.1")));
        assert!(requests[3].starts_with(&format!("POST /v1/jobs/{server_job_id}/commit HTTP/1.1")));
        drop(requests);
        drop(ledger);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_lease_is_rejected_before_create_upload_or_processing_dispatch() {
        let root = temp_dir("stale-pre-dispatch");
        let database = root.join("jobs.sqlite3");
        let source = root.join("source.wav");
        let owned_live = root.join("live-recordings");
        let remote_jobs = root.join("remote-jobs");
        fs::create_dir_all(&owned_live).unwrap();
        write_pcm_wav(&source, &vec![0_u8; 320]);
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&queued_job("job-stale-pre-dispatch", source))
            .unwrap();
        let owner = OwnerNamespace::local("i-drain-test").unwrap();
        prepare_next_queued_job(
            &ledger,
            &owned_live,
            &remote_jobs,
            &owner,
            1_720_000_000_100,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let client = BatchApiClient::new(
            reqwest::Client::builder()
                .connect_timeout(Duration::from_millis(100))
                .timeout(Duration::from_millis(200))
                .build()
                .unwrap(),
            &base_url,
        )
        .unwrap();

        tauri::async_runtime::block_on(async {
            let create_error = advance_upload_once_guarded(
                &ledger,
                &remote_jobs,
                &client,
                1_720_000_000_200,
                &BatchCommitGuard::StaleForTest,
            )
            .await
            .unwrap_err();
            assert_eq!(create_error.detail, "test stale lease");

            ledger
                .begin_remote_create_attempt("job-stale-pre-dispatch", &base_url, 1_720_000_000_300)
                .unwrap();
            ledger
                .record_server_job_id(
                    "job-stale-pre-dispatch",
                    "job-0123456789abcdef0123456789abcdef",
                    &base_url,
                    1_720_000_000_300,
                )
                .unwrap();
            let upload_error = advance_upload_once_guarded(
                &ledger,
                &remote_jobs,
                &client,
                1_720_000_000_400,
                &BatchCommitGuard::StaleForTest,
            )
            .await
            .unwrap_err();
            assert_eq!(upload_error.detail, "test stale lease");
            assert!(ledger
                .list_chunks("job-stale-pre-dispatch")
                .unwrap()
                .iter()
                .all(|chunk| chunk.acknowledged_at_ms.is_none()));

            let chunk = ledger
                .list_chunks("job-stale-pre-dispatch")
                .unwrap()
                .into_iter()
                .next()
                .unwrap();
            ledger
                .acknowledge_remote_chunk(
                    "job-stale-pre-dispatch",
                    &chunk.track_id,
                    chunk.sequence_start,
                    chunk.sequence_end,
                    &chunk.content_sha256,
                    1_720_000_000_500,
                )
                .unwrap();
            ledger
                .mark_remote_job_committed("job-stale-pre-dispatch", 1_720_000_000_600)
                .unwrap();
            let processing_error = advance_processing_once_guarded(
                &ledger,
                &remote_jobs,
                &client,
                1_720_000_000_700,
                &BatchCommitGuard::StaleForTest,
            )
            .await
            .unwrap_err();
            assert_eq!(processing_error.detail, "test stale lease");
        });

        let prepared = ledger
            .get_prepared_remote_job("job-stale-pre-dispatch")
            .unwrap()
            .unwrap();
        assert_eq!(
            prepared.server_job_id.as_deref(),
            Some("job-0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            ledger
                .get_job("job-stale-pre-dispatch")
                .unwrap()
                .unwrap()
                .status,
            RecordingJobStatus::ServerProcessing
        );

        drop(ledger);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_response_binding_is_durable_when_the_connector_changes_in_flight() {
        let root = temp_dir("stale-create-response");
        let database = root.join("jobs.sqlite3");
        let source = root.join("source.wav");
        let owned_live = root.join("live-recordings");
        let remote_jobs = root.join("remote-jobs");
        fs::create_dir_all(&owned_live).unwrap();
        write_pcm_wav(&source, &vec![0_u8; 320]);
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&queued_job("job-stale-create-response", source))
            .unwrap();
        let owner = OwnerNamespace::local("i-drain-test").unwrap();
        prepare_next_queued_job(
            &ledger,
            &owned_live,
            &remote_jobs,
            &owner,
            1_720_000_000_100,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap();
        let prepared = ledger
            .get_prepared_remote_job("job-stale-create-response")
            .unwrap()
            .unwrap();
        let request =
            CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json).unwrap();
        let server_job_id = "job-0123456789abcdef0123456789abcdef";
        let response = serde_json::json!({
            "jobId": server_job_id,
            "sessionId": request.metadata.session_id.as_str(),
            "displayName": request.display_name,
            "sessionMode": "meeting",
            "sessionOrigin": "imported_file",
            "status": "accepted",
            "route": "server_batch",
            "captureManifest": request.capture_manifest,
            "createdAtUtc": "2026-07-14T21:00:00Z",
            "updatedAtUtc": "2026-07-14T21:00:01Z"
        });
        let (base_url, observed, server) = start_json_server(vec![(202, response)]);
        let client = BatchApiClient::new(
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            &base_url,
        )
        .unwrap();
        let remaining_successes = std::sync::atomic::AtomicUsize::new(2);

        let error = tauri::async_runtime::block_on(advance_upload_once_guarded(
            &ledger,
            &remote_jobs,
            &client,
            1_720_000_000_200,
            &BatchCommitGuard::StaleAfterForTest {
                remaining_successes: &remaining_successes,
            },
        ))
        .unwrap_err();
        server.join().unwrap();

        assert_eq!(error.detail, "test stale lease");
        let durable = ledger
            .get_prepared_remote_job("job-stale-create-response")
            .unwrap()
            .unwrap();
        assert_eq!(durable.server_job_id.as_deref(), Some(server_job_id));
        assert_eq!(durable.server_base_url.as_deref(), Some(base_url.as_str()));
        assert_eq!(durable.create_attempt_base_url, None);
        assert_eq!(observed.lock().unwrap().len(), 1);

        drop(ledger);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_conflict_does_not_acknowledge_detached_cancellation() {
        let root = temp_dir("detached-cancel");
        let database = root.join("jobs.sqlite3");
        let source = root.join("source.wav");
        let owned_live = root.join("live-recordings");
        let remote_jobs = root.join("remote-jobs");
        fs::create_dir_all(&owned_live).unwrap();
        write_pcm_wav(&source, &vec![0_u8; 320]);
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&queued_job("job-detached-cancel", source))
            .unwrap();
        let owner = OwnerNamespace::local("i-drain-test").unwrap();
        prepare_next_queued_job(
            &ledger,
            &owned_live,
            &remote_jobs,
            &owner,
            1_720_000_000_100,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap();
        let server_job_id = "job-0123456789abcdef0123456789abcdef";
        let (base_url, observed, server) = start_json_server(vec![(
            409,
            serde_json::json!({
                "code": "JOB_TERMINAL",
                "message": "The server job is already terminal.",
                "retryable": false,
                "requestId": "req-detached-cancel"
            }),
        )]);
        ledger
            .begin_remote_create_attempt("job-detached-cancel", &base_url, 1_720_000_000_200)
            .unwrap();
        ledger
            .record_server_job_id(
                "job-detached-cancel",
                server_job_id,
                &base_url,
                1_720_000_000_200,
            )
            .unwrap();
        ledger
            .record_remote_error(
                "job-detached-cancel",
                "REMOTE_RETRY_EXHAUSTED",
                "The private server request did not recover.",
                None,
                1_720_000_000_300,
            )
            .unwrap();
        ledger
            .retry_to_queued_server(
                "job-detached-cancel",
                1_720_000_000_400,
                Some(1_720_604_800_400),
            )
            .unwrap();
        let client = BatchApiClient::new(
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            &base_url,
        )
        .unwrap();

        tauri::async_runtime::block_on(async {
            let error =
                advance_cancellation_once(&ledger, &remote_jobs, &client, 1_720_000_000_500)
                    .await
                    .unwrap_err();
            assert_eq!(error.detail, "JOB_TERMINAL (HTTP 409)");
            assert!(!error.automatic_retry);
        });
        server.join().unwrap();

        assert_eq!(
            ledger.list_detached_remote_cancellations().unwrap().len(),
            1
        );
        assert!(observed.lock().unwrap()[0]
            .starts_with(&format!("DELETE /v1/jobs/{server_job_id} HTTP/1.1")));
        drop(ledger);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persisted_origin_cancellation_does_not_require_a_current_connector_lease() {
        let root = temp_dir("current-cancel");
        let database = root.join("jobs.sqlite3");
        let source = root.join("source.wav");
        let owned_live = root.join("live-recordings");
        let remote_jobs = root.join("remote-jobs");
        fs::create_dir_all(&owned_live).unwrap();
        write_pcm_wav(&source, &vec![0_u8; 320]);
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&queued_job("job-current-cancel", source.clone()))
            .unwrap();
        let owner = OwnerNamespace::local("i-drain-test").unwrap();
        prepare_next_queued_job(
            &ledger,
            &owned_live,
            &remote_jobs,
            &owner,
            1_720_000_000_100,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap();
        let prepared = ledger
            .get_prepared_remote_job("job-current-cancel")
            .unwrap()
            .unwrap();
        let request =
            CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json).unwrap();
        let server_job_id = "job-0123456789abcdef0123456789abcdef";
        let response = serde_json::json!({
            "jobId": server_job_id,
            "sessionId": request.metadata.session_id.as_str(),
            "displayName": request.display_name,
            "sessionMode": "meeting",
            "sessionOrigin": "imported_file",
            "status": "cancelled",
            "route": "server_batch",
            "captureManifest": request.capture_manifest,
            "createdAtUtc": "2026-07-14T21:00:00Z",
            "updatedAtUtc": "2026-07-14T21:00:01Z"
        });
        let (base_url, observed, server) = start_json_server(vec![(202, response)]);
        ledger
            .begin_remote_create_attempt("job-current-cancel", &base_url, 1_720_000_000_200)
            .unwrap();
        ledger
            .record_server_job_id(
                "job-current-cancel",
                server_job_id,
                &base_url,
                1_720_000_000_200,
            )
            .unwrap();
        ledger
            .request_cancellation("job-current-cancel", 1_720_000_000_300)
            .unwrap();
        let connector = ServerConnector::new();

        tauri::async_runtime::block_on(async {
            assert!(advance_persisted_cancellation_once(
                &ledger,
                &remote_jobs,
                &connector,
                1_720_000_000_400,
            )
            .await
            .unwrap());
        });
        server.join().unwrap();

        assert!(!remote_jobs.join("job-current-cancel").exists());
        assert!(source.is_file(), "external source must never be deleted");
        let acknowledged = ledger
            .get_prepared_remote_job("job-current-cancel")
            .unwrap()
            .unwrap();
        assert_eq!(
            acknowledged.server_cancellation_acknowledged_at_ms,
            Some(1_720_000_000_400)
        );
        assert!(observed.lock().unwrap()[0]
            .starts_with(&format!("DELETE /v1/jobs/{server_job_id} HTTP/1.1")));
        drop(ledger);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancelled_inflight_create_is_recovered_before_its_local_tombstone_is_acknowledged() {
        let root = temp_dir("cancelled-create-attempt");
        let database = root.join("jobs.sqlite3");
        let source = root.join("source.wav");
        let owned_live = root.join("live-recordings");
        let remote_jobs = root.join("remote-jobs");
        fs::create_dir_all(&owned_live).unwrap();
        write_pcm_wav(&source, &vec![0_u8; 320]);
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&queued_job("job-cancelled-create", source.clone()))
            .unwrap();
        let owner = OwnerNamespace::local("i-drain-test").unwrap();
        prepare_next_queued_job(
            &ledger,
            &owned_live,
            &remote_jobs,
            &owner,
            1_720_000_000_100,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap();
        let prepared = ledger
            .get_prepared_remote_job("job-cancelled-create")
            .unwrap()
            .unwrap();
        let request =
            CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json).unwrap();
        let server_job_id = "job-0123456789abcdef0123456789abcdef";
        let projection = |status: &str| {
            serde_json::json!({
                "jobId": server_job_id,
                "sessionId": request.metadata.session_id.as_str(),
                "displayName": request.display_name,
                "sessionMode": "meeting",
                "sessionOrigin": "imported_file",
                "status": status,
                "route": "server_batch",
                "captureManifest": request.capture_manifest,
                "createdAtUtc": "2026-07-14T21:00:00Z",
                "updatedAtUtc": "2026-07-14T21:00:01Z"
            })
        };
        let (base_url, observed, server) = start_json_server(vec![
            (202, projection("accepted")),
            (202, projection("cancelled")),
        ]);
        ledger
            .begin_remote_create_attempt("job-cancelled-create", &base_url, 1_720_000_000_200)
            .unwrap();
        ledger
            .request_cancellation("job-cancelled-create", 1_720_000_000_201)
            .unwrap();
        let pending_probe = RemoteJobDrain {
            ledger: JobLedger::open(&database).unwrap(),
            owner_namespace: OwnerNamespace::local("i-pending-probe").unwrap(),
            owned_live_directory: owned_live.clone(),
            remote_jobs_directory: remote_jobs.clone(),
        };
        assert!(pending_probe.has_pending_work().unwrap());
        drop(pending_probe);
        let connector = ServerConnector::new();

        tauri::async_runtime::block_on(async {
            assert!(advance_persisted_cancellation_once(
                &ledger,
                &remote_jobs,
                &connector,
                1_720_000_000_300,
            )
            .await
            .unwrap());
        });
        server.join().unwrap();

        let acknowledged = ledger
            .get_prepared_remote_job("job-cancelled-create")
            .unwrap()
            .unwrap();
        assert_eq!(acknowledged.server_job_id.as_deref(), Some(server_job_id));
        assert_eq!(
            acknowledged.server_cancellation_acknowledged_at_ms,
            Some(1_720_000_000_300)
        );
        assert!(!remote_jobs.join("job-cancelled-create").exists());
        assert!(source.is_file(), "external source must never be deleted");
        let requests = observed.lock().unwrap();
        assert!(requests[0].starts_with("POST /v1/jobs HTTP/1.1"));
        assert!(requests[1].starts_with(&format!("DELETE /v1/jobs/{server_job_id} HTTP/1.1")));

        drop(requests);
        drop(ledger);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn abandoned_create_attempt_is_recovered_and_cancelled_at_its_persisted_origin() {
        let root = temp_dir("abandoned-create-attempt");
        let database = root.join("jobs.sqlite3");
        let source = root.join("source.wav");
        let owned_live = root.join("live-recordings");
        let remote_jobs = root.join("remote-jobs");
        fs::create_dir_all(&owned_live).unwrap();
        write_pcm_wav(&source, &vec![0_u8; 320]);
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&queued_job("job-abandoned-create", source.clone()))
            .unwrap();
        let owner = OwnerNamespace::local("i-drain-test").unwrap();
        prepare_next_queued_job(
            &ledger,
            &owned_live,
            &remote_jobs,
            &owner,
            1_720_000_000_100,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap();
        let prepared = ledger
            .get_prepared_remote_job("job-abandoned-create")
            .unwrap()
            .unwrap();
        let request =
            CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json).unwrap();
        let server_job_id = "job-0123456789abcdef0123456789abcdef";
        let projection = |status: &str| {
            serde_json::json!({
                "jobId": server_job_id,
                "sessionId": request.metadata.session_id.as_str(),
                "displayName": request.display_name,
                "sessionMode": "meeting",
                "sessionOrigin": "imported_file",
                "status": status,
                "route": "server_batch",
                "captureManifest": request.capture_manifest,
                "createdAtUtc": "2026-07-14T21:00:00Z",
                "updatedAtUtc": "2026-07-14T21:00:01Z"
            })
        };
        let (base_url, observed, server) = start_json_server(vec![
            (202, projection("accepted")),
            (202, projection("cancelled")),
        ]);
        ledger
            .begin_remote_create_attempt("job-abandoned-create", &base_url, 1_720_000_000_200)
            .unwrap();
        let boundary = ServerConnectorBoundary::new();
        boundary.configure(&ServerSettings::default());
        let connector = boundary.downgrade().upgrade().unwrap();

        tauri::async_runtime::block_on(async {
            assert!(advance_persisted_cancellation_once(
                &ledger,
                &remote_jobs,
                &connector,
                1_720_000_000_300,
            )
            .await
            .unwrap());
        });
        server.join().unwrap();

        let failed = ledger.get_job("job-abandoned-create").unwrap().unwrap();
        assert_eq!(failed.status, RecordingJobStatus::Failed);
        assert_eq!(failed.error_code.as_deref(), Some("REMOTE_ORIGIN_CHANGED"));
        assert!(ledger
            .get_prepared_remote_job("job-abandoned-create")
            .unwrap()
            .is_none());
        assert!(!remote_jobs.join("job-abandoned-create").exists());
        assert!(source.is_file(), "external source must never be deleted");
        let requests = observed.lock().unwrap();
        assert!(requests[0].starts_with("POST /v1/jobs HTTP/1.1"));
        assert!(requests[1].starts_with(&format!("DELETE /v1/jobs/{server_job_id} HTTP/1.1")));

        drop(requests);
        drop(ledger);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_origin_detaches_and_cancels_an_existing_server_binding() {
        let root = temp_dir("changed-origin-binding");
        let database = root.join("jobs.sqlite3");
        let source = root.join("source.wav");
        let owned_live = root.join("live-recordings");
        let remote_jobs = root.join("remote-jobs");
        fs::create_dir_all(&owned_live).unwrap();
        write_pcm_wav(&source, &vec![0_u8; 320]);
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&queued_job("job-changed-origin", source.clone()))
            .unwrap();
        let owner = OwnerNamespace::local("i-drain-test").unwrap();
        prepare_next_queued_job(
            &ledger,
            &owned_live,
            &remote_jobs,
            &owner,
            1_720_000_000_100,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap();
        let prepared = ledger
            .get_prepared_remote_job("job-changed-origin")
            .unwrap()
            .unwrap();
        let request =
            CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json).unwrap();
        let server_job_id = "job-0123456789abcdef0123456789abcdef";
        let cancelled = serde_json::json!({
            "jobId": server_job_id,
            "sessionId": request.metadata.session_id.as_str(),
            "displayName": request.display_name,
            "sessionMode": "meeting",
            "sessionOrigin": "imported_file",
            "status": "cancelled",
            "route": "server_batch",
            "captureManifest": request.capture_manifest,
            "createdAtUtc": "2026-07-14T21:00:00Z",
            "updatedAtUtc": "2026-07-14T21:00:01Z"
        });
        let (old_origin, observed, server) = start_json_server(vec![(202, cancelled)]);
        ledger
            .begin_remote_create_attempt("job-changed-origin", &old_origin, 1_720_000_000_200)
            .unwrap();
        ledger
            .record_server_job_id(
                "job-changed-origin",
                server_job_id,
                &old_origin,
                1_720_000_000_201,
            )
            .unwrap();
        let boundary = ServerConnectorBoundary::new();
        boundary.configure(&ServerSettings {
            enabled: true,
            base_url: Some("http://127.0.0.1:9".into()),
            ..ServerSettings::default()
        });
        let connector = boundary.downgrade().upgrade().unwrap();

        tauri::async_runtime::block_on(async {
            assert!(advance_persisted_cancellation_once(
                &ledger,
                &remote_jobs,
                &connector,
                1_720_000_000_300,
            )
            .await
            .unwrap());
            assert!(advance_persisted_cancellation_once(
                &ledger,
                &remote_jobs,
                &connector,
                1_720_000_000_400,
            )
            .await
            .unwrap());
        });
        server.join().unwrap();

        let failed = ledger.get_job("job-changed-origin").unwrap().unwrap();
        assert_eq!(failed.status, RecordingJobStatus::Failed);
        assert_eq!(failed.error_code.as_deref(), Some("REMOTE_ORIGIN_CHANGED"));
        assert!(ledger
            .get_prepared_remote_job("job-changed-origin")
            .unwrap()
            .is_none());
        assert!(ledger
            .list_detached_remote_cancellations()
            .unwrap()
            .is_empty());
        assert!(!remote_jobs.join("job-changed-origin").exists());
        assert!(source.is_file(), "external source must never be deleted");
        assert!(observed.lock().unwrap()[0]
            .starts_with(&format!("DELETE /v1/jobs/{server_job_id} HTTP/1.1")));

        drop(ledger);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completed_server_result_is_published_before_the_ledger_becomes_complete() {
        let root = temp_dir("result");
        let database = root.join("jobs.sqlite3");
        let source = root.join("source.wav");
        let owned_live = root.join("live-recordings");
        let remote_jobs = root.join("remote-jobs");
        fs::create_dir_all(&owned_live).unwrap();
        write_pcm_wav(&source, &vec![0_u8; 320]);
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&queued_job("job-drain-result", source))
            .unwrap();
        let owner = OwnerNamespace::local("i-drain-test").unwrap();
        prepare_next_queued_job(
            &ledger,
            &owned_live,
            &remote_jobs,
            &owner,
            1_720_000_000_100,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap();
        let prepared = ledger
            .get_prepared_remote_job("job-drain-result")
            .unwrap()
            .unwrap();
        let request =
            CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json).unwrap();
        let server_job_id = "job-0123456789abcdef0123456789abcdef";
        let projection = serde_json::json!({
            "jobId": server_job_id,
            "sessionId": request.metadata.session_id.as_str(),
            "displayName": request.display_name,
            "sessionMode": "meeting",
            "sessionOrigin": "imported_file",
            "status": "complete",
            "route": "server_batch",
            "captureManifest": request.capture_manifest,
            "createdAtUtc": "2026-07-14T21:00:00Z",
            "updatedAtUtc": "2026-07-14T21:00:02Z"
        });
        let result = serde_json::json!({
            "sessionId": request.metadata.session_id.as_str(),
            "revision": 1,
            "authority": "server_authoritative",
            "createdAtUtc": "2026-07-14T21:00:02Z",
            "captureManifestSha256": request.capture_manifest.sha256,
            "previousResultSha256": null,
            "status": "complete",
            "language": {
                "languageBcp47": "en-US",
                "confidence": null
            },
            "transcript": "Phase five is connected.",
            "alignedWords": [],
            "modelProvenance": [{
                "modelId": "CohereLabs/cohere-transcribe-03-2026",
                "revision": "b1eacc2686a3d08ceaae5f24a88b1d519620bc09",
                "calibrationRevision": "asr-not-applicable"
            }]
        });
        let valid_result: crate::server_connector::batch::TranscriptResultRevision =
            serde_json::from_value(result.clone()).unwrap();
        let mut empty_result = valid_result.clone();
        empty_result.transcript = " \n\t".into();
        assert!(validate_result_revision(&empty_result, &request).is_err());
        let mut offset_timestamp = valid_result;
        offset_timestamp.created_at_utc = "2026-07-14T16:00:02-05:00".into();
        assert!(validate_result_revision(&offset_timestamp, &request).is_err());
        let (base_url, observed, server) =
            start_json_server(vec![(200, projection), (200, result)]);
        ledger
            .begin_remote_create_attempt("job-drain-result", &base_url, 1_720_000_000_200)
            .unwrap();
        ledger
            .record_server_job_id(
                "job-drain-result",
                server_job_id,
                &base_url,
                1_720_000_000_200,
            )
            .unwrap();
        for chunk in &request.chunks {
            ledger
                .acknowledge_remote_chunk(
                    "job-drain-result",
                    &chunk.replay_key.track_id,
                    chunk.replay_key.sequence_start,
                    chunk.replay_key.sequence_end,
                    &chunk.content_identity.sha256,
                    1_720_000_000_300,
                )
                .unwrap();
        }
        ledger
            .mark_remote_job_committed("job-drain-result", 1_720_000_000_400)
            .unwrap();
        let client = BatchApiClient::new(
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            &base_url,
        )
        .unwrap();

        tauri::async_runtime::block_on(async {
            assert!(super::advance_processing_once(
                &ledger,
                &remote_jobs,
                &client,
                1_720_000_000_500,
            )
            .await
            .unwrap());
        });
        server.join().unwrap();

        let completed = ledger.get_job("job-drain-result").unwrap().unwrap();
        assert_eq!(completed.status, RecordingJobStatus::Complete);
        assert_eq!(completed.expires_at_ms, Some(1_722_592_000_000));
        let output = completed.output_path.unwrap();
        assert_eq!(
            fs::read_to_string(&output).unwrap(),
            "Phase five is connected.\n"
        );
        let result_path = output.parent().unwrap().join("result.json");
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(result_path).unwrap()).unwrap();
        assert_eq!(
            persisted["captureManifestSha256"],
            request.capture_manifest.sha256
        );
        let requests = observed.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with(&format!("GET /v1/jobs/{server_job_id} HTTP/1.1")));
        assert!(requests[1].starts_with(&format!("GET /v1/jobs/{server_job_id}/result HTTP/1.1")));
        drop(requests);
        drop(ledger);
        fs::remove_dir_all(root).unwrap();
    }

    fn queued_job(job_id: &str, source: std::path::PathBuf) -> NewRecordingJob {
        NewRecordingJob {
            job_id: job_id.into(),
            session_mode: SessionMode::Meeting,
            session_origin: SessionOrigin::ImportedFile,
            source_path: Some(source),
            source_ownership: SourceOwnership::External,
            output_path: None,
            display_name: "source.wav".into(),
            status: RecordingJobStatus::QueuedServer,
            route: Some(RecordingRoute::ServerBatch),
            attempt_count: 0,
            next_attempt_at_ms: None,
            cancellation_requested: false,
            capture_commit_path: None,
            capture_manifest_sha256: None,
            error_code: None,
            error_message: None,
            created_at_ms: 1_720_000_000_000,
            updated_at_ms: 1_720_000_000_000,
            expires_at_ms: Some(1_720_604_800_000),
        }
    }

    fn start_json_server(
        responses: Vec<(u16, serde_json::Value)>,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let server_observed = Arc::clone(&observed);
        let server = thread::spawn(move || {
            for (status, response) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                let expected = loop {
                    let read = stream.read(&mut buffer).unwrap();
                    assert_ne!(read, 0, "request ended before headers");
                    request.extend_from_slice(&buffer[..read]);
                    if let Some(split) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..split]);
                        let content_length = head
                            .lines()
                            .find_map(|line| {
                                line.split_once(':').and_then(|(name, value)| {
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().unwrap())
                                })
                            })
                            .unwrap_or(0);
                        break split + 4 + content_length;
                    }
                };
                while request.len() < expected {
                    let read = stream.read(&mut buffer).unwrap();
                    assert_ne!(read, 0, "request body ended early");
                    request.extend_from_slice(&buffer[..read]);
                }
                server_observed
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request[..expected]).into_owned());
                let body = serde_json::to_vec(&response).unwrap();
                let reason = match status {
                    200 => "OK",
                    201 => "Created",
                    202 => "Accepted",
                    _ => "Error",
                };
                write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
                stream.flush().unwrap();
            }
        });
        (format!("http://{address}"), observed, server)
    }

    fn write_pcm_wav(path: &std::path::Path, pcm: &[u8]) {
        let mut file = File::create(path).unwrap();
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36_u32 + pcm.len() as u32).to_le_bytes())
            .unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&16_u32.to_le_bytes()).unwrap();
        file.write_all(&1_u16.to_le_bytes()).unwrap();
        file.write_all(&1_u16.to_le_bytes()).unwrap();
        file.write_all(&16_000_u32.to_le_bytes()).unwrap();
        file.write_all(&32_000_u32.to_le_bytes()).unwrap();
        file.write_all(&2_u16.to_le_bytes()).unwrap();
        file.write_all(&16_u16.to_le_bytes()).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&(pcm.len() as u32).to_le_bytes()).unwrap();
        file.write_all(pcm).unwrap();
        file.sync_all().unwrap();
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yap-phase5-drain-{label}-{}-{nonce}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }
}
