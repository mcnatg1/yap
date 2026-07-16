use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read},
    path::Path,
};

pub(crate) fn read_text(path: &Path, maximum_bytes: usize) -> io::Result<String> {
    String::from_utf8(read_bytes(path, maximum_bytes)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(crate) fn read_bytes(path: &Path, maximum_bytes: usize) -> io::Result<Vec<u8>> {
    let path_metadata = fs::symlink_metadata(path)?;
    validate_metadata(&path_metadata, maximum_bytes)?;
    let mut file = open_no_follow(path)?;
    let opened_metadata = file.metadata()?;
    validate_metadata(&opened_metadata, maximum_bytes)?;
    if opened_metadata.len() != path_metadata.len()
        || !same_file_identity(&path_metadata, &opened_metadata)
    {
        return Err(invalid_data("opened file differs from its admitted path"));
    }
    let expected_bytes = usize::try_from(opened_metadata.len())
        .map_err(|_| invalid_data("file length is out of range"))?;
    let bytes = read_to_end(&mut file, maximum_bytes)?;
    if bytes.len() != expected_bytes {
        return Err(invalid_data("file changed while it was read"));
    }
    Ok(bytes)
}

pub(crate) fn read_to_string(reader: &mut impl Read, maximum_bytes: usize) -> io::Result<String> {
    String::from_utf8(read_to_end(reader, maximum_bytes)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(crate) fn read_to_end(reader: &mut impl Read, maximum_bytes: usize) -> io::Result<Vec<u8>> {
    let read_limit = u64::try_from(maximum_bytes)
        .ok()
        .and_then(|limit| limit.checked_add(1))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "read limit is out of range"))?;
    let mut bytes = Vec::with_capacity(maximum_bytes.min(64 * 1024));
    reader.take(read_limit).read_to_end(&mut bytes)?;
    if bytes.len() > maximum_bytes {
        return Err(invalid_data("file exceeded its read limit"));
    }
    Ok(bytes)
}

fn validate_metadata(metadata: &Metadata, maximum_bytes: usize) -> io::Result<()> {
    if !metadata.is_file()
        || metadata_is_link_or_reparse(metadata)
        || metadata.len() > maximum_bytes as u64
    {
        return Err(invalid_data("path is not a bounded regular file"));
    }
    Ok(())
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
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

#[cfg(not(any(unix, windows)))]
fn open_no_follow(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no-follow reads are unsupported on this platform",
    ))
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(unix)]
fn same_file_identity(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    (left.dev(), left.ino()) == (right.dev(), right.ino())
}

#[cfg(windows)]
fn same_file_identity(_left: &Metadata, _right: &Metadata) -> bool {
    // The Windows opener grants only FILE_SHARE_READ, so the path cannot be
    // replaced or mutated while the returned handle remains open. The opened
    // handle is still revalidated for type, reparse status, length, and bytes.
    true
}

#[cfg(not(any(unix, windows)))]
fn same_file_identity(_left: &Metadata, _right: &Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{read_text, read_to_end};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn stream_reader_rejects_content_past_the_limit() {
        let mut source = std::io::Cursor::new(vec![0_u8; 9]);

        let error = read_to_end(&mut source, 8).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn path_reader_accepts_exact_bound_and_rejects_oversized_content() {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("yap-bounded-file-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.txt");
        std::fs::write(&path, "12345678").unwrap();
        assert_eq!(read_text(&path, 8).unwrap(), "12345678");

        std::fs::write(&path, "123456789").unwrap();
        let error = read_text(&path, 8).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        std::fs::remove_dir_all(dir).unwrap();
    }
}
