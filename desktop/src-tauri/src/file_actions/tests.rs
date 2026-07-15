use super::*;
use crate::audio::{recording::StreamingRecording, session::SessionId};
use crate::recording_access::{
    is_yap_media_or_transcript_path,
    metadata_is_reparse_point_for_test as metadata_is_reparse_point, openable_app_path_from,
    playable_recording_path, read_registered_playback_paths, register_playback_path_at,
    register_playback_path_at_from_owned_dir,
    register_recording_job_playback_path_at_from_owned_dir, registered_playback_path_at,
    restore_playback_path_at, restore_playback_path_at_with, write_registered_playback_paths,
    RecordingPlaybackRegistry, MAX_REGISTERED_PLAYBACK_PATHS, NATIVE_SELECTION_REGISTRY_VERSION,
};
use std::sync::atomic::{AtomicBool, Ordering};

static TEMP_TEST_DIR_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn temp_test_dir(name: &str) -> std::path::PathBuf {
    let sequence = TEMP_TEST_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "yap-{name}-{}-{}-{sequence}",
        std::process::id(),
        crate::live::recordings::unix_millis_now().unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

mod playback;
mod transcript_io;
mod transcript_prune;
mod transcript_read;
mod transcript_write;

#[cfg(unix)]
fn create_reparse_point(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_reparse_point(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    let target_dir = target.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "target has no parent")
    })?;
    let output = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(link)
        .arg(target_dir)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

#[cfg(unix)]
fn remove_reparse_point(link: &std::path::Path) -> std::io::Result<()> {
    std::fs::remove_file(link)
}

#[cfg(windows)]
fn remove_reparse_point(link: &std::path::Path) -> std::io::Result<()> {
    std::fs::remove_dir(link)
}
