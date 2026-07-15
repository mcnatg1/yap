use std::fs::{File, OpenOptions};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};

use crate::stt::error::SttError;

pub(super) const FILE_SHARE_READ_VALUE: u32 = 0x0000_0001;
pub(super) const FILE_SHARE_WRITE_VALUE: u32 = 0x0000_0002;
const FILE_FLAG_BACKUP_SEMANTICS_VALUE: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT_VALUE: u32 = 0x0020_0000;

/// Denies deletion/rename of every ordinary path component without blocking child writes.
pub(super) struct DirectoryLease {
    path: PathBuf,
    file: File,
    identity: ModelFileIdentity,
}

impl DirectoryLease {
    pub(super) fn open(path: &Path) -> Result<Self, SttError> {
        let path_metadata = std::fs::symlink_metadata(path).map_err(map_model_path_error)?;
        if !path_metadata.is_dir() || metadata_is_link_or_reparse(&path_metadata) {
            return Err(SttError::ModelCorrupt);
        }
        let file = open_directory_no_reparse(path).map_err(map_model_path_error)?;
        let metadata = file.metadata().map_err(|_| SttError::ModelCorrupt)?;
        if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
            return Err(SttError::ModelCorrupt);
        }
        let identity = model_file_identity(&file)?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            identity,
        })
    }

    pub(super) fn revalidate(&self) -> Result<(), SttError> {
        if model_file_identity(&self.file)? != self.identity {
            return Err(SttError::ModelCorrupt);
        }
        let current = open_directory_no_reparse(&self.path).map_err(map_model_path_error)?;
        let metadata = current.metadata().map_err(|_| SttError::ModelCorrupt)?;
        if !metadata.is_dir()
            || metadata_is_link_or_reparse(&metadata)
            || model_file_identity(&current)? != self.identity
        {
            return Err(SttError::ModelCorrupt);
        }
        Ok(())
    }
}

pub(super) fn absolute_model_root(root: &Path) -> Result<PathBuf, SttError> {
    if root
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(SttError::ModelCorrupt);
    }
    if root.is_absolute() {
        Ok(root.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(root))
            .map_err(|_| SttError::ModelCorrupt)
    }
}

pub(super) fn open_directory_chain(root: &Path) -> Result<Vec<DirectoryLease>, SttError> {
    let mut paths = root
        .ancestors()
        .filter(|path| !path.as_os_str().is_empty())
        .collect::<Vec<_>>();
    paths.reverse();
    paths.into_iter().map(DirectoryLease::open).collect()
}

pub(super) fn open_regular_source(path: &Path) -> Result<File, SttError> {
    let path_metadata = std::fs::symlink_metadata(path).map_err(map_model_path_error)?;
    if !path_metadata.is_file() || metadata_is_link_or_reparse(&path_metadata) {
        return Err(SttError::ModelCorrupt);
    }
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ_VALUE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT_VALUE)
        .open(path)
        .map_err(map_model_path_error)?;
    let metadata = file.metadata().map_err(|_| SttError::ModelCorrupt)?;
    if !metadata.is_file() || metadata_is_link_or_reparse(&metadata) {
        return Err(SttError::ModelCorrupt);
    }
    Ok(file)
}

pub(super) fn open_snapshot_for_identity(path: &Path) -> Result<File, SttError> {
    let metadata = std::fs::symlink_metadata(path).map_err(map_model_path_error)?;
    if !metadata.is_file() || metadata_is_link_or_reparse(&metadata) {
        return Err(SttError::ModelCorrupt);
    }
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ_VALUE | FILE_SHARE_WRITE_VALUE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT_VALUE)
        .open(path)
        .map_err(map_model_path_error)
}

pub(super) fn metadata_is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
}

fn open_directory_no_reparse(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ_VALUE | FILE_SHARE_WRITE_VALUE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS_VALUE | FILE_FLAG_OPEN_REPARSE_POINT_VALUE)
        .open(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ModelFileIdentity {
    volume_serial: u32,
    file_index: u64,
}

pub(super) fn model_file_identity(file: &File) -> Result<ModelFileIdentity, SttError> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            &mut information as *mut BY_HANDLE_FILE_INFORMATION,
        )
    }
    .map_err(|_| SttError::ModelCorrupt)?;
    Ok(ModelFileIdentity {
        volume_serial: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

pub(super) fn map_model_path_error(error: std::io::Error) -> SttError {
    if error.kind() == std::io::ErrorKind::NotFound {
        SttError::ModelMissing
    } else {
        SttError::ModelCorrupt
    }
}
