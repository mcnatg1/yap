use super::*;

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
fn normal_history_scan_does_not_use_timestamp_filenames_as_history_metadata() {
    let dir = std::env::temp_dir().join(format!("yap-live-timestamp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = dir.join("live-999-1.txt");
    std::fs::write(&transcript, "hello\n").unwrap();

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert!(sessions.is_empty());
    std::fs::remove_dir_all(dir).ok();
}
