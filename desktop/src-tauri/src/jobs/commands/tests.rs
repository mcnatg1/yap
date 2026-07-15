use super::super::remote;
use super::*;
use crate::{
    commands::media_protocol::MediaOwner,
    jobs::{
        JobLedger, NewRecordingJob, RecordingJobStatus, RecordingRoute, SessionMode, SessionOrigin,
        SourceOwnership,
    },
};
use std::{
    cell::{Cell, RefCell},
    fs,
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, UNIX_EPOCH},
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

mod authority_admission;
mod catalog_imports;
mod cleanup_retention;
mod retry_security;
mod snapshot_limits;

#[cfg(unix)]
fn create_reparse_point(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_reparse_point(target: &Path, link: &Path) -> std::io::Result<()> {
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
fn remove_reparse_point(link: &Path) -> std::io::Result<()> {
    fs::remove_file(link)
}

#[cfg(windows)]
fn remove_reparse_point(link: &Path) -> std::io::Result<()> {
    fs::remove_dir(link)
}

fn capability_free_failed(view: &RecordingJobView) -> bool {
    view.status == RecordingJobStatus::Failed
        && view.source_path.is_none()
        && view.playback_path.is_none()
}

fn open_and_reveal_are_denied(
    jobs: &RecordingJobs,
    source: &Path,
    general_registry: &Path,
) -> bool {
    let authorization_denied = || {
        crate::file_actions::openable_app_path_from_registries(
            source.display().to_string(),
            general_registry,
            &jobs.registry_path,
            jobs.owned_dir(),
        )
        .is_err()
    };
    authorization_denied() && authorization_denied()
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "yap-job-commands-{label}-{}-{id}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_pcm_wav(path: &Path, pcm: &[u8]) {
    let mut file = fs::File::create(path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36_u32 + pcm.len() as u32).to_le_bytes())
        .unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap();
    file.write_all(&16_000_u32.to_le_bytes()).unwrap();
    file.write_all(&32_000_u32.to_le_bytes()).unwrap();
    file.write_all(&2_u16.to_le_bytes()).unwrap();
    file.write_all(&16_u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&(pcm.len() as u32).to_le_bytes()).unwrap();
    file.write_all(pcm).unwrap();
    file.sync_all().unwrap();
}
