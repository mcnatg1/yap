use std::time::{Duration, SystemTime};

use tauri::{Emitter, Manager};

use crate::{jobs::RecordingJobStatus, server_connector::ServerConnector};

use super::{
    preparation::prepare_next_queued_job, processing::advance_processing_with_lease,
    recovery::advance_persisted_cancellation_once, upload::advance_upload_with_lease,
    RemoteJobDrain,
};

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
        let now = now_ms();
        let drain = app.state::<RemoteJobDrain>();
        match advance_persisted_cancellation_once(
            drain.resources.ledger(),
            drain.resources.remote_jobs_directory(),
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
            connector.refresh_for_job_drain(&app).await;
        }
        if connector.batch_connection_lease().ok().flatten().is_none() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        let prepare_app = app.clone();
        let prepared = tauri::async_runtime::spawn_blocking(move || {
            let drain = prepare_app.state::<RemoteJobDrain>();
            prepare_next_queued_job(
                drain.resources.ledger(),
                drain.resources.owned_live_directory(),
                drain.resources.remote_jobs_directory(),
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
            drain.resources.ledger(),
            drain.resources.remote_jobs_directory(),
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
            drain.resources.ledger(),
            drain.resources.remote_jobs_directory(),
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
