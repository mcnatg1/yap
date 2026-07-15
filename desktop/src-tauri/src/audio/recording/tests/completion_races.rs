use super::*;

#[test]
fn abort_racing_completion_is_rejected_instead_of_reported_complete() {
    let dir = tempfile_dir("abort-finalize-linearization");
    let session = SessionId::new("s-abort-finalize-linearization").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let (publication_tx, publication_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut release_rx = Some(release_rx);
    let recording = StreamingRecording::create_with_publication_hook(
        &dir,
        session.clone(),
        None,
        move |artifact, barrier, _| {
            if artifact == PublicationArtifact::CompleteSidecar
                && barrier == PublicationBarrier::BeforeHardLink
            {
                publication_tx.send(()).unwrap();
                release_rx
                    .take()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
            }
        },
    )
    .unwrap();
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 8);
    let worker_sink = sink.clone();
    let worker_session = session.clone();
    let worker = std::thread::spawn(move || {
        drain_recording_worker(recording, worker_session, receiver, worker_sink)
    });
    let handle = Arc::new(RecordingSinkHandle::with_worker(
        sink,
        session.clone(),
        worker,
    ));
    for input in [
        recording_revision(&track, 1, 0, 16_000, 0),
        RecordingInput::PreparedFrame(prepared_frame(&session)),
    ] {
        handle.sink().try_send(input).unwrap();
    }

    let finalize_handle = Arc::clone(&handle);
    let finalize = std::thread::spawn(move || finalize_handle.finalize());
    publication_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("worker must reach complete-sidecar publication");
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        release_tx.send(()).unwrap();
    });
    let abort_result = handle.abort("adapter failed");
    release.join().unwrap();
    let finalize_result = finalize.join().unwrap();
    assert!(!matches!(
        abort_result,
        Ok(RecordingFinalizeResult {
            status: CaptureStatus::Complete,
            ..
        })
    ));
    assert_eq!(finalize_result.unwrap().status, CaptureStatus::Complete);
}

#[test]
fn accepted_abort_wins_before_completion_linearizes() {
    let dir = tempfile_dir("abort-wins-linearization");
    let session = SessionId::new("s-abort-wins-linearization").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    let (release_tx, release_rx) = mpsc::channel();
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 8);
    let worker_sink = sink.clone();
    let worker_session = session.clone();
    let worker = std::thread::spawn(move || {
        release_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("worker release must arrive");
        drain_recording_worker(recording, worker_session, receiver, worker_sink)
    });
    let handle = Arc::new(RecordingSinkHandle::with_worker(
        sink.clone(),
        session.clone(),
        worker,
    ));
    for input in [
        recording_revision(&track, 1, 0, 16_000, 0),
        RecordingInput::PreparedFrame(prepared_frame(&session)),
    ] {
        handle.sink().try_send(input).unwrap();
    }

    let abort_handle = Arc::clone(&handle);
    let abort = std::thread::spawn(move || abort_handle.abort("accepted adapter failure"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        if sink.outcome().error.as_deref() == Some("accepted adapter failure") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "abort must be accepted before the worker is released"
        );
        std::thread::yield_now();
    }
    release_tx.send(()).unwrap();

    let result = abort.join().unwrap().unwrap();
    let repeated = handle.finalize().unwrap();
    assert_eq!(result.status, CaptureStatus::Partial);
    assert_eq!(repeated, result);
    assert!(result.committed.is_none());
    assert!(scan_recordings(&dir).unwrap().complete.is_empty());
}

#[test]
fn sink_degradation_wins_before_completion_linearizes() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let dir = tempfile_dir("sink-degradation-wins-linearization");
    let session = SessionId::new("s-sink-degradation-wins-linearization").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 8);
    let completion_sampled = Arc::new(std::sync::Barrier::new(2));
    let release_completion = Arc::new(std::sync::Barrier::new(2));
    let hook_fired = Arc::new(AtomicBool::new(false));
    sink.set_before_completion_hook_for_test({
        let completion_sampled = Arc::clone(&completion_sampled);
        let release_completion = Arc::clone(&release_completion);
        let hook_fired = Arc::clone(&hook_fired);
        Arc::new(move || {
            if !hook_fired.swap(true, Ordering::SeqCst) {
                completion_sampled.wait();
                release_completion.wait();
            }
        })
    });
    let worker_sink = sink.clone();
    let worker_session = session.clone();
    let worker = std::thread::spawn(move || {
        drain_recording_worker(recording, worker_session, receiver, worker_sink)
    });
    let handle = Arc::new(RecordingSinkHandle::with_worker(
        sink.clone(),
        session.clone(),
        worker,
    ));
    for input in [
        recording_revision(&track, 1, 0, 16_000, 0),
        RecordingInput::PreparedFrame(prepared_frame(&session)),
    ] {
        sink.try_send(input).unwrap();
    }

    let finalize_handle = Arc::clone(&handle);
    let finalize = std::thread::spawn(move || finalize_handle.finalize());
    completion_sampled.wait();
    sink.degrade("recording sink failed before completion");
    release_completion.wait();
    let result = finalize.join().unwrap().unwrap();

    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
    assert_eq!(
        sink.outcome().error.as_deref(),
        Some("recording sink failed before completion")
    );
    assert!(scan_recordings(&dir).unwrap().complete.is_empty());
}

#[test]
fn sink_completion_rejects_late_degradation_after_linearization() {
    let dir = tempfile_dir("sink-completion-wins-linearization");
    let session = SessionId::new("s-sink-completion-wins-linearization").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let (publication_tx, publication_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut release_rx = Some(release_rx);
    let recording = StreamingRecording::create_with_publication_hook(
        &dir,
        session.clone(),
        None,
        move |artifact, barrier, _| {
            if artifact == PublicationArtifact::CompleteSidecar
                && barrier == PublicationBarrier::BeforeHardLink
            {
                publication_tx.send(()).unwrap();
                release_rx
                    .take()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
            }
        },
    )
    .unwrap();
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 8);
    let worker_sink = sink.clone();
    let worker_session = session.clone();
    let worker = std::thread::spawn(move || {
        drain_recording_worker(recording, worker_session, receiver, worker_sink)
    });
    let handle = Arc::new(RecordingSinkHandle::with_worker(
        sink.clone(),
        session.clone(),
        worker,
    ));
    for input in [
        recording_revision(&track, 1, 0, 16_000, 0),
        RecordingInput::PreparedFrame(prepared_frame(&session)),
    ] {
        sink.try_send(input).unwrap();
    }

    let finalize_handle = Arc::clone(&handle);
    let finalize = std::thread::spawn(move || finalize_handle.finalize());
    publication_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("worker must linearize completion before publication");
    sink.degrade("late recording sink failure");
    let late_degradation = sink.outcome().error;
    release_tx.send(()).unwrap();
    let result = finalize.join().unwrap().unwrap();

    assert_eq!(result.status, CaptureStatus::Complete);
    assert!(result.committed.is_some());
    assert_eq!(late_degradation, None);
}
