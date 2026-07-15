use std::path::Path;

use crate::{
    jobs::{
        remote, JobChunkRecord, JobLedger, PreparedRemoteJobRecord, RecordingJobRecord,
        RecordingJobStatus,
    },
    server_connector::{
        batch::{
            BatchApiClient, CaptureChunkReference, CommitRecordingJobRequest,
            CreateRecordingJobRequest,
        },
        BatchConnectionLease, ServerConnector,
    },
};

use super::{contract::validate_job_projection, BatchCommitGuard, DrainResult, DrainStepError};

#[cfg(test)]
pub(super) async fn advance_upload_once(
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

pub(super) async fn advance_upload_with_lease(
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

pub(super) async fn advance_upload_once_guarded(
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

pub(super) fn validate_durable_upload_state(
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
