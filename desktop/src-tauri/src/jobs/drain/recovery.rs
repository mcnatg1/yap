use std::path::Path;

use crate::{
    jobs::{remote, JobLedger, PreparedRemoteJobRecord},
    server_connector::{
        batch::{BatchApiClient, BatchClientError, CreateRecordingJobRequest, RecordingJob},
        ServerConnector,
    },
};

use super::{contract::validate_job_projection, BatchCommitGuard, DrainResult, DrainStepError};

pub(super) async fn advance_persisted_cancellation_once(
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
pub(super) async fn advance_cancellation_once(
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
