use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};

use crate::{
    audio::session::{OwnerNamespace, SessionId},
    paths,
};

const INSTALL_ID_FILE: &str = "install-id";
const MAX_INSTALL_ID_BYTES: usize = 64;

pub(crate) fn load_or_create() -> Result<OwnerNamespace, String> {
    load_or_create_at(&paths::app_data_dir())
}

pub(crate) fn load_or_create_at(directory: &Path) -> Result<OwnerNamespace, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("failed to create install identity directory: {error}"))?;
    let path = directory.join(INSTALL_ID_FILE);

    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            let generated = SessionId::generate()?;
            let install_id = format!("i-{}", &generated.as_str()[2..]);
            file.write_all(install_id.as_bytes())
                .map_err(|error| format!("failed to write install identity: {error}"))?;
            file.flush()
                .map_err(|error| format!("failed to flush install identity: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("failed to sync install identity: {error}"))?;
            OwnerNamespace::local(install_id)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_existing(&path),
        Err(error) => Err(format!("failed to create install identity: {error}")),
    }
}

fn install_identity_entry_must_be_regular() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "install identity storage entry must be a regular file",
    )
}

#[cfg(windows)]
const fn windows_install_identity_open_flags() -> u32 {
    0x0020_0000
}

#[cfg(windows)]
const fn windows_install_identity_attributes_are_regular(attributes: u32) -> bool {
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) == 0
}

#[cfg(windows)]
fn open_install_identity(path: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(windows_install_identity_open_flags())
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || !windows_install_identity_attributes_are_regular(metadata.file_attributes())
    {
        return Err(install_identity_entry_must_be_regular());
    }
    Ok(file)
}

#[cfg(unix)]
fn open_install_identity(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
            return Err(install_identity_entry_must_be_regular());
        }
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Err(install_identity_entry_must_be_regular());
    }
    Ok(file)
}

#[cfg(not(any(windows, unix)))]
fn open_install_identity(_path: &Path) -> std::io::Result<fs::File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "secure install identity open is unsupported on this platform",
    ))
}

fn read_existing(path: &Path) -> Result<OwnerNamespace, String> {
    let file = open_install_identity(path)
        .map_err(|error| format!("failed to open install identity: {error}"))?;
    if file
        .metadata()
        .map_err(|error| format!("failed to inspect install identity: {error}"))?
        .len()
        > MAX_INSTALL_ID_BYTES as u64
    {
        return Err("install identity is too large".into());
    }
    let mut install_id = String::new();
    file.take((MAX_INSTALL_ID_BYTES + 1) as u64)
        .read_to_string(&mut install_id)
        .map_err(|error| format!("failed to read install identity: {error}"))?;
    if install_id.len() > MAX_INSTALL_ID_BYTES {
        return Err("install identity is too large".into());
    }
    OwnerNamespace::local(&install_id)
        .map_err(|error| format!("invalid install identity at {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::load_or_create_at;
    use std::path::Path;

    #[cfg(unix)]
    fn create_file_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(source, destination)
    }

    #[cfg(windows)]
    fn create_file_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(source, destination)
    }

    fn test_symlink_is_unavailable(error: &std::io::Error) -> bool {
        cfg!(windows)
            && (error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(1314))
    }

    #[test]
    fn install_identity_is_stable_across_reopen_and_never_silently_rotates() {
        let directory =
            std::env::temp_dir().join(format!("yap-install-identity-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);

        let first = load_or_create_at(&directory).unwrap();
        let reopened = load_or_create_at(&directory).unwrap();
        assert_eq!(first, reopened);

        std::fs::write(directory.join("install-id"), "invalid/value").unwrap();
        assert!(load_or_create_at(&directory).is_err());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn oversized_install_identity_fails_closed_and_is_preserved() {
        let directory = std::env::temp_dir().join(format!(
            "yap-install-identity-oversized-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("install-id");
        let oversized = vec![b'a'; 65];
        std::fs::write(&path, &oversized).unwrap();

        let error = load_or_create_at(&directory).unwrap_err();

        assert!(error.contains("too large"), "{error}");
        assert_eq!(std::fs::read(path).unwrap(), oversized);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn install_identity_link_is_rejected_without_reading_its_target() {
        let directory = std::env::temp_dir().join(format!(
            "yap-install-identity-link-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let outside = directory.join("outside-install-id");
        let path = directory.join("install-id");
        std::fs::write(&outside, b"outside-id").unwrap();
        if let Err(error) = create_file_symlink(&outside, &path) {
            if test_symlink_is_unavailable(&error) {
                let _ = std::fs::remove_dir_all(directory);
                return;
            }
            panic!("could not create test symlink: {error}");
        }

        let error = load_or_create_at(&directory).unwrap_err();

        assert!(error.contains("regular file"), "{error}");
        assert_eq!(std::fs::read(outside).unwrap(), b"outside-id");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(windows)]
    #[test]
    fn install_identity_open_rejects_windows_reparse_entries() {
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

        assert_eq!(
            super::windows_install_identity_open_flags() & FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_FLAG_OPEN_REPARSE_POINT
        );
        assert!(super::windows_install_identity_attributes_are_regular(0));
        assert!(!super::windows_install_identity_attributes_are_regular(
            FILE_ATTRIBUTE_DIRECTORY
        ));
        assert!(!super::windows_install_identity_attributes_are_regular(
            FILE_ATTRIBUTE_REPARSE_POINT
        ));
    }
}
