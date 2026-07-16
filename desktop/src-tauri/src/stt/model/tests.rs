use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::time::Instant;

use super::{
    cleanup_stale_download_temps, hf_resolve_url, models_dir_from,
    operation::DownloadOperation,
    progress::{progress_metrics, BodyProgress},
    sha256_file, verify_sha256, write_text_atomically, DownloadProgress,
};
use crate::stt::error::SttError;

struct TestDir(PathBuf);

impl TestDir {
    fn new(prefix: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[test]
fn models_dir_prefers_override() {
    let custom = std::env::temp_dir().join("custom-yap-models");
    let dir = models_dir_from(|key| match key {
        "YAP_MODELS_DIR" => Some(custom.display().to_string()),
        _ => None,
    });
    assert_eq!(dir, custom);
}

#[test]
fn models_dir_uses_app_data_override() {
    let local = std::env::temp_dir().join("local-data");
    let dir = models_dir_from(|key| match key {
        "YAP_APP_DATA_DIR" => Some(local.display().to_string()),
        _ => None,
    });
    assert_eq!(dir, local.join("models"));
}

#[test]
fn hf_resolve_url_is_pinned_by_revision() {
    assert_eq!(
        hf_resolve_url("owner/repo", "abc123", "model.onnx"),
        "https://huggingface.co/owner/repo/resolve/abc123/model.onnx"
    );
}

#[test]
fn verify_sha256_matches_and_mismatches() {
    let dir = TestDir::new("yap-sha");
    let file = dir.0.join("model.bin");
    std::fs::write(&file, b"hello").unwrap();
    assert_eq!(
        sha256_file(&file).unwrap(),
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    assert_eq!(verify_sha256(&file, "bad"), Err(SttError::ModelCorrupt));
}

#[test]
fn progress_metrics_handle_partial_complete_and_zero_elapsed() {
    let half = DownloadProgress {
        downloaded_bytes: 50,
        total_bytes: Some(200),
        elapsed_ms: 250,
    };
    assert_eq!(half.percent(), Some(25.0));
    assert_eq!(half.speed_mbps(), Some(0.0016));
    assert_eq!(progress_metrics(300, Some(100), 1).0, Some(100.0));
    assert_eq!(progress_metrics(1, None, 0), (None, None));
}

#[test]
fn empty_body_chunks_do_not_extend_the_no_progress_deadline() {
    let base = Instant::now();
    let timeout = Duration::from_secs(30);
    let mut progress = BodyProgress::new(6, base, timeout);
    let original_deadline = progress.deadline();

    assert!(!progress
        .record_chunk(&[], base + Duration::from_secs(10))
        .unwrap());
    assert_eq!(progress.deadline(), original_deadline);

    assert!(progress
        .record_chunk(b"abc", base + Duration::from_secs(11))
        .unwrap());
    assert_eq!(
        progress.deadline(),
        base + Duration::from_secs(11) + timeout
    );
}

#[test]
fn atomic_text_write_replaces_existing_marker_without_delete_window() {
    let dir = TestDir::new("yap-marker");
    let marker = dir.0.join("model.verified");
    std::fs::write(&marker, "old").unwrap();

    write_text_atomically(&marker, "new").unwrap();

    assert_eq!(std::fs::read_to_string(marker).unwrap(), "new");
}

#[test]
fn stale_temp_cleanup_failure_is_recorded_without_touching_the_destination() {
    let dir = TestDir::new("yap-stale-cleanup-failure");
    let destination = dir.0.join("model.bin");
    let stale_temp = dir.0.join("model.bin.op-1-2-3-4.part");
    std::fs::write(&destination, b"verified-old").unwrap();
    std::fs::create_dir(&stale_temp).unwrap();
    let operation = DownloadOperation::new(9);

    assert_eq!(
        cleanup_stale_download_temps(&destination, &operation),
        Err(SttError::ModelMissing)
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"verified-old");
    assert!(stale_temp.is_dir());
    assert!(operation
        .take_cleanup_failure()
        .is_some_and(|message| message.contains("refusing to remove directory")));
}
