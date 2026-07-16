use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    path::Path,
};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MediaSourceFingerprint {
    identity: FileIdentity,
    length: u64,
    revision: FileRevision,
}

pub(super) struct AuthorizedMediaSource {
    pub(super) file: File,
    pub(super) fingerprint: MediaSourceFingerprint,
    pub(super) mime: &'static str,
}

impl AuthorizedMediaSource {
    pub(super) fn byte_length(&self) -> u64 {
        self.fingerprint.length
    }

    pub(super) fn is_unchanged(&self) -> bool {
        file_snapshot(&self.file).is_ok_and(|snapshot| snapshot == self.fingerprint)
    }
}

pub(super) fn authorize_playback_source(
    path: &Path,
    expected: Option<&MediaSourceFingerprint>,
) -> Result<AuthorizedMediaSource, String> {
    if !path.is_absolute() {
        return Err("Recording playback requires an absolute path.".into());
    }
    let mime =
        media_mime(path).ok_or_else(|| "Choose a supported audio or video file.".to_string())?;
    let file = open_no_follow(path)
        .map_err(|error| format!("Failed to open recording for playback: {error}"))?;
    let fingerprint = file_snapshot(&file)?;
    if expected.is_some_and(|expected| expected != &fingerprint) {
        return Err("Recording source changed while playback was being authorized.".into());
    }
    Ok(AuthorizedMediaSource {
        file,
        fingerprint,
        mime,
    })
}

pub(crate) fn inspect_media_source(path: &Path) -> Result<MediaSourceFingerprint, String> {
    if !path.is_absolute() {
        return Err("Recording playback requires an absolute path.".into());
    }
    media_mime(path).ok_or_else(|| "Choose a supported audio or video file.".to_string())?;
    let file = open_no_follow(path)
        .map_err(|error| format!("Failed to open recording for playback: {error}"))?;
    file_snapshot(&file)
}

pub(crate) fn open_unchanged_media_source(
    path: &Path,
    expected: &MediaSourceFingerprint,
) -> Result<File, String> {
    if !path.is_absolute() {
        return Err("Recording preprocessing requires an absolute path.".into());
    }
    media_mime(path).ok_or_else(|| "Choose a supported audio or video file.".to_string())?;
    let file = open_no_follow(path)
        .map_err(|error| format!("Failed to open recording for preprocessing: {error}"))?;
    let snapshot = file_snapshot(&file)?;
    if &snapshot != expected {
        return Err("Recording source changed before preprocessing began.".into());
    }
    Ok(file)
}

pub(super) fn file_snapshot(file: &File) -> Result<MediaSourceFingerprint, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("Failed to inspect recording: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("Choose a supported audio or video file.".into());
    }
    #[cfg(windows)]
    if std::os::windows::fs::MetadataExt::file_attributes(&metadata) & FILE_ATTRIBUTE_REPARSE_POINT
        != 0
    {
        return Err("Recording playback rejects reparse points.".into());
    }
    Ok(MediaSourceFingerprint {
        identity: file_identity(file)?,
        length: metadata.len(),
        revision: file_revision(&metadata),
    })
}

pub(super) struct FileRangeReader<'a> {
    file: &'a File,
    position: u64,
}

impl<'a> FileRangeReader<'a> {
    pub(super) fn new(file: &'a File, position: u64) -> Self {
        Self { file, position }
    }
}

impl Read for FileRangeReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = read_file_at(self.file, buffer, self.position)?;
        self.position = self.position.saturating_add(read as u64);
        Ok(read)
    }
}

#[cfg(windows)]
fn read_file_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;

    file.seek_read(buffer, offset)
}

#[cfg(unix)]
fn read_file_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;

    file.read_at(buffer, offset)
}

#[cfg(not(any(windows, unix)))]
fn read_file_at(_file: &File, _buffer: &mut [u8], _offset: u64) -> io::Result<usize> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "positioned media reads are unsupported on this platform",
    ))
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileRevision {
    changed_nanoseconds: i64,
    changed_seconds: i64,
    modified_nanoseconds: i64,
    modified_seconds: i64,
}

#[cfg(unix)]
fn file_revision(metadata: &std::fs::Metadata) -> FileRevision {
    use std::os::unix::fs::MetadataExt;

    FileRevision {
        changed_nanoseconds: metadata.ctime_nsec(),
        changed_seconds: metadata.ctime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        modified_seconds: metadata.mtime(),
    }
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileRevision {
    last_write_time: u64,
}

#[cfg(windows)]
fn file_revision(metadata: &std::fs::Metadata) -> FileRevision {
    use std::os::windows::fs::MetadataExt;

    FileRevision {
        last_write_time: metadata.last_write_time(),
    }
}

#[cfg(not(any(unix, windows)))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileRevision;

#[cfg(not(any(unix, windows)))]
fn file_revision(_metadata: &std::fs::Metadata) -> FileRevision {
    FileRevision
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn file_identity(file: &File) -> Result<FileIdentity, String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file
        .metadata()
        .map_err(|error| format!("Failed to inspect recording file identity: {error}"))?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    file_index: u64,
    volume_serial: u32,
}

#[cfg(windows)]
fn file_identity(file: &File) -> Result<FileIdentity, String> {
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
    .map_err(|_| {
        format!(
            "Failed to inspect recording file identity: {}",
            io::Error::last_os_error()
        )
    })?;
    Ok(FileIdentity {
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
        volume_serial: information.dwVolumeSerialNumber,
    })
}

#[cfg(not(any(unix, windows)))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity;

#[cfg(not(any(unix, windows)))]
fn file_identity(_file: &File) -> Result<FileIdentity, String> {
    Err("Secure media file identity is unsupported on this platform.".into())
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(any(windows, unix)))]
fn open_no_follow(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "secure no-follow media open is unsupported on this platform",
    ))
}

fn media_mime(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("mp3") {
        Some("audio/mpeg")
    } else if extension.eq_ignore_ascii_case("m4a") {
        Some("audio/mp4")
    } else if extension.eq_ignore_ascii_case("wav") {
        Some("audio/wav")
    } else if extension.eq_ignore_ascii_case("mp4") {
        Some("video/mp4")
    } else if extension.eq_ignore_ascii_case("flac") {
        Some("audio/flac")
    } else if extension.eq_ignore_ascii_case("ogg") {
        Some("audio/ogg")
    } else if extension.eq_ignore_ascii_case("webm") {
        Some("video/webm")
    } else {
        None
    }
}
