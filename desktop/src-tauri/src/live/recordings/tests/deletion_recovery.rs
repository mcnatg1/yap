use super::*;

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
