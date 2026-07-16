use super::*;

#[test]
fn rejects_pcm_that_exceeds_wav_u32_data_length_without_wrapping() {
    let dir = tempfile_dir("wav-limit");
    let mut recording =
        StreamingRecording::create(&dir, SessionId::new("s-limit").unwrap()).unwrap();
    recording.set_data_limit_for_test(2);

    assert!(recording.append_pcm16(&[1, 0, 2, 0]).is_err());
    assert_eq!(recording.finalize().unwrap().status, CaptureStatus::Partial);
}

#[test]
fn journal_coalesces_four_hours_of_contiguous_sequence_coverage() {
    let dir = tempfile_dir("journal-bounded");
    let session = SessionId::new("s-journal").unwrap();
    let mut recording = StreamingRecording::create(&dir, session).unwrap();
    for sequence in 0..(4 * 60 * 60 * 10) {
        recording.observe_frame_metadata("live-microphone", 16_000, 1, sequence, 0, 100);
    }

    let journal = recording.journal_for_test();
    assert_eq!(journal.sequence_coverage.len(), 1);
    assert!(journal.serialized_len() < 8_192);
}

#[test]
fn validates_manifest_names_as_same_directory_basename_only() {
    assert!(validate_artifact_name("live-s-a.wav").is_ok());
    for name in [
        "../live-s-a.wav",
        "C:\\live-s-a.wav",
        "/live-s-a.wav",
        "nested/live.wav",
    ] {
        assert!(validate_artifact_name(name).is_err(), "{name}");
    }
}

#[test]
fn finalization_is_idempotent() {
    let dir = tempfile_dir("idempotent");
    let mut recording =
        StreamingRecording::create(&dir, SessionId::new("s-idempotent").unwrap()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let first = recording.finalize().unwrap();
    let second = recording.finalize().unwrap();
    assert_eq!(first, second);
}

#[test]
fn sink_handle_finalizes_idempotently_for_concurrent_callers() {
    let dir = tempfile_dir("handle-idempotent");
    let session = SessionId::new("s-handle").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 3);
    let handle = Arc::new(RecordingSinkHandle::spawn(
        dir,
        session.clone(),
        sink,
        receiver,
    ));
    for input in [
        recording_revision(&track, 1, 0, 16_000, 0),
        RecordingInput::PreparedFrame(prepared_frame(&session)),
    ] {
        handle.sink().try_send(input).unwrap();
    }

    let left_handle = Arc::clone(&handle);
    let left = std::thread::spawn(move || left_handle.finalize().unwrap());
    let right_handle = Arc::clone(&handle);
    let right = std::thread::spawn(move || right_handle.finalize().unwrap());

    let left = left.join().unwrap();
    let right = right.join().unwrap();
    assert_eq!(left, right);
    assert_eq!(left.status, CaptureStatus::Complete);
}

#[test]
fn sink_handle_caches_worker_panic_for_racing_and_repeated_callers() {
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
    let handle = Arc::new(RecordingSinkHandle::spawn_panicking_for_test(
        sink,
        receiver,
        SessionId::new("s-panicking-recording").unwrap(),
    ));
    let barrier = Arc::new(std::sync::Barrier::new(3));

    let left_handle = Arc::clone(&handle);
    let left_barrier = Arc::clone(&barrier);
    let left = std::thread::spawn(move || {
        left_barrier.wait();
        left_handle.finalize()
    });
    let right_handle = Arc::clone(&handle);
    let right_barrier = Arc::clone(&barrier);
    let right = std::thread::spawn(move || {
        right_barrier.wait();
        right_handle.finalize()
    });

    barrier.wait();
    let left = left.join().unwrap();
    let right = right.join().unwrap();
    let repeated = handle.finalize();

    assert_eq!(left, right);
    assert_eq!(left, repeated);
    assert_eq!(
        left.unwrap_err(),
        "recording worker panicked during finalization"
    );
}
