use super::*;

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
