//! STT runtime: dispatcher, error contract, and local fallback artifacts.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LOG_GENERATIONS: usize = 3;
const MAX_LOG_MESSAGE_BYTES: usize = 16 * 1024;
static LOG_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn logs_dir() -> PathBuf {
    crate::paths::app_data_dir().join("logs")
}

pub fn yap_log_path() -> PathBuf {
    logs_dir().join("yap.log")
}

pub fn stt_log_path() -> PathBuf {
    logs_dir().join("asr.log")
}

fn format_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("{secs}")
}

fn append_log(path: &Path, message: &str) {
    let _ = append_log_with_limits(path, message, MAX_LOG_BYTES, MAX_LOG_GENERATIONS);
}

fn append_log_with_limits(
    path: &Path,
    message: &str,
    max_bytes: u64,
    generations: usize,
) -> std::io::Result<()> {
    if max_bytes == 0 || generations == 0 {
        return Ok(());
    }
    let _guard = LOG_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| std::io::Error::other("log writer lock poisoned"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    refuse_non_regular_log(path)?;

    let sanitized = message.replace(['\r', '\n'], " ");
    let detail = truncate_utf8(&sanitized, MAX_LOG_MESSAGE_BYTES);
    let mut line = format!("{} {detail}\n", format_timestamp()).into_bytes();
    if line.len() as u64 > max_bytes {
        line.truncate(max_bytes as usize);
        if let Some(last) = line.last_mut() {
            *last = b'\n';
        }
    }
    let current_bytes = std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Ok(0)
            } else {
                Err(error)
            }
        })?;
    if current_bytes.saturating_add(line.len() as u64) > max_bytes {
        rotate_logs(path, generations)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(&line)
}

fn refuse_non_regular_log(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_file() => {
            Err(std::io::Error::other("log path is not a regular file"))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn rotate_logs(path: &Path, generations: usize) -> std::io::Result<()> {
    let oldest = rotated_log_path(path, generations);
    remove_rotation_if_present(&oldest)?;
    for generation in (1..generations).rev() {
        let from = rotated_log_path(path, generation);
        let to = rotated_log_path(path, generation + 1);
        rename_rotation_if_present(&from, &to)?;
    }
    rename_rotation_if_present(path, &rotated_log_path(path, 1))
}

fn remove_rotation_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn rename_rotation_if_present(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn rotated_log_path(path: &Path, generation: usize) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(format!(".{generation}"));
    PathBuf::from(name)
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

pub(crate) fn log_stt(message: &str) {
    append_log(&stt_log_path(), message);
}

pub(crate) fn log_stt_timed(phase: &str, elapsed: Duration, detail: &str) {
    log_stt(&format!("[{phase}] +{}ms {detail}", elapsed.as_millis()));
}

pub(crate) fn log_yap(message: &str) {
    append_log(&yap_log_path(), message);
}

pub mod dispatch;
pub mod error;
pub mod fallback_model;
pub mod model;
pub mod nemotron;
pub mod parity;
pub mod settings;

#[cfg(test)]
mod tests {
    use super::{append_log_with_limits, rotated_log_path};

    fn temp_log_root(label: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("yap-{label}-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn logs_are_bounded_rotated_and_single_line() {
        let root = temp_log_root("bounded-log");
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("yap.log");

        for index in 0..12 {
            append_log_with_limits(&path, &format!("entry-{index}\nprivate"), 64, 2).unwrap();
        }

        for candidate in [
            &path,
            &rotated_log_path(&path, 1),
            &rotated_log_path(&path, 2),
        ] {
            if candidate.exists() {
                assert!(std::fs::metadata(candidate).unwrap().len() <= 64);
                let text = std::fs::read_to_string(candidate).unwrap();
                assert!(!text.contains("\nprivate\n"));
            }
        }
        assert!(!rotated_log_path(&path, 3).exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_utf8_messages_are_truncated_without_splitting_codepoints() {
        let root = temp_log_root("utf8-log");
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("asr.log");

        append_log_with_limits(&path, &"é".repeat(20_000), 128, 1).unwrap();

        assert!(std::fs::metadata(&path).unwrap().len() <= 128);
        assert!(std::fs::read_to_string(&path).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
}
