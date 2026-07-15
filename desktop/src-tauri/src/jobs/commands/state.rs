use super::RecordingJobs;
use crate::jobs::{JobLedger, RecordingJobResources};
#[cfg(test)]
use std::collections::VecDeque;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

impl RecordingJobs {
    pub(crate) fn from_default_resources(resources: Arc<RecordingJobResources>) -> Self {
        Self::from_storage(
            resources,
            crate::file_actions::recording_job_playback_registry_path(),
            crate::file_actions::recording_job_selection_registry_path(),
        )
    }

    fn from_storage(
        resources: Arc<RecordingJobResources>,
        registry_path: PathBuf,
        selection_registry_path: PathBuf,
    ) -> Self {
        Self {
            resources,
            playback: Mutex::new(HashMap::new()),
            #[cfg(test)]
            projection_failures: Mutex::new(VecDeque::new()),
            registry_path,
            selection_registry_path,
        }
    }

    pub(super) fn ledger(&self) -> &JobLedger {
        self.resources.ledger()
    }

    pub(super) fn mutation(&self) -> &Mutex<()> {
        self.resources.mutation()
    }

    pub(super) fn owned_dir(&self) -> &Path {
        self.resources.owned_live_directory()
    }

    pub(super) fn remote_jobs_directory(&self) -> &Path {
        self.resources.remote_jobs_directory()
    }

    pub(super) fn reset_remote_spool(&self, job_id: &str) -> Result<(), String> {
        self.resources.reset_remote_spool(job_id)
    }

    #[cfg(test)]
    pub(in crate::jobs) fn from_ledger(ledger: JobLedger, authority_dir: &Path) -> Self {
        let owned_dir = authority_dir.join("owned-live-recordings");
        std::fs::create_dir_all(&owned_dir).expect("prepare test owned directory");
        let resources = Arc::new(RecordingJobResources::from_storage(
            ledger,
            owned_dir,
            authority_dir.join("remote-jobs"),
        ));
        Self::from_resources_for_test(resources, authority_dir)
    }

    #[cfg(test)]
    pub(in crate::jobs) fn from_resources_for_test(
        resources: Arc<RecordingJobResources>,
        authority_dir: &Path,
    ) -> Self {
        Self::from_storage(
            resources,
            authority_dir.join("recording-job-playback-registry.json"),
            authority_dir.join("recording-native-selection-registry.json"),
        )
    }

    #[cfg(test)]
    pub(in crate::jobs) fn resources_for_test(&self) -> &Arc<RecordingJobResources> {
        &self.resources
    }
}
