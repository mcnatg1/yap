//! STT backends: dispatcher, error contract, Python fallback, and optional
//! CrispASR local fallback.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
pub(crate) fn hide_child_console(command: &mut std::process::Command) {
    command.creation_flags(0x08000000);
}

#[cfg(not(windows))]
pub(crate) fn hide_child_console(_command: &mut std::process::Command) {}

pub fn logs_dir() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("Yap").join("logs");
    }
    PathBuf::from("logs")
}

pub fn yap_log_path() -> PathBuf {
    logs_dir().join("yap.log")
}

pub fn stt_log_path() -> PathBuf {
    logs_dir().join("crispasr.log")
}

pub fn sidecar_stderr_log_path() -> PathBuf {
    logs_dir().join("crispasr-sidecar.log")
}

fn format_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("{secs}")
}

fn append_log(path: &std::path::Path, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{} {}", format_timestamp(), message);
    }
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

pub mod backend;
pub mod binary;
pub mod crispasr;
pub mod dispatch;
pub mod error;
pub mod gpu;
pub mod model;
pub mod parity;
pub mod pin;
pub mod progress;
pub mod python;
pub mod settings;
pub mod sidecar;
