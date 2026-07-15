use super::{
    command_error, log_registry_cleanup_failure, renewed_expiry, JobCommandError, RecordingJobs,
    RetryKind,
};
use crate::{
    commands::media_protocol::MediaOwner,
    jobs::{RecordingJobStatus, RecordingJobView},
};
use std::collections::HashSet;

impl RecordingJobs {
    pub(super) fn snapshot(
        &self,
        media: &MediaOwner,
        now_ms: u64,
    ) -> Result<Vec<RecordingJobView>, JobCommandError> {
        let _mutation = self.mutation().lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording job state is unavailable.",
            )
        })?;
        self.ledger().expire_pending_jobs(now_ms)?;
        let (expired_remote_job_ids, _) = self.ledger().enforce_remote_retention(now_ms)?;
        for job_id in expired_remote_job_ids {
            self.remove_remote_spool_best_effort(&job_id, "retention");
        }
        let mut views = Vec::new();
        let mut recoverable_ids = HashSet::new();
        let mut authorized_paths = Vec::new();
        let mut recoverable_paths = Vec::new();
        for record in self.ledger().list_recoverable_jobs()? {
            recoverable_ids.insert(record.job_id.clone());
            if let Some(source_path) = record.source_path.clone() {
                recoverable_paths.push(source_path);
            }
            match self.project_with_playback(record.clone(), media) {
                Ok(view) => {
                    if view.playback_path.is_some() {
                        if let Some(source_path) = record.source_path.clone() {
                            authorized_paths.push(source_path);
                        }
                    }
                    views.push(view);
                }
                Err(error) if error.code == "SOURCE_MISSING" || error.code == "SOURCE_UNSAFE" => {
                    let failed = self.ledger().fail_source_validation(
                        &record.job_id,
                        &error.code,
                        now_ms,
                    )?;
                    views.push(self.project_failed_capability_free(&failed, media));
                }
                Err(error) => return Err(error),
            }
        }
        self.reconcile_playback(&recoverable_ids, media)?;
        if let Err(error) = crate::file_actions::reconcile_recording_job_playback_paths_at(
            &authorized_paths,
            &self.registry_path,
        ) {
            log_registry_cleanup_failure("snapshot reconciliation", &error);
        }
        if let Err(error) = crate::file_actions::reconcile_recording_job_playback_paths_at(
            &recoverable_paths,
            &self.selection_registry_path,
        ) {
            log_registry_cleanup_failure("native selection reconciliation", &error);
        }
        Ok(views)
    }

    pub(super) fn cancel(
        &self,
        media: &MediaOwner,
        job_id: &str,
        now_ms: u64,
        notify: impl FnOnce(),
    ) -> Result<RecordingJobView, JobCommandError> {
        let mutation = self.mutation().lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording job state is unavailable.",
            )
        })?;
        let record = self.ledger().request_cancellation(job_id, now_ms)?;
        self.release_playback(job_id, media);
        self.remove_all_job_authority_best_effort(record.source_path.as_deref(), "cancellation");
        self.remove_remote_spool_best_effort(job_id, "cancellation");
        let view = RecordingJobView::from_record(&record);
        drop(mutation);
        notify();
        Ok(view)
    }

    pub(super) fn retry(
        &self,
        media: &MediaOwner,
        job_id: &str,
        now_ms: u64,
        notify: impl FnOnce(),
    ) -> Result<RecordingJobView, JobCommandError> {
        let mutation = self.mutation().lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording job state is unavailable.",
            )
        })?;
        let current = self.ledger().get_job(job_id)?.ok_or_else(|| {
            command_error(
                "JOB_NOT_FOUND",
                format!("Recording job {job_id:?} was not found."),
            )
        })?;
        let retry_kind = match current.status {
            RecordingJobStatus::Accepted => RetryKind::Accepted,
            RecordingJobStatus::BlockedSetupRequired
            | RecordingJobStatus::BlockedServerUnavailable
            | RecordingJobStatus::BlockedSignInRequired
            | RecordingJobStatus::Failed => RetryKind::Retry,
            RecordingJobStatus::QueuedServer => RetryKind::Unchanged,
            _ => {
                return Err(command_error(
                    "INVALID_JOB_TRANSITION",
                    format!("Recording job {job_id:?} cannot be retried from its current state."),
                ));
            }
        };
        let removes_prior_remote_spool = matches!(&retry_kind, RetryKind::Retry);
        let source = current.source_path.as_deref().ok_or_else(|| {
            command_error("SOURCE_UNSAFE", "Imported recording has no source path.")
        })?;
        let source = self.validate_source(source)?;

        let (record, changed) = match retry_kind {
            RetryKind::Accepted => (
                self.ledger()
                    .accept_to_queued_server(job_id, now_ms, renewed_expiry(now_ms)?)?,
                true,
            ),
            RetryKind::Retry => (
                self.ledger().retry_to_queued_server(
                    job_id,
                    now_ms,
                    Some(renewed_expiry(now_ms)?),
                )?,
                true,
            ),
            RetryKind::Unchanged => (current, false),
        };
        if removes_prior_remote_spool {
            self.remove_remote_spool_best_effort(job_id, "retry");
        }
        let view = self.project_committed_or_fail(record, source, media, now_ms)?;
        drop(mutation);
        if changed {
            notify();
        }
        Ok(view)
    }

    pub(super) fn dismiss(
        &self,
        media: &MediaOwner,
        job_id: &str,
        now_ms: u64,
        notify: impl FnOnce(),
    ) -> Result<RecordingJobView, JobCommandError> {
        let mutation = self.mutation().lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording job state is unavailable.",
            )
        })?;
        let record = self.ledger().dismiss_failed(job_id, now_ms)?;
        self.release_playback(job_id, media);
        self.remove_all_job_authority_best_effort(record.source_path.as_deref(), "dismissal");
        self.remove_remote_spool_best_effort(job_id, "dismissal");
        let view = RecordingJobView::from_record(&record);
        drop(mutation);
        notify();
        Ok(view)
    }
}
