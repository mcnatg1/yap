use std::io::Read;
use std::path::Path;

use super::platform::{file_identity, open_persisted_file, FileIdentity};

pub(super) const MAX_PERSISTED_CONFIG_BYTES: usize = 64 * 1024;

pub(super) struct PersistedFile {
    pub(super) identity: Option<FileIdentity>,
    pub(super) bytes: Vec<u8>,
}

pub(super) fn configuration_too_large() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "persisted server configuration is too large",
    )
}

pub(super) fn read_persisted_bytes(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    Ok(read_persisted_file(path)?.map(|persisted| persisted.bytes))
}

pub(super) fn read_persisted_text(path: &Path) -> std::io::Result<Option<String>> {
    read_persisted_bytes(path)?
        .map(|bytes| {
            String::from_utf8(bytes)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })
        .transpose()
}

pub(super) fn read_persisted_file(path: &Path) -> std::io::Result<Option<PersistedFile>> {
    let file = match open_persisted_file(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let identity = file_identity(&file)?;
    if file.metadata()?.len() > MAX_PERSISTED_CONFIG_BYTES as u64 {
        return Err(configuration_too_large());
    }

    let mut bytes = Vec::new();
    file.take((MAX_PERSISTED_CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PERSISTED_CONFIG_BYTES {
        return Err(configuration_too_large());
    }
    Ok(Some(PersistedFile { identity, bytes }))
}
