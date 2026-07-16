use std::{path::Path, time::SystemTime};

use crate::{
    audio::session::OwnerNamespace,
    jobs::{remote, JobLedger, RecordingJobStatus},
};

pub(super) fn prepare_next_queued_job(
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
    let validated = crate::recording_access::validate_recording_job_source_at(
        source_path,
        owned_live_directory,
    )
    .map_err(|error| match error {
        crate::recording_access::RecordingJobSourceError::Missing => {
            "imported recording source is missing".to_string()
        }
        crate::recording_access::RecordingJobSourceError::Unsafe(message) => message,
    })?;
    let mut source = crate::media_protocol::open_unchanged_media_source(
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

pub(super) fn attach_prepared_remote_job_or_cleanup(
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
