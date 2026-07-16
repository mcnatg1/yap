use super::*;
use crate::audio::recording::{CommitFaultPoint, StreamingRecording};
use crate::audio::session::{SessionId, SessionMetadata, SessionMode, SessionOrigin, TriggerMode};

fn live_view(final_text: Option<&str>, partial_text: Option<&str>) -> live::state::LiveSessionView {
    live::state::LiveSessionView {
        visibility: live::state::LiveOverlayVisibility::Enabled,
        status: live::state::LiveSessionStatus::Idle,
        route: live::state::LiveRoute::None,
        capture_mode: live::state::LiveCaptureMode::PushToTalk,
        active_capture_mode: None,
        hotkey: String::new(),
        paste_hotkey: String::new(),
        input_device_id: None,
        input_device_label: None,
        level: None,
        partial_text: partial_text.map(str::to_string),
        final_text: final_text.map(str::to_string),
        transcription_degraded: false,
        error: None,
    }
}

fn recover_session_for_test(
    dir: &Path,
    session_id: &SessionId,
) -> Result<SavedLiveSession, String> {
    let candidate = recoverable_session_from_dir(dir, session_id)?;
    let expected = recoverable_session_artifact_path(&candidate)
        .ok_or_else(|| "missing recoverable test artifact".to_string())?;
    recover_live_session_in_dir(dir, session_id.to_string(), expected.to_string())
}

fn delete_recoverable_session_for_test(dir: &Path, session_id: &SessionId) -> Result<(), String> {
    let candidate = recoverable_session_from_dir(dir, session_id)?;
    let expected = recoverable_session_artifact_path(&candidate)
        .ok_or_else(|| "missing recoverable test artifact".to_string())?;
    delete_recoverable_live_session_in_dir(dir, session_id.to_string(), expected.to_string())
}

fn delete_saved_session_for_test(dir: &Path, session_id: &SessionId) -> Result<(), String> {
    let saved = list_session_files_from_dir(dir)?
        .into_iter()
        .find(|saved| saved.session_id == session_id.as_str())
        .ok_or_else(|| "missing saved test session".to_string())?;
    delete_saved_live_session_in_dir(
        dir,
        session_id.to_string(),
        saved.output_path,
        saved
            .capture_commit_path
            .ok_or_else(|| "missing saved test commit".to_string())?,
    )
}

fn set_old_modified_time(path: &Path) {
    let old = std::time::SystemTime::now()
        .checked_sub(PARTIAL_RECOVERY_TTL + Duration::from_secs(60))
        .unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old))
        .unwrap();
}

fn intent_quarantine_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains("deletion.v1.json.delete-")
        })
        .count()
}

mod catalog;
mod deletion_concurrency;
mod deletion_maintenance;
mod deletion_manual;
mod deletion_recovery;
mod deletion_retention;
mod recovery;
mod transcripts;

fn test_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yap-live-{label}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn skip_link_test_or_panic(error: std::io::Error) {
    if error.kind() == std::io::ErrorKind::PermissionDenied || error.raw_os_error() == Some(1314) {
        return;
    }
    panic!("failed to create test symlink: {error}");
}

#[cfg(unix)]
fn create_file_symlink_for_test(
    original: &std::path::Path,
    link: &std::path::Path,
) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn create_file_symlink_for_test(
    original: &std::path::Path,
    link: &std::path::Path,
) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}

fn assert_partial_capture_transcript(fault: CommitFaultPoint) {
    let dir = test_dir(&format!("partial-transcript-{fault:?}"));
    let session = SessionId::new("s-partial-transcript").unwrap();
    let mut recording =
        StreamingRecording::create_with_fault(&dir, session.clone(), fault).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    let lineage_hash = capture.capture_sidecar_sha256().unwrap().to_string();
    let lineage_file = capture
        .partial_lineage
        .as_ref()
        .map(|lineage| lineage.capture_sidecar_file.clone())
        .or_else(|| {
            capture
                .committed
                .as_ref()
                .map(|committed| committed.manifest.capture_sidecar_file.clone())
        })
        .unwrap();

    let saved = save_finalized_capture_to_dir(
        &dir,
        &live_view(Some("transcript survives"), None),
        Some(capture),
    )
    .unwrap()
    .unwrap();

    let transcript = dir.join(format!("live-{session}.txt"));
    let revision = transcript_revision_path(&dir, &session, 1);
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&revision).unwrap()).unwrap();
    assert_eq!(
        std::fs::read_to_string(transcript).unwrap(),
        "transcript survives\n"
    );
    assert_eq!(value["status"], "partial");
    assert_eq!(
        value["textSha256"],
        recording::sha256_file(&dir.join(format!("live-{session}.txt"))).unwrap()
    );
    assert_eq!(value["captureSidecarSha256"], lineage_hash);
    assert_eq!(
        recording::sha256_file(&dir.join(lineage_file)).unwrap(),
        lineage_hash
    );
    assert!(saved.warning.unwrap().contains(AUDIO_SAVE_FAILED_WARNING));
    let scanned = recording::scan_recordings(&dir).unwrap();
    assert!(scanned.complete.is_empty());
    assert_eq!(scanned.partial.len(), 1);
    assert!(list_session_files_from_dir(&dir).unwrap().is_empty());
    std::fs::remove_dir_all(dir).ok();
}

fn assert_unavailable_recording_transcript(session: &str, panicking: bool) {
    let dir = test_dir(&format!("unavailable-recording-{session}"));
    let runtime = live::runtime::LiveRuntime::new();
    let session_id = SessionId::new(session).unwrap();
    if panicking {
        runtime.install_panicking_recording_for_test(session_id.clone());
    } else {
        runtime.install_unavailable_recording_for_test(session_id.clone());
    }

    let saved = save_session_files_to_dir(&runtime, &live_view(Some("survives"), None), &dir)
        .unwrap()
        .unwrap();
    let transcript = dir.join(format!("live-{session_id}.txt"));

    assert_eq!(std::fs::read_to_string(&transcript).unwrap(), "survives\n");
    assert_eq!(saved.source_path, saved.output_path);
    assert!(saved.warning.unwrap().contains(AUDIO_SAVE_FAILED_WARNING));
    assert!(!transcript_revision_path(&dir, &session_id, 1).exists());
    assert!(recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .is_empty());
    assert!(list_session_files_from_dir(&dir).unwrap().is_empty());
    assert!(
        runtime.finalize_recording().is_err(),
        "terminal error remains cached"
    );
    std::fs::remove_dir_all(dir).ok();
}
