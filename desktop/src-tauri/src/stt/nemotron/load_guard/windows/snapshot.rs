use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use super::path_lease::{
    map_model_path_error, metadata_is_link_or_reparse, model_file_identity, open_regular_source,
    open_snapshot_for_identity, DirectoryLease, ModelFileIdentity, FILE_SHARE_READ_VALUE,
};
use crate::stt::error::SttError;
use crate::stt::nemotron::Artifact;

const SNAPSHOT_PREFIX: &str = ".yap-model-load-";
static NEXT_SNAPSHOT: AtomicU64 = AtomicU64::new(0);

/// A fresh file held from creation with write/delete sharing denied to other handles.
///
/// The retained handle intentionally remains writable by this process: closing and reopening it
/// read-only would introduce a replacement window. Native readers receive only `path`.
pub(super) struct SnapshotArtifact {
    path: PathBuf,
    file: File,
    identity: ModelFileIdentity,
    expected_bytes: u64,
}

impl SnapshotArtifact {
    pub(super) fn create(
        source_path: &Path,
        snapshot_path: &Path,
        artifact: &Artifact,
    ) -> Result<Self, SttError> {
        let mut source = open_regular_source(source_path)?;
        let mut snapshot = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(FILE_SHARE_READ_VALUE)
            .open(snapshot_path)
            .map_err(|_| SttError::ModelCorrupt)?;
        let mut hasher = Sha256::new();
        let mut copied = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = source
                .read(&mut buffer)
                .map_err(|_| SttError::ModelCorrupt)?;
            if read == 0 {
                break;
            }
            copied = copied
                .checked_add(read as u64)
                .ok_or(SttError::ModelCorrupt)?;
            if copied > artifact.bytes {
                return Err(SttError::ModelCorrupt);
            }
            snapshot
                .write_all(&buffer[..read])
                .map_err(|_| SttError::ModelCorrupt)?;
            hasher.update(&buffer[..read]);
        }
        if copied != artifact.bytes
            || !digest_matches(hasher.finalize().as_slice(), artifact.sha256)
        {
            return Err(SttError::ModelCorrupt);
        }
        snapshot.sync_all().map_err(|_| SttError::ModelCorrupt)?;
        snapshot
            .seek(SeekFrom::Start(0))
            .map_err(|_| SttError::ModelCorrupt)?;
        let metadata = snapshot.metadata().map_err(|_| SttError::ModelCorrupt)?;
        if !metadata.is_file()
            || metadata_is_link_or_reparse(&metadata)
            || metadata.len() != artifact.bytes
        {
            return Err(SttError::ModelCorrupt);
        }
        let identity = model_file_identity(&snapshot)?;
        Ok(Self {
            path: snapshot_path.to_path_buf(),
            file: snapshot,
            identity,
            expected_bytes: artifact.bytes,
        })
    }

    pub(super) fn revalidate(&self) -> Result<(), SttError> {
        let metadata = self.file.metadata().map_err(|_| SttError::ModelCorrupt)?;
        if !metadata.is_file()
            || metadata_is_link_or_reparse(&metadata)
            || metadata.len() != self.expected_bytes
            || model_file_identity(&self.file)? != self.identity
        {
            return Err(SttError::ModelCorrupt);
        }
        let current = open_snapshot_for_identity(&self.path)?;
        if model_file_identity(&current)? != self.identity {
            return Err(SttError::ModelCorrupt);
        }
        Ok(())
    }
}

pub(super) fn create_snapshot_root(root: &Path) -> Result<(PathBuf, DirectoryLease), SttError> {
    for _ in 0..1024 {
        let sequence = NEXT_SNAPSHOT.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!(
            "{SNAPSHOT_PREFIX}{}-{sequence}",
            std::process::id()
        ));
        match std::fs::create_dir(&path) {
            Ok(()) => match DirectoryLease::open(&path) {
                Ok(lease) => return Ok((path, lease)),
                Err(error) => {
                    let _ = std::fs::remove_dir(&path);
                    return Err(error);
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(SttError::ModelCorrupt),
        }
    }
    Err(SttError::ModelCorrupt)
}

pub(super) fn cleanup_stale_snapshots(root: &Path) -> Result<(), SttError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(SttError::ModelCorrupt),
    };
    for entry in entries {
        let entry = entry.map_err(|_| SttError::ModelCorrupt)?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(SNAPSHOT_PREFIX) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(map_model_path_error)?;
        if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
            return Err(SttError::ModelCorrupt);
        }
        std::fs::remove_dir_all(entry.path()).map_err(|_| SttError::ModelCorrupt)?;
    }
    Ok(())
}

fn digest_matches(actual: &[u8], expected: &str) -> bool {
    actual.len() * 2 == expected.len()
        && actual
            .iter()
            .zip(expected.as_bytes().chunks_exact(2))
            .all(|(byte, expected_pair)| {
                let high = hex_value(expected_pair[0]);
                let low = hex_value(expected_pair[1]);
                high.zip(low)
                    .is_some_and(|(high, low)| *byte == (high << 4) | low)
            })
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
