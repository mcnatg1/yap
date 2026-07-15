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
