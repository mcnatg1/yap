use super::{
    command_error, log_registry_cleanup_failure, project_with_admission, source_error,
    CachedPlayback, JobCommandError, RecordingJobs,
};
use crate::{
    jobs::{RecordingJobStatus, RecordingJobView},
    media_protocol::MediaOwner,
    recording_access::{RecordingJobSourceAdmission, ValidatedRecordingJobSource},
};
use std::{collections::HashSet, path::Path};

impl RecordingJobs {
    pub(super) fn validate_source(
        &self,
        path: &Path,
    ) -> Result<ValidatedRecordingJobSource, JobCommandError> {
        crate::recording_access::validate_recording_job_source_at(path, self.owned_dir())
            .map_err(source_error)
    }

    pub(super) fn project_with_playback(
        &self,
        record: crate::jobs::RecordingJobRecord,
        media: &MediaOwner,
    ) -> Result<RecordingJobView, JobCommandError> {
        if record.status == RecordingJobStatus::Failed {
            return Ok(self.project_failed_capability_free(&record, media));
        }
        let Some(source) = record.source_path.as_deref() else {
            return Ok(RecordingJobView::from_record(&record));
        };
        let source = self.validate_source(source)?;
        self.project_validated(record, source, media)
    }

    pub(super) fn project_validated(
        &self,
        record: crate::jobs::RecordingJobRecord,
        source: ValidatedRecordingJobSource,
        media: &MediaOwner,
    ) -> Result<RecordingJobView, JobCommandError> {
        #[cfg(test)]
        if let Some(error) = self
            .projection_failures
            .lock()
            .expect("projection failure injection lock")
            .pop_front()
        {
            return Err(error);
        }
        let mut playback = self.playback.lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording playback state is unavailable.",
            )
        })?;
        if let Some(cached) = playback.get(&record.job_id) {
            if cached.source == source {
                let admission = RecordingJobSourceAdmission {
                    canonical_path: source.canonical_path,
                    playback_path: cached.playback_path.clone(),
                };
                return Ok(project_with_admission(record, admission));
            }
        }
        if let Some(stale) = playback.remove(&record.job_id) {
            media.release(&stale.playback_path);
        }
        let admission = crate::recording_access::authorize_registered_recording_job_source_at(
            &source,
            media,
            &self.selection_registry_path,
            &self.registry_path,
            self.owned_dir(),
        )
        .map_err(source_error)?;
        playback.insert(
            record.job_id.clone(),
            CachedPlayback {
                source,
                playback_path: admission.playback_path.clone(),
            },
        );
        Ok(project_with_admission(record, admission))
    }

    pub(super) fn project_committed_or_fail(
        &self,
        record: crate::jobs::RecordingJobRecord,
        source: ValidatedRecordingJobSource,
        media: &MediaOwner,
        now_ms: u64,
    ) -> Result<RecordingJobView, JobCommandError> {
        if record.status == RecordingJobStatus::Failed {
            return Ok(self.project_failed_capability_free(&record, media));
        }
        match self.project_validated(record.clone(), source, media) {
            Ok(view) => Ok(view),
            Err(error) => {
                let failed =
                    self.ledger()
                        .fail_source_validation(&record.job_id, &error.code, now_ms)?;
                Ok(self.project_failed_capability_free(&failed, media))
            }
        }
    }

    pub(super) fn project_failed_capability_free(
        &self,
        record: &crate::jobs::RecordingJobRecord,
        media: &MediaOwner,
    ) -> RecordingJobView {
        debug_assert_eq!(record.status, RecordingJobStatus::Failed);
        self.release_playback(&record.job_id, media);
        self.remove_active_job_authority_best_effort(
            record.source_path.as_deref(),
            "failed projection",
        );
        let mut view = RecordingJobView::from_record(record);
        view.source_path = None;
        view.playback_path = None;
        view
    }

    #[cfg(test)]
    pub(super) fn inject_projection_failures_for_test(&self, failures: Vec<JobCommandError>) {
        self.projection_failures
            .lock()
            .expect("projection failure injection lock")
            .extend(failures);
    }

    pub(super) fn release_playback(&self, job_id: &str, media: &MediaOwner) {
        let removed = self
            .playback
            .lock()
            .ok()
            .and_then(|mut playback| playback.remove(job_id));
        if let Some(removed) = removed {
            media.release(&removed.playback_path);
        }
    }

    pub(super) fn remove_active_job_authority_best_effort(
        &self,
        path: Option<&Path>,
        action: &str,
    ) {
        let Some(path) = path else {
            return;
        };
        if let Err(error) = crate::recording_access::remove_recording_job_playback_path_at(
            path,
            &self.registry_path,
        ) {
            log_registry_cleanup_failure(action, &error);
        }
    }

    pub(super) fn remove_all_job_authority_best_effort(&self, path: Option<&Path>, action: &str) {
        self.remove_active_job_authority_best_effort(path, action);
        let Some(path) = path else {
            return;
        };
        if let Err(error) = crate::recording_access::remove_recording_job_playback_path_at(
            path,
            &self.selection_registry_path,
        ) {
            log_registry_cleanup_failure(&format!("{action} native selection"), &error);
        }
    }

    pub(super) fn remove_remote_spool_best_effort(&self, job_id: &str, action: &str) {
        if let Err(error) = self.reset_remote_spool(job_id) {
            crate::stt::log_yap(&format!(
                "owned remote recording cleanup after {action} remains pending: {error}"
            ));
        }
    }

    pub(super) fn reconcile_playback(
        &self,
        recoverable_ids: &HashSet<String>,
        media: &MediaOwner,
    ) -> Result<(), JobCommandError> {
        let mut playback = self.playback.lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording playback state is unavailable.",
            )
        })?;
        let stale_ids = playback
            .keys()
            .filter(|job_id| !recoverable_ids.contains(*job_id))
            .cloned()
            .collect::<Vec<_>>();
        for job_id in stale_ids {
            if let Some(stale) = playback.remove(&job_id) {
                media.release(&stale.playback_path);
            }
        }
        Ok(())
    }
}
