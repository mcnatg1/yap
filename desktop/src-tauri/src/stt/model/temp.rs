use std::{
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::stt::error::SttError;

use super::{io_error_to_stt, operation::DownloadOperation};

pub(super) struct OperationTemp {
    path: PathBuf,
    file: Option<std::fs::File>,
    operation: DownloadOperation,
    published: bool,
}

impl OperationTemp {
    pub(super) fn create(
        destination: &Path,
        operation: DownloadOperation,
    ) -> Result<Self, SttError> {
        let (path, file) = reserve_operation_temp_file(destination, operation.generation())?;
        Ok(Self {
            path,
            file: Some(file),
            operation,
            published: false,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn file_mut(&mut self) -> Result<&mut std::fs::File, SttError> {
        self.file.as_mut().ok_or(SttError::ModelMissing)
    }

    pub(super) fn sync(&mut self) -> Result<(), SttError> {
        self.file_mut()?.sync_all().map_err(io_error_to_stt)
    }

    pub(super) fn publish_to(&mut self, destination: &Path) -> Result<(), SttError> {
        self.file.take();
        atomic_replace_same_directory(&self.path, destination)?;
        self.published = true;
        sync_parent_directory(destination).map_err(io_error_to_stt)
    }

    fn cleanup(&mut self) -> Result<(), String> {
        self.file.take();
        if self.published {
            return Ok(());
        }
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("{}: {error}", self.path.display())),
        }
    }
}

impl Drop for OperationTemp {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            self.operation.record_cleanup_failure(error);
        }
    }
}

pub(crate) fn cleanup_stale_download_temps(
    destination: &Path,
    operation: &DownloadOperation,
) -> Result<(), SttError> {
    try_cleanup_stale_download_temps(destination).map_err(|message| {
        operation.record_cleanup_failure(message);
        SttError::ModelMissing
    })
}

fn try_cleanup_stale_download_temps(destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", destination.display()))?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has no file name", destination.display()))?;
    let legacy_temp = destination.with_extension("part");
    let entries = std::fs::read_dir(parent)
        .map_err(|error| format!("could not inspect {}: {error}", parent.display()))?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "could not inspect an entry in {}: {error}",
                parent.display()
            )
        })?;
        let candidate = entry.path();
        let candidate_name = entry.file_name();
        let candidate_name = candidate_name.to_string_lossy();
        let is_legacy_temp = candidate == legacy_temp && candidate != destination;
        if !is_legacy_temp && !is_download_temp_name(file_name, &candidate_name) {
            continue;
        }

        let metadata = std::fs::symlink_metadata(&candidate)
            .map_err(|error| format!("could not inspect {}: {error}", candidate.display()))?;
        if metadata.is_dir() {
            return Err(format!(
                "refusing to remove directory {}",
                candidate.display()
            ));
        }
        std::fs::remove_file(&candidate)
            .map_err(|error| format!("could not remove {}: {error}", candidate.display()))?;
    }
    Ok(())
}

fn is_download_temp_name(file_name: &str, candidate_name: &str) -> bool {
    let Some(candidate_name) = candidate_name.strip_suffix(".part") else {
        return false;
    };
    let operation_prefix = format!("{file_name}.op-");
    if let Some(components) = candidate_name.strip_prefix(&operation_prefix) {
        return has_numeric_components(components, '-', 4);
    }
    let legacy_prefix = format!("{file_name}.");
    candidate_name
        .strip_prefix(&legacy_prefix)
        .is_some_and(|components| has_numeric_components(components, '.', 3))
}

fn has_numeric_components(value: &str, separator: char, expected: usize) -> bool {
    let mut components = value.split(separator);
    (0..expected).all(|_| {
        components
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    }) && components.next().is_none()
}

fn reserve_operation_temp_file(
    destination: &Path,
    generation: u64,
) -> Result<(PathBuf, std::fs::File), SttError> {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(SttError::ModelMissing)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SttError::ModelMissing)?
        .as_nanos();
    let pid = std::process::id();
    for attempt in 0..32 {
        let path = destination.with_file_name(format!(
            "{file_name}.op-{generation}-{pid}-{nonce}-{attempt}.part"
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error_to_stt(error)),
        }
    }
    Err(SttError::ModelMissing)
}

pub(crate) fn write_text_atomically(path: &Path, text: &str) -> Result<(), SttError> {
    let (temp, mut file) = reserve_sibling_temp_file(path)?;
    let result = (|| {
        file.write_all(text.as_bytes()).map_err(io_error_to_stt)?;
        file.sync_all().map_err(io_error_to_stt)?;
        drop(file);
        atomic_replace_same_directory(&temp, path)?;
        sync_parent_directory(path).map_err(io_error_to_stt)
    })();
    if result.is_err() {
        match std::fs::remove_file(&temp) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(io_error_to_stt(error)),
        }
    }
    result
}

fn reserve_sibling_temp_file(path: &Path) -> Result<(PathBuf, std::fs::File), SttError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(SttError::ModelMissing)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SttError::ModelMissing)?
        .as_nanos();
    let pid = std::process::id();
    for attempt in 0..32 {
        let temp = path.with_file_name(format!("{file_name}.{pid}.{nonce}.{attempt}.part"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error_to_stt(error)),
        }
    }
    Err(SttError::ModelMissing)
}

#[cfg(windows)]
fn atomic_replace_same_directory(source: &Path, destination: &Path) -> Result<(), SttError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_WRITE_THROUGH,
    };

    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let source = PCWSTR(source_wide.as_ptr());
    let destination = PCWSTR(destination_wide.as_ptr());

    let result = unsafe {
        if destination_path_exists(destination_wide.as_slice()) {
            ReplaceFileW(
                destination,
                source,
                PCWSTR::null(),
                REPLACEFILE_WRITE_THROUGH,
                None,
                None,
            )
        } else {
            MoveFileExW(
                source,
                destination,
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    result.map_err(|_| SttError::ModelMissing)
}

#[cfg(windows)]
fn destination_path_exists(wide_path: &[u16]) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};
    unsafe { GetFileAttributesW(PCWSTR(wide_path.as_ptr())) != INVALID_FILE_ATTRIBUTES }
}

#[cfg(not(windows))]
fn atomic_replace_same_directory(source: &Path, destination: &Path) -> Result<(), SttError> {
    std::fs::rename(source, destination).map_err(io_error_to_stt)
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path.parent().ok_or_else(|| {
        std::io::Error::new(ErrorKind::InvalidInput, "path has no parent directory")
    })?)?
    .sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
