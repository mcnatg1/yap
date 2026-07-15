use super::*;

#[test]
fn journal_recovers_the_valid_append_prefix_after_a_torn_tail() {
    let dir = tempfile_dir("journal-torn-tail");
    let session = SessionId::new("s-journal-torn-tail").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let mut recording = StreamingRecording::create(&dir, session).unwrap();
    recording
        .append_input(recording_revision(&track, 1, 0, 16_000, 0))
        .unwrap();
    for sequence in 0..=4 {
        recording.observe_frame_metadata(
            "live-microphone",
            16_000,
            1,
            sequence,
            sequence * 100,
            100,
        );
    }
    recording.persist_journal_for_test().unwrap();
    let journal = recording.paths.journal_part.clone();
    drop(recording);
    OpenOptions::new()
        .append(true)
        .open(&journal)
        .unwrap()
        .write_all(b"{\"delta\":")
        .unwrap();

    let recovered = read_journal_snapshot(&journal).unwrap();

    assert_eq!(recovered.sequence_coverage[0].last_sequence, 4);
}

#[test]
fn journal_never_replaces_an_adversarial_path_after_creation() {
    let dir = tempfile_dir("journal-path-replacement");
    let session = SessionId::new("s-journal-path-replacement").unwrap();
    let mut recording = StreamingRecording::create(&dir, session).unwrap();
    let journal = recording.paths.journal_part.clone();
    let displaced = dir.join("displaced-journal");
    fs::rename(&journal, &displaced).unwrap();
    fs::write(&journal, b"attacker replacement").unwrap();
    recording.observe_frame_metadata("live-microphone", 16_000, 1, 1, 0, 100);

    recording.persist_journal_for_test().unwrap();

    assert_eq!(fs::read(&journal).unwrap(), b"attacker replacement");
    assert!(fs::metadata(&displaced).unwrap().len() > 0);
}

#[test]
fn four_hour_journal_growth_stops_at_the_explicit_hard_limit() {
    let dir = tempfile_dir("journal-hard-limit");
    let session = SessionId::new("s-journal-hard-limit").unwrap();
    let mut recording = StreamingRecording::create(&dir, session).unwrap();
    let mut terminal_error = None;
    for second in 0..(4 * 60 * 60) {
        recording.observe_frame_metadata(
            "live-microphone",
            16_000,
            1,
            second * 2,
            second * 1_000,
            100,
        );
        if let Err(error) = recording.persist_journal_for_test() {
            terminal_error = Some(error);
            break;
        }
    }
    let terminal_error = terminal_error.expect("four-hour churn must reach the journal bound");

    let journal = recording.paths.journal_part.clone();
    let bounded = fs::metadata(&journal).unwrap().len();
    let repeated = recording.persist_journal_for_test().unwrap_err();

    assert!(bounded <= MAX_JOURNAL_BYTES);
    assert_eq!(fs::metadata(journal).unwrap().len(), bounded);
    assert!(recording.journal_growth_stopped_for_test());
    assert_eq!(repeated, terminal_error);
}

#[test]
fn exhausted_journal_capacity_cannot_publish_complete_and_remains_recoverable() {
    let dir = tempfile_dir("journal-capacity-terminal");
    let session = SessionId::new("s-journal-capacity-terminal").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording
        .append_input(recording_revision(&track, 1, 0, 16_000, 0))
        .unwrap();
    recording
        .append_input(RecordingInput::PreparedFrame(prepared_frame(&session)))
        .unwrap();
    recording
        .append_input(RecordingInput::Gap(AudioGap {
            session_id: session.clone(),
            track_id: track.clone(),
            start_ms: 10,
            duration_ms: 10,
            source_position_frames: 160,
            dropped_frames: 160,
            cause: GapCause::CallbackPoolExhausted,
            generation: 1,
        }))
        .unwrap();
    recording.journal_bytes = MAX_JOURNAL_BYTES - MAX_JOURNAL_TERMINAL_BYTES;

    let append = recording.append_input(RecordingInput::Gap(AudioGap {
        session_id: session.clone(),
        track_id: track,
        start_ms: 10,
        duration_ms: 20,
        source_position_frames: 160,
        dropped_frames: 320,
        cause: GapCause::CallbackPoolExhausted,
        generation: 2,
    }));
    let result = recording.finalize().unwrap();

    assert!(append.is_err());
    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
    assert!(recording.paths.journal_part.is_file());
    let recovered = read_journal_snapshot(&recording.paths.journal_part).unwrap();
    assert_eq!(recovered.session_id, session);
    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty());
    assert_eq!(scan.partial.len(), 1);
}

#[test]
fn revision_transition_capacity_boundary_keeps_a_replayable_partial_prefix() {
    let dir = tempfile_dir("revision-transition-capacity-boundary");
    let session = SessionId::new("s-revision-transition-capacity-boundary").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording
        .append_input(recording_revision(&track, 1, 0, 16_000, 0))
        .unwrap();
    let transition = revision_transition(&track, 2, 10, 48_000, 480);
    let mut projected = recording.journal.clone();
    projected
        .observe_revision_transition(transition.clone())
        .unwrap();
    let transition_record = JournalRecord::Delta {
        delta: recording.journal_durable.delta(&projected),
    };
    let serialized_transition = serialize_journal_record(&transition_record).unwrap();
    let serialized_transition_text = std::str::from_utf8(&serialized_transition).unwrap();
    assert!(serialized_transition_text.contains("revisionTransitions"));
    assert!(!serialized_transition_text.contains("trackConfigurations"));
    assert!(!serialized_transition_text.contains("clockMappings"));
    let transition_bytes = serialized_transition.len();
    recording.journal_bytes =
        MAX_JOURNAL_BYTES - MAX_JOURNAL_TERMINAL_BYTES - transition_bytes as u64 + 1;

    let append_error = recording
        .append_input(RecordingInput::RevisionTransition(transition))
        .unwrap_err();
    let journal = recording.paths.journal_part.clone();
    drop(recording);
    let recovered = read_journal_snapshot(&journal).unwrap();

    assert!(append_error.contains("journal durability stopped"));
    assert_eq!(recovered.track_configurations.len(), 1);
    assert_eq!(recovered.clock_mappings.len(), 1);
    assert_eq!(recovered.track_configurations[0].revision, 1);
    assert_eq!(recovered.clock_mappings[0].revision, 1);
    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty());
    assert_eq!(scan.partial.len(), 1);
}

#[test]
fn repeated_journal_write_failure_returns_the_cached_terminal_error() {
    let dir = tempfile_dir("journal-write-failure");
    let session = SessionId::new("s-journal-write-failure").unwrap();
    let mut recording =
        StreamingRecording::create_with_fault(&dir, session, CommitFaultPoint::JournalAppend)
            .unwrap();
    let journal = recording.paths.journal_part.clone();
    let initial_len = fs::metadata(&journal).unwrap().len();

    let first = recording.persist_journal_for_test().unwrap_err();
    let second = recording.persist_journal_for_test().unwrap_err();

    assert_eq!(first, second);
    assert_eq!(fs::metadata(journal).unwrap().len(), initial_len);
}
