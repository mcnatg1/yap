use super::*;

pub(super) fn validate_recoverable_wav(
    file: &mut File,
    label: &str,
) -> Result<(u64, bool), String> {
    let length = file
        .metadata()
        .map_err(|error| format!("Failed to inspect {label}: {error}"))?
        .len();
    if length < WAV_HEADER_BYTES {
        return Err(format!("{label} is shorter than a WAV header"));
    }
    let mut header = [0_u8; WAV_HEADER_BYTES as usize];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut header))
        .map_err(|error| format!("Failed to read {label} header: {error}"))?;
    let read_u16 = |offset: usize| u16::from_le_bytes([header[offset], header[offset + 1]]);
    let read_u32 = |offset: usize| {
        u32::from_le_bytes([
            header[offset],
            header[offset + 1],
            header[offset + 2],
            header[offset + 3],
        ])
    };
    if &header[0..4] != b"RIFF"
        || &header[8..12] != b"WAVE"
        || &header[12..16] != b"fmt "
        || read_u32(16) != 16
        || read_u16(20) != 1
        || read_u16(22) != 1
        || read_u32(24) != 16_000
        || read_u32(28) != 32_000
        || read_u16(32) != 2
        || read_u16(34) != 16
        || &header[36..40] != b"data"
    {
        return Err(format!(
            "{label} is not Yap PCM mono 16 kHz 16-bit WAV audio"
        ));
    }
    let data_bytes = length - WAV_HEADER_BYTES;
    if !data_bytes.is_multiple_of(PCM16_BYTES_PER_SAMPLE) {
        return Err(format!("{label} has an unaligned PCM data length"));
    }
    let riff_bytes = u64::from(read_u32(4));
    let declared_data_bytes = u64::from(read_u32(40));
    let placeholder = riff_bytes == 36 && declared_data_bytes == 0;
    let finalized = riff_bytes == 36 + data_bytes && declared_data_bytes == data_bytes;
    if !placeholder && !finalized {
        return Err(format!(
            "{label} header lengths do not match its opened file length"
        ));
    }
    Ok((data_bytes, placeholder))
}

pub(crate) fn sha256_regular_artifact(directory: &Path, name: &str) -> Result<String, String> {
    let mut file = open_regular_artifact(directory, name)?;
    sha256_open_file(&mut file)
}

pub(super) fn open_regular_path(path: &Path) -> Result<File, String> {
    let directory = path
        .parent()
        .ok_or_else(|| "recording artifact has no parent directory".to_string())?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "recording artifact has no valid file name".to_string())?;
    open_regular_artifact(directory, name)
}

pub(super) fn open_regular_artifact_for_update(
    directory: &Path,
    name: &str,
) -> Result<File, String> {
    validate_artifact_name(name)?;
    let file = open_no_follow_update(&directory.join(name))
        .map_err(|error| format!("Failed to open recording artifact for update: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Failed to inspect recording artifact: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("recording artifact is not a regular file".into());
    }
    #[cfg(windows)]
    if metadata.file_attributes() & 0x400 != 0 {
        return Err("recording artifact is a reparse point".into());
    }
    Ok(file)
}

#[cfg(any(unix, windows))]
pub(super) fn same_file_identity(left: &File, right: &File) -> Result<bool, String> {
    Ok(file_identity(left)? == file_identity(right)?)
}

#[cfg(not(any(unix, windows)))]
pub(super) fn same_file_identity(_left: &File, _right: &File) -> Result<bool, String> {
    Err("recording publication ownership is unsupported on this platform".into())
}

#[cfg(unix)]
pub(super) fn file_identity(file: &File) -> Result<FileIdentity, String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file
        .metadata()
        .map_err(|error| format!("Failed to inspect recording file identity: {error}"))?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(unix)]
pub(super) fn file_link_count(file: &File) -> Result<u64, String> {
    use std::os::unix::fs::MetadataExt;

    file.metadata()
        .map(|metadata| metadata.nlink())
        .map_err(|error| format!("Failed to inspect recording file link count: {error}"))
}

#[cfg(windows)]
pub(super) fn file_link_count(file: &File) -> Result<u64, String> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            &mut information as *mut BY_HANDLE_FILE_INFORMATION,
        )
    }
    .map_err(|error| format!("Failed to inspect recording file link count: {error}"))?;
    Ok(u64::from(information.nNumberOfLinks))
}

#[cfg(not(any(unix, windows)))]
pub(super) fn file_link_count(_file: &File) -> Result<u64, String> {
    Err("recording link ownership is unsupported on this platform".into())
}

#[cfg(windows)]
pub(super) fn file_identity(file: &File) -> Result<FileIdentity, String> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            &mut information as *mut BY_HANDLE_FILE_INFORMATION,
        )
    }
    .map_err(|error| format!("Failed to inspect recording file identity: {error}"))?;
    Ok(FileIdentity {
        volume_serial: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

#[cfg(not(any(unix, windows)))]
pub(super) fn file_identity(_file: &File) -> Result<FileIdentity, String> {
    Err("recording publication ownership is unsupported on this platform".into())
}

pub(crate) fn read_regular_artifact(directory: &Path, name: &str) -> Result<String, String> {
    let mut file = open_regular_artifact(directory, name)?;
    read_open_file(&mut file)
}

pub(crate) fn read_and_hash_regular_artifact(
    directory: &Path,
    name: &str,
) -> Result<(String, String), String> {
    let mut file = open_regular_artifact(directory, name)?;
    let text = read_open_file(&mut file)?;
    let hash = Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((text, hash))
}

pub(super) fn read_open_file(file: &mut File) -> Result<String, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("Failed to read recording artifact: {error}"))?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|error| format!("Failed to read recording artifact: {error}"))?;
    Ok(text)
}

#[cfg(unix)]
pub(super) fn open_no_follow(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(unix)]
fn open_no_follow_update(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(windows)]
fn open_no_follow_update(path: &Path) -> std::io::Result<File> {
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_no_follow_update(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).open(path)
}

#[cfg(windows)]
pub(super) fn open_no_follow(path: &Path) -> std::io::Result<File> {
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
pub(super) fn open_no_follow(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

pub(super) fn session_from_private_artifact(name: &str) -> Option<SessionId> {
    let session = name.strip_prefix("live-")?;
    [
        ".wav.part",
        ".capture.journal.part",
        ".capture.json.part",
        ".capture.partial.json",
        ".capture.partial.json.part",
        ".commit.json.part",
    ]
    .into_iter()
    .find_map(|suffix| session.strip_suffix(suffix))
    .and_then(|session| SessionId::new(session).ok())
}

pub(super) fn session_from_orphan_wav_artifact(name: &str) -> Option<SessionId> {
    let session = name.strip_prefix("live-")?.strip_suffix(".wav")?;
    SessionId::new(session)
        .ok()
        .filter(SessionId::is_current_writer_id)
}

pub(super) fn has_owned_partial_lineage(directory: &Path, session_id: &SessionId) -> bool {
    let name = format!("live-{session_id}.capture.partial.json");
    let Ok(text) = read_regular_artifact(directory, &name) else {
        return false;
    };
    serde_json::from_str::<PartialCaptureSidecar>(&text)
        .map(|sidecar| {
            sidecar.schema_version == CAPTURE_SCHEMA_VERSION
                && sidecar.session_id == *session_id
                && sidecar.status == CaptureStatus::Partial
        })
        .unwrap_or(false)
}
