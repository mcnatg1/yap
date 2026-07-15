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

#[test]
fn transcript_text_prefers_final_then_partial() {
    let mut view = live_view(Some("final"), Some("partial"));

    assert_eq!(transcript_text(&view).as_deref(), Some("final"));
    view.final_text = None;
    assert_eq!(transcript_text(&view).as_deref(), Some("partial"));
}

#[test]
fn damaged_complete_commit_past_partial_ttl_is_preserved_and_warned() {
    let dir = test_dir("damaged-commit-ttl");
    let session = SessionId::new("s-damaged-commit-ttl").unwrap();
    let mut capture = StreamingRecording::create(&dir, session.clone()).unwrap();
    capture.append_pcm16(&[1, 0]).unwrap();
    capture.finalize().unwrap();
    let journal = dir.join(format!("live-{session}.capture.journal.part"));
    std::fs::write(&journal, b"residual journal").unwrap();
    std::fs::write(dir.join(format!("live-{session}.commit.json")), b"{broken").unwrap();
    set_old_modified_time(&dir.join(format!("live-{session}.wav")));
    set_old_modified_time(&journal);

    let catalog = list_session_catalog_from_dir(&dir).unwrap();

    assert!(catalog.sessions.is_empty());
    assert!(catalog
        .maintenance_warnings
        .iter()
        .any(|warning| warning.contains("damaged")));
    assert!(dir.join(format!("live-{session}.wav")).is_file());
    assert!(journal.is_file());
    assert!(list_recoverable_live_sessions_from_dir(&dir)
        .unwrap()
        .is_empty());
    assert!(recover_session_for_test(&dir, &session).is_err());
    assert!(delete_recoverable_session_for_test(&dir, &session).is_err());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn valid_recovered_partial_commit_past_partial_ttl_remains_recoverable() {
    let dir = test_dir("recovered-partial-ttl");
    let session = SessionId::new("s-recovered-partial-ttl").unwrap();
    {
        let mut capture = StreamingRecording::create(&dir, session.clone()).unwrap();
        capture.append_pcm16(&[1, 0]).unwrap();
    }
    recover_session_for_test(&dir, &session).unwrap();
    for name in [
        format!("live-{session}.wav"),
        format!("live-{session}.capture.journal.part"),
        format!("live-{session}.capture.partial.json"),
        format!("live-{session}.commit.json"),
    ] {
        set_old_modified_time(&dir.join(name));
    }

    let catalog = list_session_catalog_from_dir(&dir).unwrap();

    assert!(catalog
        .sessions
        .iter()
        .any(|saved| saved.recovery_state.as_deref() == Some("recoverable")));
    assert!(dir.join(format!("live-{session}.wav")).is_file());
    assert!(dir.join(format!("live-{session}.commit.json")).is_file());
    delete_recoverable_session_for_test(&dir, &session).unwrap();
    assert!(!dir.join(format!("live-{session}.wav")).exists());
    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn completed_transcript_text_never_promotes_a_partial() {
    let mut view = live_view(None, Some("partial"));
    assert_eq!(completed_transcript_text(&view), None);

    view.final_text = Some("final".into());
    assert_eq!(completed_transcript_text(&view).as_deref(), Some("final"));
}

#[cfg(target_os = "windows")]
#[test]
fn stable_path_strings_remove_windows_verbatim_prefixes() {
    assert_eq!(
        stable_path_string(std::path::Path::new(r"\\?\C:\Users\Me\live-1.txt")),
        r"C:\Users\Me\live-1.txt"
    );
    assert_eq!(
        stable_path_string(std::path::Path::new(r"\\?\UNC\server\share\live-1.txt")),
        r"\\server\share\live-1.txt"
    );
}

#[test]
fn transcript_text_cleans_streaming_artifacts() {
    let mut view = live_view(Some("  THank   you.. "), None);

    assert_eq!(transcript_text(&view).as_deref(), Some("Thank you."));
    view.final_text = Some("NASA called.".into());
    assert_eq!(transcript_text(&view).as_deref(), Some("NASA called."));
}

#[test]
fn normal_history_scan_ignores_pre_release_timestamp_pairs() {
    let dir = std::env::temp_dir().join(format!("yap-live-scan-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = dir.join("live-200.txt");
    let audio = dir.join("live-200.wav");
    let ignored = dir.join("note.txt");
    std::fs::write(&transcript, "hello\n").unwrap();
    std::fs::write(&audio, b"RIFF").unwrap();
    std::fs::write(&ignored, "not a live session\n").unwrap();

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert!(sessions.is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn native_history_sources_share_one_catalog_snapshot() {
    let dir = test_dir("native-history-source-snapshot");
    let committed_session = SessionId::new("s-history-committed").unwrap();
    let mut committed = StreamingRecording::create(&dir, committed_session.clone()).unwrap();
    committed.append_pcm16(&[1, 0]).unwrap();
    committed.finalize().unwrap();

    let partial_session = SessionId::new("s-history-partial").unwrap();
    {
        let mut partial = StreamingRecording::create(&dir, partial_session.clone()).unwrap();
        partial.append_pcm16(&[1, 0]).unwrap();
    }

    let sources = list_history_sources_from_dir_at_with_queue_observer(
        &dir,
        OffsetDateTime::now_utc(),
        || {},
    )
    .unwrap();

    assert!(sources
        .saved
        .sessions
        .iter()
        .any(|session| session.session_id == committed_session.as_str()));
    assert!(sources
        .recoverable
        .iter()
        .any(|session| session.session_id == partial_session.as_str()));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn normal_history_scan_leaves_a_wav_only_pre_release_recording_untouched() {
    let dir = test_dir("ignore-legacy-wav");
    let session = SessionId::new("s-migrate-legacy-source").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let legacy = "live-1720656000000.wav";
    std::fs::rename(dir.join(format!("live-{session}.wav")), dir.join(legacy)).unwrap();
    std::fs::remove_file(dir.join(format!("live-{session}.capture.json"))).unwrap();
    std::fs::remove_file(dir.join(format!("live-{session}.commit.json"))).unwrap();

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert!(sessions.is_empty());
    assert!(dir.join(legacy).is_file());
    assert!(!dir.join(format!("live-{session}.capture.json")).exists());
    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn normal_history_scan_leaves_pre_release_wav_and_txt_untouched() {
    let dir = test_dir("ignore-legacy-pair");
    let session = SessionId::new("s-migrate-legacy-pair-source").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let legacy_wav = "live-1720656000001.wav";
    let legacy_txt = "live-1720656000001.txt";
    std::fs::rename(
        dir.join(format!("live-{session}.wav")),
        dir.join(legacy_wav),
    )
    .unwrap();
    std::fs::remove_file(dir.join(format!("live-{session}.capture.json"))).unwrap();
    std::fs::remove_file(dir.join(format!("live-{session}.commit.json"))).unwrap();
    std::fs::write(dir.join(legacy_txt), "old transcript\n").unwrap();

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert!(sessions.is_empty());
    assert!(dir.join(legacy_wav).is_file());
    assert_eq!(
        std::fs::read_to_string(dir.join(legacy_txt)).unwrap(),
        "old transcript\n"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn new_style_uncommitted_artifacts_are_not_listed_as_legacy_history() {
    let dir = test_dir("uncommitted-new-style");
    let session = SessionId::new("s-pending").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    std::fs::write(dir.join(format!("live-{session}.txt")), "pending\n").unwrap();

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert!(sessions.is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recovery_patches_a_private_wav_and_publishes_only_partial_metadata() {
    let dir = test_dir("recover-private-wav");
    let session = SessionId::new("s-recover-private-wav").unwrap();
    {
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0, 2, 0]).unwrap();
    }

    let recoverable = list_recoverable_live_sessions_from_dir(&dir).unwrap();
    assert_eq!(recoverable.len(), 1);
    let saved = recover_session_for_test(&dir, &session).unwrap();

    assert_eq!(saved.recovery_state.as_deref(), Some("recoverable"));
    assert!(dir.join(format!("live-{session}.wav")).is_file());
    let commit = std::fs::read_to_string(dir.join(format!("live-{session}.commit.json"))).unwrap();
    assert!(commit.contains("\"status\":\"partial\""));
    assert!(list_session_files_from_dir(&dir)
        .unwrap()
        .iter()
        .any(|entry| entry.recovery_state.as_deref() == Some("recoverable")));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recovery_retry_returns_the_existing_verified_partial_commit() {
    let dir = test_dir("recover-retry");
    let session = SessionId::new("s-recover-retry").unwrap();
    {
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0, 2, 0]).unwrap();
    }

    let first = recover_session_for_test(&dir, &session).unwrap();
    let retry = recover_session_for_test(&dir, &session).unwrap();

    assert_eq!(retry.capture_commit_path, first.capture_commit_path);
    assert_eq!(retry.source_path, first.source_path);
    assert_eq!(retry.recovery_state.as_deref(), Some("recoverable"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn journal_owned_final_wav_without_a_partial_sidecar_remains_recoverable() {
    let dir = test_dir("journal-owned-orphan");
    let session = SessionId::new("s-journal-owned-orphan").unwrap();
    {
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
    }
    let partial = dir.join(format!("live-{session}.wav.part"));
    let admitted = recording::admit_expected_private_regular_artifact(&partial, &partial).unwrap();
    recording::recover_partial_wav_with_identity(&dir, &session, &admitted).unwrap();

    let recoverable = list_recoverable_live_sessions_from_dir(&dir).unwrap();

    assert_eq!(recoverable.len(), 1);
    assert!(recoverable[0]
        .audio_partial_path
        .as_deref()
        .unwrap()
        .ends_with(".wav"));
    assert!(recoverable[0]
        .journal_partial_path
        .as_deref()
        .unwrap()
        .ends_with(".capture.journal.part"));
    assert!(!dir
        .join(format!("live-{session}.capture.partial.json"))
        .exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn orphan_audio_after_sidecar_failure_is_visible_retryable_and_deletable() {
    let dir = test_dir("orphan-audio-retry");
    let session = SessionId::new("s-orphan-audio-retry").unwrap();
    let mut recording =
        StreamingRecording::create_with_fault(&dir, session.clone(), CommitFaultPoint::AudioSync)
            .unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let invalid_sidecar = dir.join(format!("live-{session}.capture.partial.json"));
    std::fs::write(&invalid_sidecar, "invalid sidecar").unwrap();

    assert!(recover_session_for_test(&dir, &session).is_err());
    assert!(dir.join(format!("live-{session}.wav")).is_file());
    let partial = list_recoverable_live_sessions_from_dir(&dir).unwrap();
    assert_eq!(partial.len(), 1);
    assert!(partial[0]
        .audio_partial_path
        .as_deref()
        .unwrap()
        .ends_with(".wav"));

    delete_recoverable_session_for_test(&dir, &session).unwrap();
    assert!(!dir.join(format!("live-{session}.wav")).exists());
    std::fs::remove_file(&invalid_sidecar).ok();

    let mut retry =
        StreamingRecording::create_with_fault(&dir, session.clone(), CommitFaultPoint::AudioSync)
            .unwrap();
    retry.append_pcm16(&[1, 0]).unwrap();
    retry.finalize().unwrap();
    assert!(recover_session_for_test(&dir, &session).is_ok());
    assert!(dir.join(format!("live-{session}.commit.json")).is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recovery_delete_rejects_unknown_sessions_and_preserves_unrelated_files() {
    let dir = test_dir("recover-delete-boundary");
    let session = SessionId::new("s-recover-delete-boundary").unwrap();
    {
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
    }
    let unrelated = dir.join("not-yap.txt");
    std::fs::write(&unrelated, "keep").unwrap();

    assert!(delete_recoverable_live_session_in_dir(
        &dir,
        "../outside".into(),
        unrelated.display().to_string(),
    )
    .is_err());
    delete_recoverable_session_for_test(&dir, &session).unwrap();

    assert!(unrelated.is_file());
    assert!(!dir.join(format!("live-{session}.wav.part")).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recovery_actions_reject_a_mismatched_expected_artifact_without_mutation() {
    let dir = test_dir("recover-expected-identity");
    let session = SessionId::new("s-recover-expected-identity").unwrap();
    {
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
    }
    let unrelated = dir.join("unrelated.wav.part");
    std::fs::write(&unrelated, b"unrelated").unwrap();

    assert!(recover_live_session_in_dir(
        &dir,
        session.to_string(),
        unrelated.display().to_string(),
    )
    .is_err());
    assert!(delete_recoverable_live_session_in_dir(
        &dir,
        session.to_string(),
        unrelated.display().to_string(),
    )
    .is_err());

    assert!(dir.join(format!("live-{session}.wav.part")).is_file());
    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recovery_rejects_a_multi_link_private_wav_before_mutation() {
    let dir = test_dir("recover-hardlinked-private-wav");
    let session = SessionId::new("s-recover-hardlinked-private-wav").unwrap();
    {
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
    }
    let partial = dir.join(format!("live-{session}.wav.part"));
    let external = dir.join("external-session.wav");
    if std::fs::hard_link(&partial, &external).is_err() {
        std::fs::remove_dir_all(dir).ok();
        return;
    }
    let original = std::fs::read(&partial).unwrap();

    let result =
        recover_live_session_in_dir(&dir, session.to_string(), partial.display().to_string());

    assert!(result.is_err());
    assert_eq!(std::fs::read(&partial).unwrap(), original);
    assert_eq!(std::fs::read(&external).unwrap(), original);
    assert!(!dir.join(format!("live-{session}.wav")).exists());
    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recoverable_delete_rejects_a_replacement_after_identity_admission() {
    let dir = test_dir("recover-delete-admission-replacement");
    let session = SessionId::new("s-recover-delete-admission-replacement").unwrap();
    {
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
    }
    let expected = dir.join(format!("live-{session}.wav.part"));
    let replacement = b"replacement must survive";

    let result = delete_recoverable_live_session_in_dir_with_mutation_barrier(
        &dir,
        session.to_string(),
        expected.display().to_string(),
        || {
            std::fs::remove_file(&expected).unwrap();
            std::fs::write(&expected, replacement).unwrap();
        },
    );

    assert!(result.is_err());
    assert_eq!(std::fs::read(&expected).unwrap(), replacement);
    assert!(dir
        .join(format!("live-{session}.capture.journal.part"))
        .is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recoverable_delete_preserves_a_same_content_sibling_replacement() {
    let dir = test_dir("recover-delete-sibling-replacement");
    let session = SessionId::new("s-recover-delete-sibling-replacement").unwrap();
    {
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
    }
    let expected = dir.join(format!("live-{session}.wav.part"));
    let journal = dir.join(format!("live-{session}.capture.journal.part"));
    let original_journal = std::fs::read(&journal).unwrap();

    let result = delete_recoverable_live_session_in_dir_with_mutation_barrier(
        &dir,
        session.to_string(),
        expected.display().to_string(),
        || {
            std::fs::remove_file(&journal).unwrap();
            std::fs::write(&journal, &original_journal).unwrap();
        },
    );

    assert!(result.is_err());
    assert_eq!(std::fs::read(&journal).unwrap(), original_journal);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recoverable_delete_preserves_a_valid_sidecar_created_after_admission() {
    let dir = test_dir("recover-delete-late-sidecar");
    let session = SessionId::new("s-recover-delete-late-sidecar").unwrap();
    {
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
    }
    let expected = dir.join(format!("live-{session}.wav.part"));
    let sidecar = dir.join(format!("live-{session}.capture.partial.json"));
    let sidecar_text =
        format!("{{\"schemaVersion\":1,\"sessionId\":\"{session}\",\"status\":\"partial\"}}\n");

    delete_recoverable_live_session_in_dir_with_mutation_barrier(
        &dir,
        session.to_string(),
        expected.display().to_string(),
        || std::fs::write(&sidecar, &sidecar_text).unwrap(),
    )
    .unwrap();

    assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), sidecar_text);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recover_delete_and_catalog_threads_share_one_mutation_owner() {
    let dir = test_dir("recover-delete-list-owner-race");
    let session = SessionId::new("s-recover-delete-list-owner-race").unwrap();
    {
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
    }
    let expected = dir.join(format!("live-{session}.wav.part"));
    let (delete_ready_tx, delete_ready_rx) = std::sync::mpsc::channel();
    let (release_delete_tx, release_delete_rx) = std::sync::mpsc::channel();
    let delete_dir = dir.clone();
    let delete_session = session.clone();
    let delete_expected = expected.display().to_string();
    let deleting = std::thread::spawn(move || {
        delete_recoverable_live_session_in_dir_with_mutation_barrier(
            &delete_dir,
            delete_session.to_string(),
            delete_expected,
            || {
                delete_ready_tx.send(()).unwrap();
                release_delete_rx.recv().unwrap();
            },
        )
    });
    delete_ready_rx.recv().unwrap();

    let recover_dir = dir.clone();
    let recover_session = session.clone();
    let recover_expected = expected.display().to_string();
    let (recover_queued_tx, recover_queued_rx) = std::sync::mpsc::channel();
    let (recover_tx, recover_rx) = std::sync::mpsc::channel();
    let recovering = std::thread::spawn(move || {
        recover_tx
            .send(recover_live_session_in_dir_with_queue_observer(
                &recover_dir,
                recover_session.to_string(),
                recover_expected,
                || recover_queued_tx.send(()).unwrap(),
                || {},
            ))
            .unwrap();
    });
    let list_dir = dir.clone();
    let (list_queued_tx, list_queued_rx) = std::sync::mpsc::channel();
    let (list_tx, list_rx) = std::sync::mpsc::channel();
    let listing = std::thread::spawn(move || {
        list_tx
            .send(list_session_catalog_from_dir_at_with_queue_observer(
                &list_dir,
                OffsetDateTime::now_utc(),
                || list_queued_tx.send(()).unwrap(),
            ))
            .unwrap();
    });

    recover_queued_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    list_queued_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(recover_rx.try_recv().is_err());
    assert!(list_rx.try_recv().is_err());
    release_delete_tx.send(()).unwrap();

    assert!(deleting.join().unwrap().is_ok());
    assert!(recover_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .is_err());
    assert!(list_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap()
        .sessions
        .is_empty());
    recovering.join().unwrap();
    listing.join().unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recovered_session_actions_bind_to_the_source_artifact() {
    let saved = SavedLiveSession {
        session_id: "recovered".into(),
        name: "live-recovered".into(),
        source_path: "C:/Yap/live-recovered.wav".into(),
        output_path: "C:/Yap/live-recovered.txt".into(),
        created_at_ms: 1,
        warning: None,
        capture_commit_path: Some("C:/Yap/live-recovered.commit.json".into()),
        recovery_state: Some("recovered".into()),
    };

    assert_eq!(
        saved_session_action_artifact_path(&saved),
        saved.source_path.as_str(),
    );
}

#[test]
fn committed_capture_is_listed_only_after_manifest_validation() {
    let dir = test_dir("committed-history");
    let session = SessionId::new("s-history").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    save_finalized_capture_to_dir(&dir, &live_view(Some("hello"), None), Some(capture)).unwrap();

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].name, format!("live-{session}"));
    assert!(sessions[0].source_path.ends_with(".wav"));
    assert!(sessions[0].created_at_ms > 0);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn committed_history_exposes_its_hash_validated_commit_path() {
    let dir = test_dir("committed-history-commit-path");
    let session_id = SessionId::new("s-history-commit-path").unwrap();
    let mut recording = StreamingRecording::create(&dir, session_id.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    save_finalized_capture_to_dir(&dir, &live_view(Some("hello"), None), Some(capture)).unwrap();

    let saved = list_session_files_from_dir(&dir).unwrap().pop().unwrap();
    let serialized = serde_json::to_value(saved).unwrap();

    assert_eq!(
        serialized["captureCommitPath"],
        serde_json::Value::String(
            dir.join(format!("live-{session_id}.commit.json"))
                .display()
                .to_string()
        )
    );
    assert_eq!(serialized["sessionId"], session_id.as_str());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn history_rejects_linked_legacy_transcripts_inside_or_outside_the_directory_when_supported() {
    let dir = test_dir("history-linked-legacy-transcript");
    let outside = std::env::temp_dir().join(format!(
        "yap-linked-transcript-target-{}",
        std::process::id()
    ));
    std::fs::remove_file(&outside).ok();
    std::fs::write(&outside, "outside\n").unwrap();
    let legacy = dir.join("live-401.txt");
    if let Err(error) = create_file_symlink_for_test(&outside, &legacy) {
        skip_link_test_or_panic(error);
        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(dir).ok();
        return;
    }

    assert!(list_session_files_from_dir(&dir).unwrap().is_empty());
    std::fs::remove_file(&legacy).ok();
    std::fs::remove_file(&outside).ok();

    let inside = dir.join("ordinary-transcript.txt");
    std::fs::write(&inside, "inside\n").unwrap();
    let internal_link = dir.join("live-402.txt");
    create_file_symlink_for_test(&inside, &internal_link).unwrap();
    assert!(list_session_files_from_dir(&dir).unwrap().is_empty());
    std::fs::remove_file(&internal_link).ok();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn history_ignores_linked_pre_release_audio_and_leaves_the_safe_transcript_untouched() {
    let dir = test_dir("history-linked-legacy-audio");
    let outside =
        std::env::temp_dir().join(format!("yap-linked-audio-target-{}", std::process::id()));
    std::fs::remove_file(&outside).ok();
    std::fs::write(&outside, b"RIFF").unwrap();
    let transcript = dir.join("live-402.txt");
    let audio = dir.join("live-402.wav");
    std::fs::write(&transcript, "safe\n").unwrap();
    if let Err(error) = create_file_symlink_for_test(&outside, &audio) {
        skip_link_test_or_panic(error);
        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(dir).ok();
        return;
    }

    assert!(!recording::is_regular_artifact(&audio));
    let sessions = list_session_files_from_dir(&dir).unwrap();
    assert!(sessions.is_empty());
    assert_eq!(std::fs::read_to_string(&transcript).unwrap(), "safe\n");
    std::fs::remove_file(&audio).ok();
    std::fs::remove_file(&outside).ok();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn committed_history_falls_back_to_audio_when_its_transcript_is_linked() {
    let dir = test_dir("history-linked-committed-transcript");
    let session = SessionId::new("s-linked-committed-transcript").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    save_finalized_capture_to_dir(&dir, &live_view(Some("hello"), None), Some(capture)).unwrap();
    let outside = std::env::temp_dir().join(format!(
        "yap-linked-committed-transcript-target-{}",
        std::process::id()
    ));
    std::fs::remove_file(&outside).ok();
    std::fs::write(&outside, "outside\n").unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));
    std::fs::remove_file(&transcript).unwrap();
    if let Err(error) = create_file_symlink_for_test(&outside, &transcript) {
        skip_link_test_or_panic(error);
        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(dir).ok();
        return;
    }

    let sessions = list_session_files_from_dir(&dir).unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].source_path.ends_with(".wav"));
    assert_eq!(sessions[0].output_path, sessions[0].source_path);
    std::fs::remove_file(&transcript).ok();
    std::fs::remove_file(&outside).ok();

    let inside = dir.join("ordinary-committed-transcript.txt");
    std::fs::write(&inside, "inside\n").unwrap();
    create_file_symlink_for_test(&inside, &transcript).unwrap();
    let sessions = list_session_files_from_dir(&dir).unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].source_path.ends_with(".wav"));
    assert_eq!(sessions[0].output_path, sessions[0].source_path);
    std::fs::remove_file(&transcript).ok();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_revision_rejects_a_linked_prior_revision_when_supported() {
    let dir = test_dir("linked-transcript-revision");
    let session = SessionId::new("s-linked-transcript-revision").unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));
    let transcript_receipt = write_new_text_file(&transcript, "first\n").unwrap();
    write_transcript_revision(
        &dir,
        &session,
        &"a".repeat(64),
        &transcript_receipt,
        "first",
        ResultStatus::Complete,
    )
    .unwrap();
    let outside =
        std::env::temp_dir().join(format!("yap-linked-revision-target-{}", std::process::id()));
    std::fs::remove_file(&outside).ok();
    std::fs::write(&outside, "outside revision\n").unwrap();
    let first = transcript_revision_path(&dir, &session, 1);
    std::fs::remove_file(&first).unwrap();
    if let Err(error) = create_file_symlink_for_test(&outside, &first) {
        skip_link_test_or_panic(error);
        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(dir).ok();
        return;
    }

    assert!(write_transcript_revision(
        &dir,
        &session,
        &"a".repeat(64),
        &transcript_receipt,
        "second",
        ResultStatus::Complete,
    )
    .is_err());
    assert!(!transcript_revision_path(&dir, &session, 2).exists());
    std::fs::remove_file(&first).ok();
    std::fs::remove_file(&outside).ok();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn partial_capture_before_sidecar_publication_keeps_transcript_and_publishes_partial_revision() {
    assert_partial_capture_transcript(CommitFaultPoint::AudioSync);
}

#[test]
fn partial_capture_after_sidecar_publication_keeps_transcript_and_publishes_partial_revision() {
    assert_partial_capture_transcript(CommitFaultPoint::CommitSync);
}

#[test]
fn worker_panic_still_publishes_a_usable_transcript_without_fabricating_history() {
    assert_unavailable_recording_transcript("s-worker-panic", true);
}

#[test]
fn unavailable_worker_still_publishes_a_usable_transcript_without_fabricating_history() {
    assert_unavailable_recording_transcript("s-worker-unavailable", false);
}

#[test]
fn transcript_sync_failure_does_not_rename_the_partial_file() {
    let dir = test_dir("transcript-sync-failure");
    let transcript = dir.join("live-301.txt");
    let renamed = std::cell::Cell::new(false);

    let error = write_new_text_file_with(
        &transcript,
        "hello\n",
        |_| Err(std::io::Error::other("injected transcript sync failure")),
        |_, _, _| {
            renamed.set(true);
            Err("test publisher should not be called".into())
        },
    )
    .unwrap_err();

    assert!(error.contains("injected transcript sync failure"));
    assert!(!renamed.get());
    assert!(!transcript.exists());
    assert!(!partial_text_path(&transcript).unwrap().exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_pre_link_replacement_keeps_the_attacker_staging_file_and_writes_no_revision() {
    let dir = test_dir("transcript-pre-link-replacement");
    let session = SessionId::new("s-transcript-pre-link-replacement").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));
    let partial = partial_text_path(&transcript).unwrap();

    let error = save_finalized_capture_to_dir_with_text_publisher(
        &dir,
        &live_view(Some("owned transcript"), None),
        Some(capture),
        |source, destination, owned| {
            let displaced = source.with_extension("displaced");
            std::fs::rename(source, &displaced).map_err(|error| error.to_string())?;
            std::fs::write(source, b"attacker staging").map_err(|error| error.to_string())?;
            recording::publish_no_replace(source, destination, owned, "publish live transcript")
        },
    )
    .unwrap_err();

    assert!(error.contains("staging path no longer names the owned file"));
    assert_eq!(std::fs::read(&partial).unwrap(), b"attacker staging");
    assert!(!transcript.exists());
    assert!(!transcript_revision_path(&dir, &session, 1).exists());
    assert_eq!(recording::scan_recordings(&dir).unwrap().complete.len(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_post_link_replacement_keeps_the_attacker_text_and_writes_no_revision() {
    let dir = test_dir("transcript-post-link-replacement");
    let session = SessionId::new("s-transcript-post-link-replacement").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));

    let error = save_finalized_capture_to_dir_with_text_publisher(
        &dir,
        &live_view(Some("owned transcript"), None),
        Some(capture),
        |source, destination, owned| {
            recording::publish_no_replace_with_after_link_for_test(
                source,
                destination,
                owned,
                "publish live transcript",
                || {
                    let displaced = destination.with_extension("displaced");
                    std::fs::rename(destination, displaced).unwrap();
                    std::fs::write(destination, b"attacker text").unwrap();
                },
            )
        },
    )
    .unwrap_err();

    assert!(error.contains("published destination does not name the owned file"));
    assert_eq!(std::fs::read(&transcript).unwrap(), b"attacker text");
    assert!(!transcript_revision_path(&dir, &session, 1).exists());
    assert_eq!(recording::scan_recordings(&dir).unwrap().complete.len(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_replacement_after_publication_preserves_independent_text_without_a_revision() {
    let dir = test_dir("transcript-post-publication-replacement");
    let session = SessionId::new("s-transcript-post-publication-replacement").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));

    let saved = save_finalized_capture_to_dir_with_text_publisher(
        &dir,
        &live_view(Some("owned transcript"), None),
        Some(capture),
        |source, destination, owned| {
            let published = recording::publish_no_replace(
                source,
                destination,
                owned,
                "publish live transcript",
            )?;
            let displaced = destination.with_extension("displaced");
            std::fs::rename(destination, displaced).map_err(|error| error.to_string())?;
            std::fs::write(destination, b"attacker transcript")
                .map_err(|error| error.to_string())?;
            Ok(published)
        },
    )
    .unwrap()
    .unwrap();

    assert_eq!(std::fs::read(&transcript).unwrap(), b"attacker transcript");
    assert!(saved
        .warning
        .as_deref()
        .unwrap_or_default()
        .contains("Transcript revision was not saved"));
    assert!(!transcript_revision_path(&dir, &session, 1).exists());
    let scan = recording::scan_recordings(&dir).unwrap();
    assert_eq!(scan.complete.len(), 1);
    assert_eq!(scan.complete[0].manifest.session_id, session);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_receipt_allows_destination_move_and_revalidates_identity() {
    let dir = test_dir("transcript-receipt-handle-lifetime");
    let session = SessionId::new("s-transcript-receipt-handle-lifetime").unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));

    let receipt = write_new_text_file(&transcript, "owned transcript\n").unwrap();

    receipt.revalidate().unwrap();
    let displaced = transcript.with_extension("displaced");
    std::fs::rename(&transcript, &displaced).unwrap();
    std::fs::write(&transcript, "replacement transcript\n").unwrap();
    assert!(displaced.is_file());
    assert!(transcript.is_file());
    assert!(receipt.revalidate().is_err());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_replacement_before_revision_publication_writes_no_revision() {
    let dir = test_dir("transcript-revision-pre-publication-replacement");
    let session = SessionId::new("s-transcript-revision-pre-publication").unwrap();
    let mut recording_capture = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording_capture.append_pcm16(&[1, 0]).unwrap();
    let manifest = recording_capture
        .finalize()
        .unwrap()
        .committed
        .unwrap()
        .manifest;
    let transcript = dir.join(format!("live-{session}.txt"));
    let receipt = write_new_text_file(&transcript, "owned transcript\n").unwrap();

    let error = write_transcript_revision_with_barrier(
        &dir,
        &session,
        &manifest.capture_sidecar_sha256,
        &receipt,
        "owned transcript",
        ResultStatus::Complete,
        |barrier| {
            if barrier == TranscriptRevisionPublicationBarrier::BeforePublication {
                let displaced = transcript.with_extension("displaced");
                std::fs::rename(&transcript, displaced).unwrap();
                std::fs::write(&transcript, "replacement transcript\n").unwrap();
            }
        },
    )
    .unwrap_err();

    assert!(error.contains("transcript path no longer names"));
    assert!(!transcript_revision_path(&dir, &session, 1).exists());
    assert_eq!(
        std::fs::read_to_string(&transcript).unwrap(),
        "replacement transcript\n"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_replacement_after_revision_publication_is_not_selected_by_history() {
    let dir = test_dir("transcript-revision-post-publication-replacement");
    let session = SessionId::new("s-transcript-revision-post-publication").unwrap();
    let mut recording_capture = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording_capture.append_pcm16(&[1, 0]).unwrap();
    let manifest = recording_capture
        .finalize()
        .unwrap()
        .committed
        .unwrap()
        .manifest;
    let transcript = dir.join(format!("live-{session}.txt"));
    let receipt = write_new_text_file(&transcript, "owned transcript\n").unwrap();

    let error = write_transcript_revision_with_barrier(
        &dir,
        &session,
        &manifest.capture_sidecar_sha256,
        &receipt,
        "owned transcript",
        ResultStatus::Complete,
        |barrier| {
            if barrier == TranscriptRevisionPublicationBarrier::AfterPublication {
                let displaced = transcript.with_extension("displaced");
                std::fs::rename(&transcript, displaced).unwrap();
                std::fs::write(&transcript, "replacement transcript\n").unwrap();
            }
        },
    )
    .unwrap_err();

    assert!(error.contains("transcript path no longer names"));
    assert!(transcript_revision_path(&dir, &session, 1).is_file());
    assert!(!has_valid_transcript_revision(
        &dir,
        &session,
        &manifest.capture_sidecar_sha256,
    ));
    let sessions = list_session_files_from_dir(&dir).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].output_path, sessions[0].source_path);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn replaced_capture_sidecar_preserves_text_but_blocks_transcript_revision() {
    let dir = test_dir("transcript-sidecar-revalidation");
    let session = SessionId::new("s-transcript-sidecar-revalidation").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    let sidecar = dir.join(format!("live-{session}.capture.json"));
    let displaced = sidecar.with_extension("displaced");
    std::fs::rename(&sidecar, displaced).unwrap();
    std::fs::write(&sidecar, b"attacker sidecar").unwrap();

    let saved =
        save_finalized_capture_to_dir(&dir, &live_view(Some("survives"), None), Some(capture))
            .unwrap()
            .unwrap();

    assert_eq!(
        std::fs::read_to_string(dir.join(format!("live-{session}.txt"))).unwrap(),
        "survives\n"
    );
    assert!(saved
        .warning
        .unwrap()
        .contains("Transcript revision was not saved"));
    assert!(!transcript_revision_path(&dir, &session, 1).exists());
    assert!(recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_revisions_are_create_new_and_monotonic() {
    let dir = test_dir("transcript-revisions");
    let session = SessionId::new("s-revisions").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let manifest = recording.finalize().unwrap().committed.unwrap().manifest;
    let text_path = dir.join(format!("live-{session}.txt"));
    let transcript_receipt = write_new_text_file(&text_path, "first\n").unwrap();

    write_transcript_revision(
        &dir,
        &manifest.session_id,
        &manifest.capture_sidecar_sha256,
        &transcript_receipt,
        "first",
        ResultStatus::Complete,
    )
    .unwrap();
    write_transcript_revision(
        &dir,
        &manifest.session_id,
        &manifest.capture_sidecar_sha256,
        &transcript_receipt,
        "second",
        ResultStatus::Complete,
    )
    .unwrap();

    assert!(transcript_revision_path(&dir, &session, 1).is_file());
    assert!(transcript_revision_path(&dir, &session, 2).is_file());
    let revision = std::fs::read_to_string(transcript_revision_path(&dir, &session, 1)).unwrap();
    let revision: serde_json::Value = serde_json::from_str(&revision).unwrap();
    assert_eq!(revision["textFile"], format!("live-{session}.txt"));
    assert_eq!(revision["textSha256"], transcript_receipt.sha256());
    assert_eq!(revision["modelId"], crate::stt::nemotron::MODEL_ID);
    let sessions = list_session_files_from_dir(&dir).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].output_path, text_path.display().to_string());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn highest_corrupt_revision_does_not_fall_back_to_a_valid_lower_revision() {
    let dir = test_dir("highest-corrupt-revision");
    let session = SessionId::new("s-highest-corrupt-revision").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let manifest = recording.finalize().unwrap().committed.unwrap().manifest;
    let transcript = dir.join(format!("live-{session}.txt"));
    let receipt = write_new_text_file(&transcript, "first\n").unwrap();
    write_transcript_revision(
        &dir,
        &session,
        &manifest.capture_sidecar_sha256,
        &receipt,
        "first",
        ResultStatus::Complete,
    )
    .unwrap();
    write_transcript_revision(
        &dir,
        &session,
        &manifest.capture_sidecar_sha256,
        &receipt,
        "second",
        ResultStatus::Complete,
    )
    .unwrap();
    std::fs::write(transcript_revision_path(&dir, &session, 2), "tampered").unwrap();

    assert!(!has_valid_transcript_revision(
        &dir,
        &session,
        &manifest.capture_sidecar_sha256,
    ));
    let saved = list_session_files_from_dir(&dir).unwrap().pop().unwrap();
    assert_eq!(saved.output_path, saved.source_path);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn expired_live_meeting_is_deleted_but_future_and_non_live_origins_survive() {
    let dir = test_dir("meeting-retention");
    let start = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
    let make = |id: &str, origin: SessionOrigin, expiry: u64| {
        let session = SessionId::new(id).unwrap();
        let metadata = SessionMetadata::new(
            session.clone(),
            SessionMode::Meeting,
            origin,
            TriggerMode::Toggle,
            start,
            None,
            None,
            None,
            Vec::new(),
            Some(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(expiry)),
        )
        .unwrap();
        let mut recording =
            StreamingRecording::create_with_session_metadata(&dir, metadata).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap().committed.unwrap().manifest
    };
    let expired = make("s-expired-meeting", SessionOrigin::LiveCapture, 1_010);
    let future = make("s-future-meeting", SessionOrigin::LiveCapture, 2_000);
    let imported = make("s-imported-meeting", SessionOrigin::ImportedFile, 1_010);
    let now = OffsetDateTime::from_unix_timestamp(1_020).unwrap();

    let sessions = list_session_files_from_dir_at(&dir, now).unwrap();
    assert!(!dir
        .join(format!("live-{}.commit.json", expired.session_id))
        .exists());
    assert!(dir
        .join(format!("live-{}.commit.json", future.session_id))
        .exists());
    assert!(dir
        .join(format!("live-{}.commit.json", imported.session_id))
        .exists());
    assert_eq!(sessions.len(), 2);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn expired_meeting_with_an_incomplete_transcript_chain_is_retained() {
    let dir = test_dir("meeting-retention-incomplete-transcript");
    let session = SessionId::new("s-expired-incomplete-transcript").unwrap();
    let start = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
    let metadata = SessionMetadata::new(
        session.clone(),
        SessionMode::Meeting,
        SessionOrigin::LiveCapture,
        TriggerMode::Toggle,
        start,
        None,
        None,
        None,
        Vec::new(),
        Some(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_010)),
    )
    .unwrap();
    let mut recording = StreamingRecording::create_with_session_metadata(&dir, metadata).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));
    std::fs::write(&transcript, "unbound transcript\n").unwrap();

    let now = OffsetDateTime::from_unix_timestamp(1_020).unwrap();
    let saved = list_session_files_from_dir_at(&dir, now).unwrap();

    assert_eq!(saved.len(), 1);
    assert!(saved[0]
        .warning
        .as_deref()
        .unwrap()
        .contains("cleanup is pending"));
    assert!(dir.join(format!("live-{session}.wav")).is_file());
    assert!(dir.join(format!("live-{session}.capture.json")).is_file());
    assert!(dir.join(format!("live-{session}.commit.json")).is_file());
    assert!(transcript.is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn manual_saved_session_deletion_removes_bound_artifacts_and_intent() {
    let dir = test_dir("manual-session-deletion");
    let session = SessionId::new("s-manual-delete").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let journal = std::fs::read(recording.journal_path_for_test()).unwrap();
    let capture = recording.finalize().unwrap();
    let journal_name = format!("live-{session}.capture.journal.part");
    std::fs::write(dir.join(&journal_name), journal).unwrap();
    save_finalized_capture_to_dir(&dir, &live_view(Some("delete me"), None), Some(capture))
        .unwrap();
    let polished = dir.join(format!("live-{session}.polished.txt"));
    std::fs::write(&polished, "polished\n").unwrap();

    let saved = list_session_files_from_dir(&dir).unwrap().pop().unwrap();
    delete_saved_live_session_in_dir(
        &dir,
        session.to_string(),
        saved.output_path,
        saved.capture_commit_path.unwrap(),
    )
    .unwrap();

    for name in [
        format!("live-{session}.wav"),
        format!("live-{session}.capture.json"),
        format!("live-{session}.txt"),
        format!("live-{session}.transcript.r1.json"),
        format!("live-{session}.polished.txt"),
        journal_name,
        format!("live-{session}.commit.json"),
        format!("live-{session}.deletion.v1.json"),
    ] {
        assert!(!dir.join(name).exists());
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn saved_delete_rejects_mismatched_expected_paths_without_mutation() {
    let dir = test_dir("manual-session-identity-mismatch");
    let session = SessionId::new("s-manual-identity-mismatch").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    save_finalized_capture_to_dir(&dir, &live_view(Some("keep me"), None), Some(capture)).unwrap();
    let saved = list_session_files_from_dir(&dir).unwrap().pop().unwrap();

    assert!(delete_saved_live_session_in_dir(
        &dir,
        session.to_string(),
        saved.source_path.clone(),
        saved.capture_commit_path.clone().unwrap(),
    )
    .is_err());
    assert!(delete_saved_live_session_in_dir(
        &dir,
        session.to_string(),
        saved.output_path.clone(),
        saved.output_path,
    )
    .is_err());

    assert!(dir.join(format!("live-{session}.wav")).is_file());
    assert!(dir.join(format!("live-{session}.commit.json")).is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn pending_deletion_resumes_after_audio_was_removed_before_a_crash() {
    let dir = test_dir("resume-deletion-after-audio");
    let session = SessionId::new("s-resume-delete").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    save_finalized_capture_to_dir(&dir, &live_view(Some("resume me"), None), Some(capture))
        .unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
    let intent_name = deletion_intent_name(&session);
    write_deletion_intent(&dir.join(&intent_name), &intent).unwrap();
    recording::remove_regular_artifact_if_hash(
        &dir,
        &intent.artifacts[0].name,
        &intent.artifacts[0].sha256,
    )
    .unwrap();

    assert!(list_session_files_from_dir(&dir).unwrap().is_empty());
    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    assert!(!dir.join(intent_name).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn hash_mismatched_replacement_is_preserved_and_keeps_deletion_intent() {
    let dir = test_dir("deletion-replacement");
    let session = SessionId::new("s-replacement-delete").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
    let intent_name = deletion_intent_name(&session);
    write_deletion_intent(&dir.join(&intent_name), &intent).unwrap();
    let audio = &intent.artifacts[0];
    recording::remove_regular_artifact_if_hash(&dir, &audio.name, &audio.sha256).unwrap();
    std::fs::write(dir.join(&audio.name), b"replacement").unwrap();

    assert!(resume_deletion_intent(&dir, &intent_name).is_err());
    assert_eq!(
        std::fs::read(dir.join(&audio.name)).unwrap(),
        b"replacement"
    );
    assert!(dir.join(intent_name).is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn identity_free_legacy_intent_never_deletes_a_same_content_replacement() {
    let dir = test_dir("legacy-intent-same-content-replacement");
    let session = SessionId::new("s-legacy-intent-same-content-replacement").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let mut intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
    let audio_name = intent
        .artifacts
        .iter()
        .find(|artifact| artifact.name.ends_with(".wav"))
        .unwrap()
        .name
        .clone();
    let original_audio = std::fs::read(dir.join(&audio_name)).unwrap();
    intent.commit_file_identity = None;
    for artifact in &mut intent.artifacts {
        artifact.file_identity = None;
    }
    let intent_name = deletion_intent_name(&session);
    std::fs::write(
        dir.join(&intent_name),
        format!("{}\n", serde_json::to_string(&intent).unwrap()),
    )
    .unwrap();

    std::fs::remove_file(dir.join(&audio_name)).unwrap();
    std::fs::write(dir.join(&audio_name), &original_audio).unwrap();

    for _ in 0..2 {
        let warnings = reconcile_pending_deletion_intents(&dir);
        assert!(warnings
            .session_warnings
            .get(session.as_str())
            .is_some_and(|warning| warning.contains("identity")));
    }

    assert_eq!(
        std::fs::read(dir.join(&audio_name)).unwrap(),
        original_audio
    );
    assert!(dir.join(&intent.commit_file).is_file());
    assert!(dir.join(intent_name).is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn deletion_intent_validation_rejects_partial_and_missing_identity_evidence() {
    let dir = test_dir("deletion-intent-identity-shape");
    let session = SessionId::new("s-deletion-intent-identity-shape").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();

    let mut missing_commit_identity = intent.clone();
    missing_commit_identity.commit_file_identity = None;
    assert!(validate_deletion_intent(&missing_commit_identity)
        .unwrap_err()
        .contains("identity"));

    let mut missing_artifact_identity = intent.clone();
    missing_artifact_identity.artifacts[0].file_identity = None;
    assert!(validate_deletion_intent(&missing_artifact_identity)
        .unwrap_err()
        .contains("identity"));

    let mut missing_all_identities = intent;
    missing_all_identities.commit_file_identity = None;
    for artifact in &mut missing_all_identities.artifacts {
        artifact.file_identity = None;
    }
    assert!(validate_deletion_intent(&missing_all_identities)
        .unwrap_err()
        .contains("identity"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn corrupt_final_intent_is_quarantined_only_before_deletion_has_started() {
    let dir = test_dir("corrupt-intent-recovery");
    let session = SessionId::new("s-corrupt-intent-recovery").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let intent_name = deletion_intent_name(&session);
    std::fs::write(dir.join(&intent_name), b"{\"truncated\"").unwrap();

    delete_saved_session_for_test(&dir, &session).unwrap();

    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    assert!(!dir.join(&intent_name).exists());
    assert!(!std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .contains("deletion.v1.json.delete-")));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn reconciliation_collects_only_old_foreign_private_deletion_leftovers() {
    let dir = test_dir("private-deletion-leftovers");
    let stale_staging = dir.join(".live-s-stale-leftover.deletion.v1.999999-0.part");
    let stale_quarantine = dir.join(".live-s-stale-leftover.deletion.v1.json.delete-999999-0");
    let active_staging = dir.join(format!(
        ".live-s-active-leftover.deletion.v1.{}-0.part",
        std::process::id()
    ));
    let unknown = dir.join(".live-s-unknown-leftover.deletion.v1.invalid.part");
    for path in [&stale_staging, &stale_quarantine, &active_staging, &unknown] {
        std::fs::write(path, b"leftover").unwrap();
        set_old_modified_time(path);
    }

    let catalog = list_session_catalog_from_dir(&dir).unwrap();

    assert!(!stale_staging.exists());
    assert!(!stale_quarantine.exists());
    assert!(active_staging.is_file());
    assert!(unknown.is_file());
    assert!(catalog
        .maintenance_warnings
        .iter()
        .any(|warning| warning.contains("Unknown private deletion artifact")));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn reconciliation_collects_old_generic_artifact_quarantines_and_retains_invalid_evidence() {
    let dir = test_dir("generic-private-deletion-leftovers");
    let stale = [
        ".live-s-generic-cleanup.wav.delete-999999-0",
        ".live-s-generic-cleanup.capture.json.delete-999999-1",
        ".live-s-generic-cleanup.txt.delete-999999-2",
        ".live-s-generic-cleanup.transcript.r1.json.delete-999999-3",
        ".live-s-generic-cleanup.commit.json.delete-999999-4",
        ".live-s-generic-cleanup.capture.journal.part.delete-999999-5",
        ".live-s-generic-cleanup.deletion.v1.json.delete-999999-6",
    ];
    for name in stale {
        let path = dir.join(name);
        std::fs::write(&path, b"leftover").unwrap();
        set_old_modified_time(&path);
    }
    let nested = dir.join("..live-s-generic-cleanup.wav.delete-999999-0.delete-999999-7");
    let malformed = dir.join(".live-s-generic-cleanup.wav.delete-999999-extra-8");
    let active = dir.join(format!(
        ".live-s-generic-cleanup.wav.delete-{}-9",
        std::process::id()
    ));
    let recent = dir.join(".live-s-generic-cleanup.capture.json.delete-999999-10");
    let nonregular = dir.join(".live-s-generic-cleanup.txt.delete-999999-11");
    for path in [&nested, &malformed] {
        std::fs::write(path, b"evidence").unwrap();
        set_old_modified_time(path);
    }
    std::fs::write(&active, b"active evidence").unwrap();
    set_old_modified_time(&active);
    std::fs::write(&recent, b"recent evidence").unwrap();
    std::fs::create_dir(&nonregular).unwrap();

    let catalog = list_session_catalog_from_dir(&dir).unwrap();

    for name in stale {
        assert!(!dir.join(name).exists(), "{name}");
    }
    assert!(nested.is_file());
    assert!(malformed.is_file());
    assert!(active.is_file());
    assert!(recent.is_file());
    assert!(nonregular.is_dir());
    assert!(catalog
        .maintenance_warnings
        .iter()
        .any(|warning| warning.contains("Unknown private deletion artifact")));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn private_cleanup_filters_before_its_budget_and_progresses_across_batches() {
    let dir = test_dir("private-deletion-cleanup-budget");
    for index in 0..256 {
        std::fs::write(dir.join(format!("unrelated-{index:03}.tmp")), b"keep").unwrap();
    }
    let leftovers = (1..=129)
        .map(|revision| {
            format!(".live-s-cleanup-budget.transcript.r{revision}.json.delete-999999-{revision}")
        })
        .collect::<Vec<_>>();
    for name in &leftovers {
        let path = dir.join(name);
        std::fs::write(&path, b"leftover").unwrap();
        set_old_modified_time(&path);
    }

    list_session_catalog_from_dir(&dir).unwrap();
    assert_eq!(
        leftovers
            .iter()
            .filter(|name| dir.join(name).exists())
            .count(),
        1
    );
    assert!(dir.join("unrelated-000.tmp").is_file());

    list_session_catalog_from_dir(&dir).unwrap();
    assert!(leftovers.iter().all(|name| !dir.join(name).exists()));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn private_cleanup_rotation_advances_past_a_failed_full_batch() {
    let names = (0..=MAX_PRIVATE_DELETION_LEFTOVERS)
        .map(|index| format!("candidate-{index:03}"))
        .collect::<Vec<_>>();

    let mut first = RotatingDeletionCandidates::new(None, MAX_PRIVATE_DELETION_LEFTOVERS);
    for name in &names {
        first.push(name.clone());
    }
    let (first_batch, _, cursor) = first.finish();
    assert_eq!(first_batch.len(), MAX_PRIVATE_DELETION_LEFTOVERS);
    assert!(!first_batch.contains(names.last().unwrap()));

    let mut second = RotatingDeletionCandidates::new(cursor, MAX_PRIVATE_DELETION_LEFTOVERS);
    for name in &names {
        second.push(name.clone());
    }
    let (second_batch, _, _) = second.finish();

    assert!(second_batch.contains(names.last().unwrap()));
}

#[test]
fn pending_intent_reconciliation_rotates_past_a_failed_full_batch() {
    let dir = test_dir("pending-intent-rotation");
    for index in 0..MAX_PRIVATE_DELETION_LEFTOVERS {
        let session = SessionId::new(format!("s-pending-intent-{index:03}")).unwrap();
        let audio = format!("live-{session}.wav");
        let intent = DeletionIntent {
            schema_version: DELETION_INTENT_SCHEMA_VERSION,
            session_id: session.clone(),
            reason: "manual".into(),
            commit_file: format!("live-{session}.commit.json"),
            commit_sha256: "0".repeat(64),
            commit_file_identity: None,
            artifacts: vec![DeletionArtifact {
                name: audio.clone(),
                sha256: "0".repeat(64),
                file_identity: None,
            }],
        };
        std::fs::write(dir.join(audio), b"retained evidence").unwrap();
        std::fs::write(
            dir.join(deletion_intent_name(&session)),
            format!("{}\n", serde_json::to_string(&intent).unwrap()),
        )
        .unwrap();
    }
    let session = SessionId::new("s-pending-intent-999").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
    let intent_name = deletion_intent_name(&session);
    write_deletion_intent(&dir.join(&intent_name), &intent).unwrap();

    reconcile_pending_deletion_intents(&dir);
    assert!(dir.join(&intent_name).is_file());
    reconcile_pending_deletion_intents(&dir);

    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    assert!(!dir.join(intent_name).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn deletion_intent_resume_waits_for_an_existing_owner() {
    let dir = test_dir("deletion-intent-resume-ownership");
    let session = SessionId::new("s-deletion-intent-resume-ownership").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
    let intent_name = deletion_intent_name(&session);
    write_deletion_intent(&dir.join(&intent_name), &intent).unwrap();

    let (owner_ready_tx, owner_ready_rx) = std::sync::mpsc::channel();
    let (release_owner_tx, release_owner_rx) = std::sync::mpsc::channel();
    let owner = std::thread::spawn(move || {
        let _ownership = session_mutation_ownership();
        owner_ready_tx.send(()).unwrap();
        release_owner_rx.recv().unwrap();
    });
    owner_ready_rx.recv().unwrap();

    let resume_dir = dir.clone();
    let resume_name = intent_name.clone();
    let (resumed_tx, resumed_rx) = std::sync::mpsc::channel();
    let resume = std::thread::spawn(move || {
        resumed_tx
            .send(resume_deletion_intent(&resume_dir, &resume_name))
            .unwrap();
    });

    assert!(resumed_rx.recv_timeout(Duration::from_millis(100)).is_err());
    release_owner_tx.send(()).unwrap();
    owner.join().unwrap();
    assert!(resumed_rx.recv().unwrap().is_ok());
    resume.join().unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn catalog_reconciliation_cannot_resume_a_manual_deletion_after_publication() {
    let dir = test_dir("manual-deletion-catalog-ownership");
    let session = SessionId::new("s-manual-deletion-catalog-ownership").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent_name = deletion_intent_name(&session);

    let (published_tx, published_rx) = std::sync::mpsc::channel();
    let (release_manual_tx, release_manual_rx) = std::sync::mpsc::channel();
    let manual_dir = dir.clone();
    let manual = std::thread::spawn(move || {
        delete_committed_session_in_dir_with_publication_barrier(
            &manual_dir,
            &capture,
            "manual",
            move |published| {
                if published {
                    published_tx.send(()).unwrap();
                    release_manual_rx.recv().unwrap();
                }
            },
        )
    });
    published_rx.recv().unwrap();
    assert!(dir.join(&intent_name).is_file());

    let catalog_dir = dir.clone();
    let (catalog_started_tx, catalog_started_rx) = std::sync::mpsc::channel();
    let (catalog_finished_tx, catalog_finished_rx) = std::sync::mpsc::channel();
    let catalog = std::thread::spawn(move || {
        catalog_started_tx.send(()).unwrap();
        catalog_finished_tx
            .send(list_session_catalog_from_dir(&catalog_dir))
            .unwrap();
    });
    catalog_started_rx.recv().unwrap();

    assert!(catalog_finished_rx
        .recv_timeout(Duration::from_millis(100))
        .is_err());
    release_manual_tx.send(()).unwrap();
    assert!(manual.join().unwrap().is_ok());
    assert!(catalog_finished_rx.recv().unwrap().is_ok());
    catalog.join().unwrap();
    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    assert!(!dir.join(intent_name).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn corrupt_intent_retries_remove_each_verified_quarantine() {
    let dir = test_dir("corrupt-intent-retry-cleanup");
    let session = SessionId::new("s-corrupt-intent-retry-cleanup").unwrap();
    let mut capture = StreamingRecording::create(&dir, session.clone()).unwrap();
    capture.append_pcm16(&[1, 0]).unwrap();
    capture.finalize().unwrap();
    let committed = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &committed, "manual").unwrap();
    let intent_path = dir.join(deletion_intent_name(&session));

    for _ in 0..3 {
        std::fs::write(&intent_path, b"{corrupt").unwrap();
        write_deletion_intent(&intent_path, &intent).unwrap();
        assert!(!std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains("deletion.v1.json.delete-")));
        std::fs::remove_file(&intent_path).unwrap();
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn repeated_post_publication_failures_keep_one_intent_evidence_quarantine() {
    let dir = test_dir("corrupt-intent-post-publication-retries");
    let session = SessionId::new("s-corrupt-intent-post-publication-retries").unwrap();
    let mut capture = StreamingRecording::create(&dir, session.clone()).unwrap();
    capture.append_pcm16(&[1, 0]).unwrap();
    capture.finalize().unwrap();
    let committed = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &committed, "manual").unwrap();
    let intent_path = dir.join(deletion_intent_name(&session));

    for _ in 0..3 {
        std::fs::write(&intent_path, b"{corrupt").unwrap();
        let replacement = intent_path.clone();
        assert!(write_deletion_intent_with_publication_barrier(
            &intent_path,
            &intent,
            move |published| {
                if published {
                    std::fs::remove_file(&replacement).unwrap();
                    std::fs::write(&replacement, b"replacement intent").unwrap();
                }
            }
        )
        .is_err());
        assert_eq!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .contains("deletion.v1.json.delete-")
                })
                .count(),
            1
        );
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn missing_intent_restores_the_newest_verified_quarantine_before_replacement() {
    let dir = test_dir("missing-intent-quarantine-recovery");
    let session = SessionId::new("s-missing-intent-quarantine-recovery").unwrap();
    let intent_name = deletion_intent_name(&session);
    let older = format!(".{intent_name}.delete-999999-1");
    let newer = format!(".{intent_name}.delete-999999-2");
    std::fs::write(dir.join(&older), b"{older").unwrap();
    std::fs::write(dir.join(&newer), b"{newer").unwrap();
    set_old_modified_time(&dir.join(&older));
    set_old_modified_time(&dir.join(&newer));

    reconcile_intent_evidence_quarantines(&dir, &intent_name).unwrap();

    assert_eq!(std::fs::read(dir.join(&intent_name)).unwrap(), b"{newer");
    assert!(!dir.join(&older).exists());
    assert!(!dir.join(&newer).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn fresh_foreign_intent_quarantine_is_retained_during_reconciliation() {
    let dir = test_dir("fresh-foreign-intent-quarantine");
    let session = SessionId::new("s-fresh-foreign-intent-quarantine").unwrap();
    let intent_name = deletion_intent_name(&session);
    let quarantine = format!(".{intent_name}.delete-999999-0");
    std::fs::write(dir.join(&quarantine), b"foreign in-flight intent").unwrap();

    reconcile_intent_evidence_quarantines(&dir, &intent_name).unwrap();

    assert!(!dir.join(&intent_name).exists());
    assert!(dir.join(&quarantine).is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn current_pid_intent_quarantine_is_reconciled_during_catalog_maintenance() {
    let dir = test_dir("current-pid-intent-quarantine");
    let session = SessionId::new("s-current-pid-intent-quarantine").unwrap();
    let intent_name = deletion_intent_name(&session);
    let quarantine = format!(".{intent_name}.delete-{}-0", std::process::id());
    std::fs::write(dir.join(&quarantine), b"prior failed intent").unwrap();

    let catalog = list_session_catalog_from_dir(&dir).unwrap();

    assert!(dir.join(&intent_name).is_file());
    assert!(!dir.join(&quarantine).exists());
    assert!(catalog
        .maintenance_warnings
        .iter()
        .any(|warning| warning.contains("pending")));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn damaged_commit_warnings_take_priority_when_maintenance_warning_cap_is_full() {
    let dir = test_dir("damaged-warning-priority");
    let session = SessionId::new("s-damaged-warning-priority").unwrap();
    let mut capture = StreamingRecording::create(&dir, session.clone()).unwrap();
    capture.append_pcm16(&[1, 0]).unwrap();
    capture.finalize().unwrap();
    std::fs::write(dir.join(format!("live-{session}.commit.json")), b"{damaged").unwrap();
    for index in 0..MAX_MAINTENANCE_WARNINGS {
        std::fs::write(
            dir.join(format!(".live-s-warning-{index}.deletion.v1.invalid.part")),
            b"evidence",
        )
        .unwrap();
    }

    let catalog = list_session_catalog_from_dir(&dir).unwrap();

    assert_eq!(catalog.maintenance_warnings.len(), MAX_MAINTENANCE_WARNINGS);
    assert!(catalog.maintenance_warnings[0].contains("Damaged live recording"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn deletion_intent_publication_rejects_injected_pre_and_post_publication_replacements() {
    let dir = test_dir("intent-publication-barriers");
    let session = SessionId::new("s-intent-publication-barriers").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
    let intent_path = dir.join(deletion_intent_name(&session));

    let before = intent_path.clone();
    assert!(
        write_deletion_intent_with_publication_barrier(&intent_path, &intent, move |after| {
            if !after {
                std::fs::write(&before, b"competing intent").unwrap();
            }
        })
        .is_err()
    );
    assert_eq!(std::fs::read(&intent_path).unwrap(), b"competing intent");
    std::fs::remove_file(&intent_path).unwrap();

    let after = intent_path.clone();
    assert!(write_deletion_intent_with_publication_barrier(
        &intent_path,
        &intent,
        move |published| {
            if published {
                std::fs::remove_file(&after).unwrap();
                std::fs::write(&after, b"replacement intent").unwrap();
            }
        }
    )
    .is_err());
    assert_eq!(std::fs::read(&intent_path).unwrap(), b"replacement intent");
    assert!(dir.join(format!("live-{session}.commit.json")).is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn concurrent_intent_replacements_do_not_reconcile_an_active_quarantine() {
    let dir = test_dir("concurrent-intent-replacements");
    let session = SessionId::new("s-concurrent-intent-replacements").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
    let intent_path = dir.join(deletion_intent_name(&session));
    std::fs::write(&intent_path, b"{corrupt").unwrap();

    let (first_quarantined_tx, first_quarantined_rx) = std::sync::mpsc::channel();
    let (release_first_tx, release_first_rx) = std::sync::mpsc::channel();
    let first_path = intent_path.clone();
    let first_intent = intent.clone();
    let first = std::thread::spawn(move || {
        write_deletion_intent_with_publication_barrier(
            &first_path,
            &first_intent,
            move |published| {
                if !published {
                    first_quarantined_tx.send(()).unwrap();
                    release_first_rx.recv().unwrap();
                }
            },
        )
    });

    first_quarantined_rx.recv().unwrap();
    assert!(!intent_path.exists());
    assert_eq!(intent_quarantine_count(&dir), 1);

    let (contender_started_tx, contender_started_rx) = std::sync::mpsc::channel();
    let (contender_finished_tx, contender_finished_rx) = std::sync::mpsc::channel();
    let contender_path = intent_path.clone();
    let contender_intent = intent.clone();
    let contender = std::thread::spawn(move || {
        contender_started_tx.send(()).unwrap();
        let result = write_deletion_intent(&contender_path, &contender_intent);
        contender_finished_tx.send(result).unwrap();
    });

    contender_started_rx.recv().unwrap();
    assert!(!intent_path.exists());
    assert_eq!(intent_quarantine_count(&dir), 1);

    release_first_tx.send(()).unwrap();
    assert!(first.join().unwrap().is_ok());
    assert!(contender_finished_rx.recv().unwrap().is_ok());
    contender.join().unwrap();
    assert_eq!(
        std::fs::read_to_string(&intent_path).unwrap(),
        serde_json::to_string(&intent).unwrap() + "\n"
    );
    assert_eq!(intent_quarantine_count(&dir), 0);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn truncated_final_intent_after_progress_is_retained_as_a_catalog_warning() {
    let dir = test_dir("truncated-intent-after-progress");
    let session = SessionId::new("s-truncated-intent-after-progress").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    let capture = recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .pop()
        .unwrap();
    let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
    let intent_name = deletion_intent_name(&session);
    write_deletion_intent(&dir.join(&intent_name), &intent).unwrap();
    let audio = &intent.artifacts[0];
    recording::remove_regular_artifact_if_hash(&dir, &audio.name, &audio.sha256).unwrap();
    std::fs::write(dir.join(&intent_name), b"{\"truncated\"").unwrap();

    let catalog = list_session_catalog_from_dir(&dir).unwrap();

    assert!(catalog.sessions.is_empty());
    assert!(catalog
        .maintenance_warnings
        .iter()
        .any(|warning| warning.contains("pending")));
    assert!(dir.join(&intent_name).is_file());
    assert!(dir.join(format!("live-{session}.capture.json")).is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn forged_deletion_intent_cannot_name_another_session_or_arbitrary_file() {
    let dir = test_dir("forged-deletion-intent");
    let session = SessionId::new("s-forged-intent").unwrap();
    let arbitrary = dir.join("keep-me.txt");
    std::fs::write(&arbitrary, "keep").unwrap();
    let intent_name = deletion_intent_name(&session);
    let forged = DeletionIntent {
        schema_version: DELETION_INTENT_SCHEMA_VERSION,
        session_id: session,
        reason: "manual".into(),
        commit_file: "live-s-forged-intent.commit.json".into(),
        commit_sha256: "0".repeat(64),
        commit_file_identity: None,
        artifacts: vec![
            DeletionArtifact {
                name: "live-s-forged-intent.wav".into(),
                sha256: "0".repeat(64),
                file_identity: None,
            },
            DeletionArtifact {
                name: "live-s-forged-intent.capture.json".into(),
                sha256: "0".repeat(64),
                file_identity: None,
            },
            DeletionArtifact {
                name: "keep-me.txt".into(),
                sha256: recording::sha256_file(&arbitrary).unwrap(),
                file_identity: None,
            },
        ],
    };
    std::fs::write(
        dir.join(&intent_name),
        format!("{}\n", serde_json::to_string(&forged).unwrap()),
    )
    .unwrap();

    let warnings = reconcile_pending_deletion_intents(&dir);

    assert!(arbitrary.is_file());
    assert!(dir.join(intent_name).is_file());
    assert!(warnings.session_warnings.contains_key("s-forged-intent"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn audio_only_committed_session_can_be_deleted() {
    let dir = test_dir("audio-only-delete");
    let session = SessionId::new("s-audio-only-delete").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();

    delete_saved_session_for_test(&dir, &session).unwrap();

    assert!(!dir.join(format!("live-{session}.wav")).exists());
    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn canonical_paths_accept_only_the_hash_valid_committed_audio_and_transcript() {
    let dir = test_dir("canonical-paths");
    let session = SessionId::new("s-canonical-paths").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    save_finalized_capture_to_dir(
        &dir,
        &live_view(Some("canonical text"), None),
        Some(capture),
    )
    .unwrap();
    let audio = dir.join(format!("live-{session}.wav"));
    let transcript = dir.join(format!("live-{session}.txt"));

    assert_eq!(
        canonical_committed_live_path_from_dir(&audio, &dir, false).unwrap(),
        audio.canonicalize().unwrap()
    );
    assert_eq!(
        canonical_committed_live_path_from_dir(&transcript, &dir, true).unwrap(),
        transcript.canonicalize().unwrap()
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn dictation_without_retention_never_creates_a_deletion_intent() {
    let dir = test_dir("dictation-no-retention");
    let session = SessionId::new("s-dictation-no-retention").unwrap();
    let metadata = SessionMetadata::new(
        session.clone(),
        SessionMode::Dictation,
        SessionOrigin::LiveCapture,
        TriggerMode::Toggle,
        std::time::SystemTime::UNIX_EPOCH,
        None,
        None,
        None,
        Vec::new(),
        None,
    )
    .unwrap();
    let mut recording = StreamingRecording::create_with_session_metadata(&dir, metadata).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();

    list_session_files_from_dir_at(&dir, OffsetDateTime::now_utc()).unwrap();

    assert!(dir.join(format!("live-{session}.commit.json")).is_file());
    assert!(!dir.join(deletion_intent_name(&session)).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn normal_history_scan_ignores_timestamp_transcripts_and_leaves_them_untouched() {
    let dir = std::env::temp_dir().join(format!("yap-live-primary-scan-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in [
        "live-205.txt",
        "live-205-1.txt",
        "live-205.polished.txt",
        "live-not-a-time.txt",
        "live-205-extra-part.txt",
    ] {
        std::fs::write(dir.join(name), "hello\n").unwrap();
    }

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert!(sessions.is_empty());
    assert_eq!(
        std::fs::read_to_string(dir.join("live-205.txt")).unwrap(),
        "hello\n"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn normal_history_scan_ignores_uncommitted_directory_shaped_entries() {
    let dir = std::env::temp_dir().join(format!("yap-live-dir-scan-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let transcript_dir = dir.join("live-203.txt");
    let transcript = dir.join("live-204.txt");
    let audio_dir = dir.join("live-204.wav");
    std::fs::create_dir_all(&transcript_dir).unwrap();
    std::fs::write(&transcript, "hello\n").unwrap();
    std::fs::create_dir_all(&audio_dir).unwrap();

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert!(sessions.is_empty());
    assert_eq!(std::fs::read_to_string(&transcript).unwrap(), "hello\n");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn saved_live_session_scan_does_not_rewrite_streaming_artifacts() {
    let dir = std::env::temp_dir().join(format!("yap-live-clean-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = dir.join("live-201.txt");
    std::fs::write(&transcript, "  THank   you.. \n").unwrap();

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert!(sessions.is_empty());
    assert_eq!(
        std::fs::read_to_string(&transcript).unwrap(),
        "  THank   you.. \n"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn saved_live_session_scan_does_not_rewrite_old_empty_placeholder() {
    let dir = std::env::temp_dir().join(format!("yap-live-placeholder-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = dir.join("live-202.txt");
    std::fs::write(&transcript, "No live transcript captured.\n").unwrap();

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert!(sessions.is_empty());
    assert_eq!(
        std::fs::read_to_string(&transcript).unwrap(),
        "No live transcript captured.\n"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recordings_dir_uses_absolute_override_or_app_data() {
    let override_dir = std::env::temp_dir().join("custom-live-recordings");
    assert_eq!(
        recordings_dir_from(
            |key| (key == "YAP_LIVE_RECORDINGS_DIR").then(|| override_dir.display().to_string())
        ),
        override_dir
    );

    let local = std::env::temp_dir().join("local-data");
    assert_eq!(
        recordings_dir_from(|key| match key {
            "YAP_LIVE_RECORDINGS_DIR" => Some("relative-live-recordings".into()),
            "YAP_APP_DATA_DIR" => Some(local.display().to_string()),
            _ => None,
        }),
        local.join("live-recordings")
    );
}

#[test]
fn write_new_text_file_does_not_scan_partial_transcripts() {
    let dir = std::env::temp_dir().join(format!("yap-live-text-partial-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = dir.join("live-77.txt");
    let partial = partial_text_path(&transcript).unwrap();
    std::fs::write(&partial, "stale").unwrap();

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert!(sessions.is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn normal_history_scan_does_not_use_timestamp_filenames_as_history_metadata() {
    let dir = std::env::temp_dir().join(format!("yap-live-timestamp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = dir.join("live-999-1.txt");
    std::fs::write(&transcript, "hello\n").unwrap();

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert!(sessions.is_empty());
    std::fs::remove_dir_all(dir).ok();
}

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
