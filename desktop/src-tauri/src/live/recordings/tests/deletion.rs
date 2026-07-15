use super::*;

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
