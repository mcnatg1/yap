use super::*;

#[test]
fn successful_commit_removes_its_owned_journal_but_partial_finalization_keeps_it() {
    let complete_dir = tempfile_dir("journal-cleanup-complete");
    let complete_session = SessionId::new("s-journal-cleanup-complete").unwrap();
    let mut complete = StreamingRecording::create(&complete_dir, complete_session.clone()).unwrap();
    complete.append_pcm16(&[1, 0]).unwrap();
    assert_eq!(complete.finalize().unwrap().status, CaptureStatus::Complete);
    assert!(!complete_dir
        .join(format!("live-{complete_session}.capture.journal.part"))
        .exists());

    let partial_dir = tempfile_dir("journal-cleanup-partial");
    let partial_session = SessionId::new("s-journal-cleanup-partial").unwrap();
    let mut partial = StreamingRecording::create_with_fault(
        &partial_dir,
        partial_session.clone(),
        CommitFaultPoint::CommitRename,
    )
    .unwrap();
    partial.append_pcm16(&[1, 0]).unwrap();
    assert_eq!(partial.finalize().unwrap().status, CaptureStatus::Partial);
    assert!(partial_dir
        .join(format!("live-{partial_session}.capture.journal.part"))
        .is_file());
    std::fs::remove_dir_all(complete_dir).ok();
    std::fs::remove_dir_all(partial_dir).ok();
}

#[test]
fn valid_committed_session_suppresses_a_crash_residue_journal_from_recovery() {
    let dir = tempfile_dir("journal-committed-residue");
    let session = SessionId::new("s-journal-committed-residue").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let journal = std::fs::read(&recording.paths.journal_part).unwrap();
    assert_eq!(
        recording.finalize().unwrap().status,
        CaptureStatus::Complete
    );
    std::fs::write(
        dir.join(format!("live-{session}.capture.journal.part")),
        journal,
    )
    .unwrap();

    let scan = scan_recordings(&dir).unwrap();

    assert_eq!(scan.complete.len(), 1);
    assert!(scan.partial.is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn lone_wav_part_is_a_partial_candidate_without_inventing_metadata() {
    let dir = tempfile_dir("lone-wav-part");
    let session = SessionId::new("s-lone-wav-part").unwrap();
    std::fs::write(dir.join(format!("live-{session}.wav.part")), b"RIFF").unwrap();

    let scan = scan_recordings(&dir).unwrap();

    assert!(scan.complete.is_empty());
    assert_eq!(scan.partial.len(), 1);
    assert_eq!(scan.partial[0].session_id.as_ref(), Some(&session));
    assert_eq!(scan.partial[0].directory, dir);
}

#[test]
fn scanner_ignores_malformed_or_unknown_partial_artifacts() {
    let dir = tempfile_dir("malformed-partials");
    for name in [
        "live-.wav.part",
        "live-not a session.wav.part",
        "live-s-known.wav.partial",
        "capture.wav.part",
    ] {
        std::fs::write(dir.join(name), b"partial").unwrap();
    }

    assert!(scan_recordings(&dir).unwrap().is_empty());
}

#[test]
fn alternating_four_hour_gaps_have_bounded_journal_memory_and_snapshot_size() {
    const FOUR_HOURS_AT_TEN_HZ: u64 = 4 * 60 * 60 * 10;
    const MAX_JOURNAL_BYTES: u64 = 128 * 1024;

    let dir = tempfile_dir("journal-alternating-gaps");
    let session = SessionId::new("s-journal-alternating-gaps").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let mut recording = StreamingRecording::create(&dir, session).unwrap();
    recording
        .append_input(recording_revision(&track, 1, 0, 16_000, 0))
        .unwrap();
    let mut terminal_error = None;
    for sequence in (0..FOUR_HOURS_AT_TEN_HZ * 2).step_by(2) {
        recording.observe_frame_metadata("live-microphone", 16_000, 1, sequence, 0, 100);
        if sequence % 20_000 == 0 {
            if let Err(error) = recording.persist_journal_for_test() {
                terminal_error = Some(error);
                break;
            }
        }
    }
    if terminal_error.is_none() {
        terminal_error = recording.persist_journal_for_test().err();
    }

    assert!(terminal_error.is_some());
    assert_eq!(
        recording.journal_for_test().sequence_gaps.len(),
        MAX_SEQUENCE_GAP_DETAILS
    );
    assert!(recording.journal_for_test().sequence_gap_overflow.is_some());
    assert!(
        std::fs::metadata(&recording.paths.journal_part)
            .unwrap()
            .len()
            <= MAX_JOURNAL_BYTES
    );
    let recovered = read_journal_snapshot(&recording.paths.journal_part).unwrap();
    assert!(!recovered.sequence_coverage.is_empty());
    assert!(
        recovered.sequence_coverage[0].last_sequence <= FOUR_HOURS_AT_TEN_HZ * 2 - 2,
        "a bounded append journal may retain a valid prefix once it reaches its terminal marker"
    );
}

#[test]
fn journal_replay_keeps_an_earlier_multitrack_gap_replacement() {
    let dir = tempfile_dir("journal-multitrack-gap-replacement");
    let session = SessionId::new("s-journal-multitrack-gap-replacement").unwrap();
    let microphone = TrackId::new("microphone").unwrap();
    let loopback = TrackId::new("system-loopback").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    for track in [&microphone, &loopback] {
        recording
            .append_input(recording_revision(track, 1, 0, 16_000, 0))
            .unwrap();
    }
    recording
        .append_input(RecordingInput::Gap(AudioGap {
            session_id: session.clone(),
            track_id: microphone.clone(),
            start_ms: 0,
            duration_ms: 10,
            source_position_frames: 0,
            dropped_frames: 160,
            cause: GapCause::CallbackPoolExhausted,
            generation: 1,
        }))
        .unwrap();
    recording
        .append_input(RecordingInput::Gap(AudioGap {
            session_id: session.clone(),
            track_id: loopback,
            start_ms: 0,
            duration_ms: 10,
            source_position_frames: 0,
            dropped_frames: 160,
            cause: GapCause::DeviceDiscontinuity,
            generation: 2,
        }))
        .unwrap();
    recording
        .append_input(RecordingInput::Gap(AudioGap {
            session_id: session,
            track_id: microphone.clone(),
            start_ms: 0,
            duration_ms: 20,
            source_position_frames: 0,
            dropped_frames: 320,
            cause: GapCause::CallbackPoolExhausted,
            generation: 3,
        }))
        .unwrap();

    let replayed = read_journal_snapshot(&recording.paths.journal_part).unwrap();
    assert_eq!(replayed.timeline_gaps.len(), 2);
    let microphone_gap = replayed
        .timeline_gaps
        .iter()
        .find(|gap| gap.track_id == microphone)
        .unwrap();
    assert_eq!(microphone_gap.duration_ms, 20);
    assert_eq!(microphone_gap.dropped_frames, 320);
    assert_eq!(microphone_gap.generation, 3);
}
