use std::sync::Arc;

use crate::{
    audio::session::OwnerNamespace,
    jobs::{RecordingJobResources, RecordingJobStatus},
};

use super::error::{remote_retry_plan, DrainStepError};

pub(crate) struct RemoteJobDrain {
    pub(super) resources: Arc<RecordingJobResources>,
    pub(super) owner_namespace: OwnerNamespace,
}

impl RemoteJobDrain {
    pub(crate) fn from_resources(resources: Arc<RecordingJobResources>) -> Result<Self, String> {
        Ok(Self {
            resources,
            owner_namespace: crate::install_identity::load_or_create()?,
        })
    }

    #[cfg(test)]
    pub(in crate::jobs) fn from_resources_for_test(
        resources: Arc<RecordingJobResources>,
        owner_namespace: OwnerNamespace,
    ) -> Self {
        Self {
            resources,
            owner_namespace,
        }
    }

    #[cfg(test)]
    pub(in crate::jobs) fn resources_for_test(&self) -> &Arc<RecordingJobResources> {
        &self.resources
    }

    pub(super) fn has_pending_work(&self) -> Result<bool, String> {
        let active_job = self
            .resources
            .ledger()
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
                .resources
                .ledger()
                .has_remote_reconciliation_work()
                .map_err(|error| error.to_string())?)
    }

    pub(super) fn enforce_retention(&self, now_ms: u64) -> Result<bool, String> {
        let _mutation = self
            .resources
            .mutation()
            .lock()
            .map_err(|_| "recording job mutation gate is unavailable".to_string())?;
        let expired_pending = self
            .resources
            .ledger()
            .expire_pending_jobs(now_ms)
            .map_err(|error| error.to_string())?;
        let (expired_remote_job_ids, changed_remote_jobs) = self
            .resources
            .ledger()
            .enforce_remote_retention(now_ms)
            .map_err(|error| error.to_string())?;
        let mut cleanup_error = None;
        for job_id in expired_remote_job_ids {
            if let Err(error) = self.resources.reset_remote_spool(&job_id) {
                cleanup_error.get_or_insert(error);
            }
        }
        if let Some(error) = cleanup_error {
            return Err(error);
        }
        let mut pruned_spools = 0_usize;
        for job_id in self
            .resources
            .ledger()
            .list_pending_remote_spool_cleanup()
            .map_err(|error| error.to_string())?
        {
            self.resources.reset_remote_spool(&job_id)?;
            if self
                .resources
                .ledger()
                .acknowledge_remote_spool_cleanup(&job_id)
                .map_err(|error| error.to_string())?
            {
                pruned_spools = pruned_spools.saturating_add(1);
            }
        }
        Ok(expired_pending > 0 || changed_remote_jobs > 0 || pruned_spools > 0)
    }

    pub(super) fn fail_preprocessing_candidate(&self, updated_at_ms: u64) {
        let candidate = self
            .resources
            .ledger()
            .list_recoverable_jobs()
            .ok()
            .and_then(|jobs| {
                jobs.into_iter()
                    .find(|job| job.status == RecordingJobStatus::Preprocessing)
            });
        let Some(candidate) = candidate else {
            return;
        };
        let _ = self.resources.ledger().record_remote_error(
            &candidate.job_id,
            "PREPROCESSING_FAILED",
            "The selected recording could not be prepared for private-server transcription.",
            None,
            updated_at_ms,
        );
    }

    pub(super) fn schedule_remote_retry(
        &self,
        statuses: &[RecordingJobStatus],
        error: &DrainStepError,
        updated_at_ms: u64,
    ) {
        let candidate = self
            .resources
            .ledger()
            .list_recoverable_jobs()
            .ok()
            .and_then(|jobs| {
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
        let _ = self.resources.ledger().record_remote_error(
            &candidate.job_id,
            code,
            message,
            retry_at_ms,
            updated_at_ms,
        );
    }
}
