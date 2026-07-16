use super::*;

#[test]
fn worker_caches_the_first_append_failure_while_draining_later_frames() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let dir = tempfile_dir("worker-terminal-append-failure");
    let session = SessionId::new("s-worker-terminal-append-failure").unwrap();
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 64);
    let attempts = Arc::new(AtomicUsize::new(0));
    let journal_attempts = Arc::new(AtomicUsize::new(0));
    let handle = Arc::new(RecordingSinkHandle::spawn_with_fault_for_test(
        dir.clone(),
        session.clone(),
        sink,
        receiver,
        CommitFaultPoint::Append,
        Arc::clone(&attempts),
        journal_attempts,
    ));

    handle
        .sink()
        .try_send(RecordingInput::PreparedFrame(prepared_frame(&session)))
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while attempts.load(Ordering::SeqCst) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "first append was not attempted"
        );
        std::thread::yield_now();
    }
    for _ in 0..1_000 {
        let _ = handle
            .sink()
            .try_send(RecordingInput::PreparedFrame(prepared_frame(&session)));
    }

    let (result_tx, result_rx) = mpsc::channel();
    let finalize_handle = Arc::clone(&handle);
    std::thread::spawn(move || {
        result_tx.send(finalize_handle.finalize()).unwrap();
    });
    let result = result_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("worker must finish after draining the bounded receiver")
        .unwrap();
    let repeated = handle.finalize().unwrap();

    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(result, repeated);
    assert_eq!(
        result.error.as_deref(),
        Some("injected recording fault at Append")
    );
    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(dir.join(format!("live-{session}.wav.part")).is_file());
    assert!(!dir.join(format!("live-{session}.commit.json")).exists());
    assert!(scan_recordings(&dir).unwrap().complete.is_empty());
    assert_eq!(scan_recordings(&dir).unwrap().partial.len(), 1);
}

#[test]
fn journal_persistence_failures_are_terminal_while_the_worker_drains_later_frames() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    for point in [
        CommitFaultPoint::JournalAppend,
        CommitFaultPoint::JournalSync,
    ] {
        let dir = tempfile_dir(&format!("worker-terminal-{point:?}"));
        let session = SessionId::new("s-worker-terminal-journal-failure").unwrap();
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 64);
        let attempts = Arc::new(AtomicUsize::new(0));
        let journal_attempts = Arc::new(AtomicUsize::new(0));
        let handle = Arc::new(RecordingSinkHandle::spawn_with_fault_for_test(
            dir.clone(),
            session.clone(),
            sink,
            receiver,
            point,
            Arc::clone(&attempts),
            Arc::clone(&journal_attempts),
        ));

        handle
            .sink()
            .try_send(RecordingInput::PreparedFrame(prepared_frame(&session)))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while attempts.load(Ordering::SeqCst) == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "first append was not attempted for {point:?}"
            );
            std::thread::yield_now();
        }
        for _ in 0..1_000 {
            let _ = handle
                .sink()
                .try_send(RecordingInput::PreparedFrame(prepared_frame(&session)));
        }

        let (result_tx, result_rx) = mpsc::channel();
        let finalize_handle = Arc::clone(&handle);
        std::thread::spawn(move || {
            result_tx.send(finalize_handle.finalize()).unwrap();
        });
        let result = result_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("worker must finish after draining the bounded receiver")
            .unwrap();
        let repeated = handle.finalize().unwrap();

        assert_eq!(attempts.load(Ordering::SeqCst), 1, "{point:?}");
        assert_eq!(journal_attempts.load(Ordering::SeqCst), 1, "{point:?}");
        assert_eq!(result, repeated, "{point:?}");
        assert_eq!(
            result.error,
            Some(format!("injected recording fault at {point:?}")),
            "{point:?}"
        );
        assert_eq!(result.status, CaptureStatus::Partial, "{point:?}");
        assert!(dir.join(format!("live-{session}.wav.part")).is_file());
        assert!(!dir.join(format!("live-{session}.commit.json")).exists());
        assert!(scan_recordings(&dir).unwrap().complete.is_empty());
        assert_eq!(scan_recordings(&dir).unwrap().partial.len(), 1);
    }
}

#[test]
fn rejected_recording_control_event_is_terminal_for_the_worker() {
    let dir = tempfile_dir("worker-rejected-control");
    let session = SessionId::new("s-worker-rejected-control").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 8);
    let handle = RecordingSinkHandle::spawn(dir.clone(), session.clone(), sink, receiver);

    handle
        .sink()
        .try_send(recording_revision(&track, 1, 0, 16_000, 0))
        .unwrap();
    handle
        .sink()
        .try_send(RecordingInput::PreparedFrame(prepared_frame(&session)))
        .unwrap();
    handle
        .sink()
        .try_send(recording_revision(&track, 3, 1, 16_000, 1))
        .unwrap();

    let result = handle.finalize().unwrap();

    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
    assert!(result
        .error
        .as_deref()
        .is_some_and(|error| error.contains("not monotonic")));
    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty());
    assert_eq!(scan.partial.len(), 1);
}

