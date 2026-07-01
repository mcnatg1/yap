//! STT backends: dispatcher, error contract, Python fallback, and the CrispASR
//! HTTP sidecar. Spec: docs/superpowers/specs/2026-06-30-crispasr-stt-sidecar-design.md

use std::io::Write;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
pub(crate) fn hide_child_console(command: &mut std::process::Command) {
    command.creation_flags(0x08000000);
}

#[cfg(not(windows))]
pub(crate) fn hide_child_console(_command: &mut std::process::Command) {}

pub(crate) fn log_stt(message: &str) {
    let path = stt_log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let _ = writeln!(file, "{stamp} {message}");
    }
}

fn stt_log_path() -> std::path::PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return std::path::PathBuf::from(local).join("Yap").join("logs").join("crispasr.log");
    }
    std::path::PathBuf::from("crispasr.log")
}

pub mod backend;
pub mod crispasr;
pub mod error;
pub mod model;
pub mod parity;
pub mod pin;
pub mod python;
pub mod sidecar;
