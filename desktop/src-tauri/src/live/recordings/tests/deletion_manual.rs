use super::*;

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
