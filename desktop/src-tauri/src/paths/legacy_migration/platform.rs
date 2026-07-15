use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

use super::secure_tree::metadata_is_link_or_reparse;

mod directory;

pub(super) use directory::DirectoryLease;

#[cfg(windows)]
pub(super) fn open_migration_lock(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    validate_opened_regular_file(path, &file, "legacy migration lock")?;
    Ok(file)
}

#[cfg(unix)]
pub(super) fn open_migration_lock(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    validate_opened_regular_file(path, &file, "legacy migration lock")?;
    Ok(file)
}

#[cfg(not(any(windows, unix)))]
pub(super) fn open_migration_lock(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "secure migration locking is unsupported on this platform",
    ))
}

#[cfg(windows)]
pub(super) fn open_regular_file_read(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    validate_opened_regular_file(path, &file, "migration file")?;
    Ok(file)
}

#[cfg(unix)]
pub(super) fn open_regular_file_read(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    validate_opened_regular_file(path, &file, "migration file")?;
    Ok(file)
}

#[cfg(not(any(windows, unix)))]
pub(super) fn open_regular_file_read(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "secure migration file reads are unsupported on this platform",
    ))
}

fn validate_opened_regular_file(path: &Path, file: &File, label: &str) -> io::Result<()> {
    let metadata = file.metadata()?;
    if metadata.is_file() && !metadata_is_link_or_reparse(&metadata) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{label} is not a normal file: {}", path.display()),
    ))
}

#[cfg(windows)]
pub(super) fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let source = wide(source);
    let destination = wide(destination);
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|_| io::Error::last_os_error())
}

#[cfg(target_os = "linux")]
pub(super) fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    let source = path_c_string(source, "source")?;
    let destination = path_c_string(destination, "destination")?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
pub(super) fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    let source = path_c_string(source, "source")?;
    let destination = path_c_string(destination, "destination")?;
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn path_c_string(path: &Path, label: &str) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;

    std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} path contains a NUL byte"),
        )
    })
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(super) fn rename_no_replace(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace migration publication is unsupported on this platform",
    ))
}
