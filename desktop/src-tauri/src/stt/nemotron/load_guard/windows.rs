use std::path::{Path, PathBuf};

use super::super::{Artifact, NemotronPaths};
use crate::stt::error::SttError;

mod path_lease;
mod snapshot;

use path_lease::{absolute_model_root, open_directory_chain, DirectoryLease};
use snapshot::{create_snapshot_root, SnapshotArtifact};

pub(super) struct WindowsModelLoadGuard {
    paths: NemotronPaths,
    snapshot_root: PathBuf,
    artifacts: Vec<SnapshotArtifact>,
    directories: Vec<DirectoryLease>,
}

impl WindowsModelLoadGuard {
    pub(super) fn open(root: &Path, artifacts: &[Artifact]) -> Result<Self, SttError> {
        let root = absolute_model_root(root)?;
        let mut directories = open_directory_chain(&root)?;
        cleanup_stale_snapshots(&root)?;
        let (snapshot_root, snapshot_directory) = create_snapshot_root(&root)?;
        directories.push(snapshot_directory);
        let mut snapshots = Vec::with_capacity(artifacts.len());

        let built = (|| -> Result<NemotronPaths, SttError> {
            for artifact in artifacts {
                snapshots.push(SnapshotArtifact::create(
                    &root.join(artifact.file),
                    &snapshot_root.join(artifact.file),
                    artifact,
                )?);
            }
            super::super::paths_at(snapshot_root.clone())
        })();

        match built {
            Ok(paths) => {
                let guard = Self {
                    paths,
                    snapshot_root,
                    artifacts: snapshots,
                    directories,
                };
                guard.revalidate_after_native_load()?;
                Ok(guard)
            }
            Err(error) => {
                drop(snapshots);
                drop(directories);
                let _ = std::fs::remove_dir_all(&snapshot_root);
                Err(error)
            }
        }
    }

    pub(super) fn paths(&self) -> &NemotronPaths {
        &self.paths
    }

    pub(super) fn revalidate_after_native_load(&self) -> Result<(), SttError> {
        for directory in &self.directories {
            directory.revalidate()?;
        }
        for artifact in &self.artifacts {
            artifact.revalidate()?;
        }
        Ok(())
    }
}

impl Drop for WindowsModelLoadGuard {
    fn drop(&mut self) {
        self.artifacts.clear();
        self.directories.clear();
        if let Err(error) = std::fs::remove_dir_all(&self.snapshot_root) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::stt::log_yap(&format!(
                    "local model load snapshot cleanup failed: {error}"
                ));
            }
        }
    }
}

pub(super) fn cleanup_stale_snapshots(root: &Path) -> Result<(), SttError> {
    snapshot::cleanup_stale_snapshots(root)
}
