use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_STAGING_DIRECTORY: AtomicU64 = AtomicU64::new(0);

pub(super) fn next_staging_nonce() -> u64 {
    NEXT_STAGING_DIRECTORY.fetch_add(1, Ordering::Relaxed)
}

#[cfg(windows)]
pub(super) fn open_no_follow_read(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(target_os = "linux")]
pub(super) fn open_no_follow_read(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    const O_NOFOLLOW: i32 = 0x0002_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)
}

#[cfg(target_os = "macos")]
pub(super) fn open_no_follow_read(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    const O_NOFOLLOW: i32 = 0x0000_0100;
    OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(super) fn open_no_follow_read(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "secure no-follow chunk open is unsupported on this platform",
    ))
}

#[cfg(windows)]
pub(super) fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub(super) fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub(super) struct StagingDirectory {
    pub(super) path: PathBuf,
    published: bool,
}

impl StagingDirectory {
    pub(super) fn create(path: PathBuf) -> Result<Self, String> {
        fs::create_dir(&path)
            .map_err(|error| format!("failed to reserve job spool staging: {error}"))?;
        Ok(Self {
            path,
            published: false,
        })
    }

    pub(super) fn publish(&mut self, destination: &Path) -> Result<(), String> {
        fs::rename(&self.path, destination)
            .map_err(|error| format!("failed to publish prepared job spool: {error}"))?;
        self.published = true;
        Ok(())
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub(super) fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("failed to create prepared job artifact: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to write prepared job artifact: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush prepared job artifact: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync prepared job artifact: {error}"))
}

pub(super) fn sha256_reader(file: &mut File, expected_bytes: u64) -> Result<String, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("failed to seek imported recording: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut remaining = expected_bytes;
    while remaining > 0 {
        let requested = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| "imported recording hash length is out of range")?;
        let read = file
            .read(&mut buffer[..requested])
            .map_err(|error| format!("failed to hash imported recording: {error}"))?;
        if read == 0 {
            return Err("imported recording changed while it was being hashed".into());
        }
        digest.update(&buffer[..read]);
        remaining -= read as u64;
    }
    let mut trailing = [0_u8; 1];
    if file
        .read(&mut trailing)
        .map_err(|error| format!("failed to verify imported recording length: {error}"))?
        != 0
    {
        return Err("imported recording changed while it was being hashed".into());
    }
    Ok(format_digest(digest.finalize().as_slice()))
}

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    format_digest(Sha256::digest(bytes).as_slice())
}

fn format_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn validate_identifier(value: &str, maximum: usize, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > maximum
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(format!("{label} is invalid"));
    }
    Ok(())
}

pub(super) fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
