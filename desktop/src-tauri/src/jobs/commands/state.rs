use super::{JobCommandError, RecordingJobs};
use crate::jobs::JobLedger;
#[cfg(test)]
use std::collections::VecDeque;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

impl RecordingJobs {
    pub fn open_default() -> Result<Self, JobCommandError> {
        Ok(Self::from_storage(
            JobLedger::open_default()?,
            crate::live::recordings::recordings_dir(),
            crate::paths::app_data_dir().join("remote-jobs"),
            crate::file_actions::recording_job_playback_registry_path(),
            crate::file_actions::recording_job_selection_registry_path(),
        ))
    }

    #[doc(hidden)]
    pub fn open(
        ledger_path: impl AsRef<Path>,
        owned_dir: impl Into<PathBuf>,
        registry_path: impl Into<PathBuf>,
    ) -> Result<Self, JobCommandError> {
        let registry_path = registry_path.into();
        let selection_registry_path = registry_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("recording-native-selection-registry.json");
        let remote_jobs_directory = registry_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("remote-jobs");
        Ok(Self::from_storage(
            JobLedger::open(ledger_path)?,
            owned_dir.into(),
            remote_jobs_directory,
            registry_path,
            selection_registry_path,
        ))
    }

    fn from_storage(
        ledger: JobLedger,
        owned_dir: PathBuf,
        remote_jobs_directory: PathBuf,
        registry_path: PathBuf,
        selection_registry_path: PathBuf,
    ) -> Self {
        Self {
            ledger,
            mutation: Mutex::new(()),
            playback: Mutex::new(HashMap::new()),
            #[cfg(test)]
            projection_failures: Mutex::new(VecDeque::new()),
            owned_dir,
            remote_jobs_directory,
            registry_path,
            selection_registry_path,
        }
    }

    #[cfg(test)]
    pub(super) fn from_ledger(ledger: JobLedger, authority_dir: &Path) -> Self {
        let owned_dir = authority_dir.join("owned-live-recordings");
        std::fs::create_dir_all(&owned_dir).expect("prepare test owned directory");
        Self::from_storage(
            ledger,
            owned_dir,
            authority_dir.join("remote-jobs"),
            authority_dir.join("recording-job-playback-registry.json"),
            authority_dir.join("recording-native-selection-registry.json"),
        )
    }
}
