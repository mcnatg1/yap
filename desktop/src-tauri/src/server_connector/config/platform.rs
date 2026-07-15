use std::path::Path;

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FileIdentity {
    volume_serial_number: u32,
    file_index: u64,
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(any(windows, unix)))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FileIdentity;

#[cfg(windows)]
pub(super) fn file_identity(file: &std::fs::File) -> std::io::Result<Option<FileIdentity>> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information) }
        .map_err(|_| std::io::Error::last_os_error())?;
    Ok(Some(FileIdentity {
        volume_serial_number: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    }))
}

#[cfg(unix)]
pub(super) fn file_identity(file: &std::fs::File) -> std::io::Result<Option<FileIdentity>> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    Ok(Some(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }))
}

#[cfg(not(any(windows, unix)))]
pub(super) fn file_identity(_file: &std::fs::File) -> std::io::Result<Option<FileIdentity>> {
    Ok(None)
}

pub(super) struct SettingsFileLock {
    file: std::fs::File,
}

pub(super) fn open_settings_lock(path: &Path) -> std::io::Result<SettingsFileLock> {
    let lock_path = path.with_extension("json.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock_file_exclusive(&file)?;
    Ok(SettingsFileLock { file })
}

impl Drop for SettingsFileLock {
    fn drop(&mut self) {
        unlock_file(&self.file).ok();
    }
}

#[cfg(windows)]
fn lock_file_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{LockFileEx, LOCKFILE_EXCLUSIVE_LOCK};
    use windows::Win32::System::IO::OVERLAPPED;

    let mut overlapped = OVERLAPPED::default();
    unsafe {
        LockFileEx(
            HANDLE(file.as_raw_handle()),
            LOCKFILE_EXCLUSIVE_LOCK,
            None,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    }
    .map_err(|_| std::io::Error::last_os_error())
}

#[cfg(windows)]
fn unlock_file(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::UnlockFileEx;
    use windows::Win32::System::IO::OVERLAPPED;

    let mut overlapped = OVERLAPPED::default();
    unsafe {
        UnlockFileEx(
            HANDLE(file.as_raw_handle()),
            None,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    }
    .map_err(|_| std::io::Error::last_os_error())
}

#[cfg(unix)]
fn lock_file_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn unlock_file(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(super) fn atomic_replace_same_directory(
    source: &Path,
    destination: &Path,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::MoveFileExW;

    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let source_wide = wide(source);
    let destination_wide = wide(destination);
    let result = unsafe {
        MoveFileExW(
            PCWSTR(source_wide.as_ptr()),
            PCWSTR(destination_wide.as_ptr()),
            windows_move_flags(),
        )
    };
    result.map_err(|_| std::io::Error::last_os_error())
}

#[cfg(windows)]
pub(super) fn windows_move_flags() -> windows::Win32::Storage::FileSystem::MOVE_FILE_FLAGS {
    use windows::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};

    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
}

#[cfg(not(windows))]
pub(super) fn atomic_replace_same_directory(
    source: &Path,
    destination: &Path,
) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(unix)]
pub(super) fn sync_parent_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?)?
    .sync_all()
}

#[cfg(not(unix))]
pub(super) fn sync_parent_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
