use std::{io::Read, path::Path};

use sha2::{Digest, Sha256};

use crate::stt::error::SttError;

use super::{io_error_to_stt, operation::DownloadOperation, DownloadRequest};

const HASH_BUFFER_BYTES: usize = 64 * 1024;

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; HASH_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_digest(hasher.finalize()))
}

pub fn verify_sha256(path: &Path, expected: &str) -> Result<(), SttError> {
    let actual = sha256_file(path).map_err(|_| SttError::ModelMissing)?;
    actual
        .eq_ignore_ascii_case(expected)
        .then_some(())
        .ok_or(SttError::ModelCorrupt)
}

pub(super) fn verify_download(
    path: &Path,
    request: &DownloadRequest,
    operation: &DownloadOperation,
) -> Result<(), SttError> {
    let metadata = std::fs::metadata(path).map_err(io_error_to_stt)?;
    if metadata.len() != request.expected_bytes {
        return Err(SttError::ModelCorrupt);
    }
    let mut file = std::fs::File::open(path).map_err(io_error_to_stt)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; HASH_BUFFER_BYTES];
    loop {
        if operation.is_cancelled() {
            return Err(SttError::ModelInstallCancelled);
        }
        let read = file.read(&mut buffer).map_err(io_error_to_stt)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = hex_digest(hasher.finalize());
    actual
        .eq_ignore_ascii_case(&request.expected_sha256)
        .then_some(())
        .ok_or(SttError::ModelCorrupt)
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
    }
    hex
}
