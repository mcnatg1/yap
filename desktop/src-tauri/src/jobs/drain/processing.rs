use std::path::Path;

use crate::{
    jobs::{remote, JobLedger, RecordingJobStatus},
    server_connector::{
        batch::{BatchApiClient, CreateRecordingJobRequest},
        BatchConnectionLease, ServerConnector,
    },
};

use super::{
    contract::{result_retention_expiry_ms, validate_job_projection, validate_result_revision},
    upload::validate_durable_upload_state,
    BatchCommitGuard, DrainResult, DrainStepError,
};

#[cfg(test)]
pub(super) async fn advance_processing_once(
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

pub(super) async fn advance_processing_with_lease(
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

pub(super) async fn advance_processing_once_guarded(
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
