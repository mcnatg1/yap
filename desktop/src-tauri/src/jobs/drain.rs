use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};
use tauri::{Emitter, Manager};
mod contract;
mod preparation;
mod processing;
mod recovery;
mod upload;

#[cfg(test)]
use contract::validate_result_revision;
#[cfg(test)]
use preparation::attach_prepared_remote_job_or_cleanup;
use preparation::prepare_next_queued_job;
use processing::advance_processing_with_lease;
#[cfg(test)]
use processing::{advance_processing_once, advance_processing_once_guarded};
#[cfg(test)]
use recovery::advance_cancellation_once;
use recovery::advance_persisted_cancellation_once;
use upload::advance_upload_with_lease;
#[cfg(test)]
use upload::{advance_upload_once, advance_upload_once_guarded};

use crate::{
    audio::session::OwnerNamespace,
    jobs::{remote, JobLedger, RecordingJobStatus},
    server_connector::batch::{ApiError, BatchClientError},
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

pub(crate) fn start(
    app: &tauri::AppHandle,
    lifecycle: &crate::runtime::DesktopLifecycle,
) -> std::io::Result<()> {
    let app = app.clone();
    lifecycle.spawn_async_task("remote-job-drain", async move {
        run(app).await;
    })
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

#[cfg(test)]
#[path = "drain/tests.rs"]
mod tests;