#[test]
fn prepared_frame_session_must_match_the_recording_session() {
    let dir = tempfile_dir("prepared-frame-session");
    let session = SessionId::new("s-prepared-frame-session").unwrap();
    let other_session = SessionId::new("s-other-prepared-frame-session").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let mut recording = StreamingRecording::create(&dir, session).unwrap();
    recording
        .append_input(recording_revision(&track, 1, 0, 16_000, 0))
        .unwrap();

    let error = recording
        .append_input(RecordingInput::PreparedFrame(prepared_frame(
            &other_session,
        )))
        .unwrap_err();
    let result = recording.finalize().unwrap();

    assert_eq!(error, "recording prepared frame session does not match");
    assert_eq!(result.status, CaptureStatus::Partial);
    assert_eq!(result.error.as_deref(), Some(error.as_str()));
    assert!(result.committed.is_none());
}

#[test]
fn first_prepared_frame_sequence_must_be_zero() {
    let dir = tempfile_dir("prepared-frame-sequence-prefix");
    let session = SessionId::new("s-prepared-frame-sequence-prefix").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording
        .append_input(recording_revision(&track, 1, 0, 16_000, 0))
        .unwrap();

    let error = recording
        .append_input(RecordingInput::PreparedFrame(prepared_frame_at(
            &session, 5, 0,
        )))
        .unwrap_err();
    let result = recording.finalize().unwrap();

    assert_eq!(error, "recording track sequence must start at zero");
    assert_eq!(result.status, CaptureStatus::Partial);
    assert_eq!(result.error.as_deref(), Some(error.as_str()));
    assert!(result.committed.is_none());
}

#[test]
fn journal_replay_rejects_sequence_coverage_that_starts_after_zero() {
    let session = SessionId::new("s-replay-sequence-prefix").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let header = JournalRecord::Header {
        journal: CaptureJournal::new(session.clone()),
    };
    let delta = JournalRecord::Delta {
        delta: JournalDelta {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id: session,
            tracks: vec![JournalTrack {
                track_id: track.as_str().to_string(),
                sample_rate_hz: 16_000,
                channels: 1,
                first_start_ms: 0,
            }],
            revision_transitions: vec![revision_transition(&track, 1, 0, 16_000, 0)],
            timeline_gap_start_index: 0,
            timeline_gaps: Vec::new(),
            sequence_coverage: vec![SequenceCoverage {
                track_id: track.as_str().to_string(),
                first_sequence: 5,
                last_sequence: 5,
            }],
            gap_start_index: 0,
            sequence_gaps: Vec::new(),
            sequence_gap_overflow: None,
            sink_degraded: false,
        },
    };
    let mut bytes = serialize_journal_record(&header).unwrap();
    bytes.extend(serialize_journal_record(&delta).unwrap());
    let text = String::from_utf8(bytes).unwrap();

    let error = parse_journal_append_log(&text).unwrap_err();

    assert_eq!(error, "recording track sequence must start at zero");
}

#[test]
fn sequence_discontinuity_keeps_later_audio_but_cannot_publish_complete() {
    let dir = tempfile_dir("worker-sequence-discontinuity");
    let session = SessionId::new("s-worker-sequence-discontinuity").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 8);
    let handle = RecordingSinkHandle::spawn(dir.clone(), session.clone(), sink, receiver);
    let recording_sink = handle.sink();
    for input in [
        recording_revision(&track, 1, 0, 16_000, 0),
        RecordingInput::PreparedFrame(prepared_frame_at(&session, 0, 0)),
        RecordingInput::PreparedFrame(prepared_frame_at(&session, 2, 1)),
        RecordingInput::PreparedFrame(prepared_frame_at(&session, 3, 2)),
    ] {
        recording_sink.try_send(input).unwrap();
    }

    let result = handle.finalize().unwrap();

    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
    assert!(result
        .error
        .as_deref()
        .is_some_and(|error| error.contains("sequence")));
    assert_eq!(
        recording_sink.outcome().error.as_deref(),
        Some("recording sequence discontinuity")
    );
    assert_eq!(
        fs::metadata(dir.join(format!("live-{session}.wav.part")))
            .unwrap()
            .len(),
        50
    );
    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty());
    assert_eq!(scan.partial.len(), 1);
}
