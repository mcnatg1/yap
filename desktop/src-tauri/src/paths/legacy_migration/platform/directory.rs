use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use super::super::secure_tree::metadata_is_link_or_reparse;

pub(in crate::paths::legacy_migration) struct DirectoryLease {
    path: PathBuf,
    file: File,
    identity: FileIdentity,
}

impl DirectoryLease {
    pub(in crate::paths::legacy_migration) fn open(path: &Path) -> io::Result<Self> {
        let path_metadata = std::fs::symlink_metadata(path)?;
        if !path_metadata.is_dir() || metadata_is_link_or_reparse(&path_metadata) {
            return Err(invalid_directory(path));
        }
        let file = open_directory_no_follow(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
            return Err(invalid_directory(path));
        }
        let identity = file_identity(&file)?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            identity,
        })
    }

    pub(in crate::paths::legacy_migration) fn sorted_entry_names(
        &self,
    ) -> io::Result<Vec<std::ffi::OsString>> {
        self.revalidate()?;
        let mut names = std::fs::read_dir(&self.path)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<Result<Vec<_>, _>>()?;
        names.sort();
        self.revalidate()?;
        Ok(names)
    }

    fn revalidate(&self) -> io::Result<()> {
        if file_identity(&self.file)? != self.identity {
            return Err(invalid_directory(&self.path));
        }
        let current = open_directory_no_follow(&self.path)?;
        let metadata = current.metadata()?;
        if !metadata.is_dir()
            || metadata_is_link_or_reparse(&metadata)
            || file_identity(&current)? != self.identity
        {
            return Err(invalid_directory(&self.path));
        }
        Ok(())
    }
}

fn invalid_directory(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "migration tree contains an unstable or linked directory: {}",
            path.display()
        ),
    )
}

#[cfg(windows)]
fn open_directory_no_follow(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(unix)]
fn open_directory_no_follow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(any(windows, unix)))]
fn open_directory_no_follow(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "secure migration directory opens are unsupported on this platform",
    ))
}

#[cfg(windows)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    volume_serial: u32,
    file_index: u64,
}

#[cfg(windows)]
fn file_identity(file: &File) -> io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            &mut information as *mut BY_HANDLE_FILE_INFORMATION,
        )
    }
    .map_err(|_| io::Error::last_os_error())?;
    Ok(FileIdentity {
        volume_serial: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn file_identity(file: &File) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(any(windows, unix)))]
#[derive(Clone, Copy, PartialEq, Eq)]
struct FileIdentity;

#[cfg(not(any(windows, unix)))]
fn file_identity(_file: &File) -> io::Result<FileIdentity> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "migration file identity is unsupported on this platform",
    ))
}
