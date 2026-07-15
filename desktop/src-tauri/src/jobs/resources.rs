use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use super::{remote, JobLedger, JobLedgerError};

/// Owns the durable ledger, retention/command gate, and filesystem roots shared by job actors.
pub(crate) struct RecordingJobResources {
    ledger: JobLedger,
    mutation: Mutex<()>,
    owned_live_directory: PathBuf,
    remote_jobs_directory: PathBuf,
}

impl RecordingJobResources {
    pub(crate) fn open_default() -> Result<Self, JobLedgerError> {
        Ok(Self::from_storage(
            JobLedger::open_default()?,
            crate::live::recordings::recordings_dir(),
            crate::paths::app_data_dir().join("remote-jobs"),
        ))
    }

    pub(in crate::jobs) fn from_storage(
        ledger: JobLedger,
        owned_live_directory: PathBuf,
        remote_jobs_directory: PathBuf,
    ) -> Self {
        Self {
            ledger,
            mutation: Mutex::new(()),
            owned_live_directory,
            remote_jobs_directory,
        }
    }

    pub(in crate::jobs) fn ledger(&self) -> &JobLedger {
        &self.ledger
    }

    pub(in crate::jobs) fn mutation(&self) -> &Mutex<()> {
        &self.mutation
    }

    pub(in crate::jobs) fn owned_live_directory(&self) -> &Path {
        &self.owned_live_directory
    }

    pub(in crate::jobs) fn remote_jobs_directory(&self) -> &Path {
        &self.remote_jobs_directory
    }

    pub(in crate::jobs) fn reset_remote_spool(&self, job_id: &str) -> Result<(), String> {
        remote::reset_unattached_spool(job_id, &self.remote_jobs_directory)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        audio::session::OwnerNamespace,
        jobs::{commands::RecordingJobs, RemoteJobDrain},
    };

    #[test]
    fn recording_command_and_drain_share_one_resource_owner() {
        let root =
            std::env::temp_dir().join(format!("yap-shared-job-resources-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        let resources = Arc::new(RecordingJobResources::from_storage(
            JobLedger::open_in_memory().unwrap(),
            root.join("recordings"),
            root.join("remote-jobs"),
        ));
        let commands = RecordingJobs::from_resources_for_test(Arc::clone(&resources), &root);
        let drain = RemoteJobDrain::from_resources_for_test(
            Arc::clone(&resources),
            OwnerNamespace::local("i-shared-job-resources").unwrap(),
        );

        assert!(Arc::ptr_eq(commands.resources_for_test(), &resources));
        assert!(Arc::ptr_eq(drain.resources_for_test(), &resources));
        assert!(std::ptr::eq(
            commands.resources_for_test().ledger(),
            drain.resources_for_test().ledger()
        ));
        let command_gate = commands.resources_for_test().mutation().lock().unwrap();
        assert!(drain.resources_for_test().mutation().try_lock().is_err());
        drop(command_gate);
        assert_eq!(
            commands.resources_for_test().owned_live_directory(),
            drain.resources_for_test().owned_live_directory()
        );
        assert_eq!(
            commands.resources_for_test().remote_jobs_directory(),
            drain.resources_for_test().remote_jobs_directory()
        );

        drop(commands);
        drop(drain);
        drop(resources);
        std::fs::remove_dir_all(root).unwrap();
    }
}
