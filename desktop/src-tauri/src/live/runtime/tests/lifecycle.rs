use super::*;

#[test]
fn stream_crash_retires_runtime_handles() {
    let mut inner = LiveRuntimeInner::for_test();
    inner.has_capture_for_test = true;
    inner.has_stream_for_test = true;

    inner.mark_stream_crashed_for_test();

    assert!(!inner.has_capture_for_test);
    assert!(!inner.has_stream_for_test);
}

#[test]
fn stale_stream_crash_cannot_claim_a_newer_session() {
    let runtime = LiveRuntime::new();
    runtime.active_session.store(7, Ordering::SeqCst);

    assert!(!runtime.claim_stream_crash(6));
    assert_eq!(runtime.active_session.load(Ordering::SeqCst), 7);
    assert!(runtime.claim_stream_crash(7));
    assert_eq!(
        runtime.active_session.load(Ordering::SeqCst),
        7 | CRASH_CLAIM_BIT
    );
    assert!(active_session_matches(
        runtime.active_session.load(Ordering::SeqCst),
        7
    ));
    assert!(runtime.is_session_current(7));
    assert!(!runtime.is_session_current(8));
    assert!(!runtime.claim_stream_crash(7));
    assert!(!runtime.claim_stream_crash(0));
}

#[test]
fn stale_start_failure_cannot_clear_a_newer_session() {
    let runtime = LiveRuntime::new();
    runtime.active_session.store(8, Ordering::SeqCst);

    assert_eq!(
        runtime.claim_start_failure(LiveStartFailure::new(7, "old failure".into())),
        None
    );
    assert_eq!(runtime.active_session.load(Ordering::SeqCst), 8);
    assert_eq!(
        runtime.claim_start_failure(LiveStartFailure::new(8, "current failure".into())),
        Some("current failure".into())
    );
    assert_eq!(runtime.active_session.load(Ordering::SeqCst), 0);
}

#[test]
fn cancelling_a_start_intent_preserves_the_active_session_for_final_drain() {
    let runtime = LiveRuntime::new();
    runtime.active_session.store(7, Ordering::SeqCst);

    runtime.cancel_pending_start();

    assert_eq!(runtime.active_session.load(Ordering::SeqCst), 7);
}

#[test]
fn cancellation_after_capture_handoff_keeps_the_recording_for_stop_cataloging() {
    let runtime = LiveRuntime::new();
    let directory = std::env::temp_dir().join(format!(
        "yap-runtime-cancelled-start-recording-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&directory).ok();
    let session_id = SessionId::new("cancelled-after-open").unwrap();
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
    {
        let mut inner = runtime.inner.lock().unwrap();
        inner.session = 7;
        inner.recording = Some(RecordingSinkHandle::spawn(
            directory.clone(),
            session_id.clone(),
            sink,
            receiver,
        ));
    }
    runtime.active_session.store(7, Ordering::SeqCst);
    let intent = runtime.capture_start_intent();

    runtime.cancel_pending_start();
    assert!(!runtime.start_intent_is_current(intent));
    assert_eq!(runtime.active_session.load(Ordering::SeqCst), 7);

    let stopped = runtime.stop();
    let recording = stopped.recording.unwrap().unwrap();
    assert_eq!(recording.session_id, session_id);
    assert_eq!(recording.status, CaptureStatus::Complete);
    let catalog = scan_recordings(&directory).unwrap();
    assert_eq!(catalog.complete.len(), 1);
    assert_eq!(catalog.complete[0].manifest.session_id, session_id);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queued_stop_runs_before_a_later_start() {
    let lifecycle = Arc::new(LifecycleGate::new());
    let held = lifecycle.begin_start();
    let (stop_queued_tx, stop_queued_rx) = mpsc::channel();
    let (stop_entered_tx, stop_entered_rx) = mpsc::channel();
    let (release_stop_tx, release_stop_rx) = mpsc::channel();
    let stop_lifecycle = Arc::clone(&lifecycle);
    let stopper = std::thread::spawn(move || {
        let _stop = stop_lifecycle.begin_stop_with_wait_hook(|| {
            stop_queued_tx.send(()).unwrap();
        });
        stop_entered_tx.send(()).unwrap();
        release_stop_rx.recv().unwrap();
    });
    stop_queued_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    let (start_queued_tx, start_queued_rx) = mpsc::channel();
    let (start_entered_tx, start_entered_rx) = mpsc::channel();
    let start_lifecycle = Arc::clone(&lifecycle);
    let starter = std::thread::spawn(move || {
        let _start = start_lifecycle.begin_start_with_wait_hook(|| {
            start_queued_tx.send(()).unwrap();
        });
        start_entered_tx.send(()).unwrap();
    });
    start_queued_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    drop(held);
    stop_entered_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(matches!(
        start_entered_rx.recv_timeout(Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));

    release_stop_tx.send(()).unwrap();
    start_entered_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    stopper.join().unwrap();
    starter.join().unwrap();
}

#[test]
fn stop_finalizes_before_a_concurrent_start_activates_the_next_session() {
    let lifecycle = Arc::new(LifecycleGate::new());
    let (samples_tx, samples_rx) = mpsc::sync_channel(8);
    let (old_adapter_drained_tx, old_adapter_drained_rx) = mpsc::channel();
    let (allow_old_finish_tx, allow_old_finish_rx) = mpsc::channel();
    let (old_finish_acked_tx, old_finish_acked_rx) = mpsc::channel();
    let (new_start_attempted_tx, new_start_attempted_rx) = mpsc::channel();
    let (new_start_waiting_tx, new_start_waiting_rx) = mpsc::channel();
    let (new_start_complete_tx, new_start_complete_rx) = mpsc::channel();
    let finalized = Arc::new(Mutex::new(Vec::new()));
    let finalized_for_worker = Arc::clone(&finalized);
    let recognizer = std::thread::spawn(move || {
        let mut expected_session = 1;
        while expected_session <= 2 {
            match samples_rx.recv_timeout(Duration::from_secs(1)).unwrap() {
                StreamMessage::Samples { session, .. } => {
                    assert_eq!(session, expected_session);
                }
                StreamMessage::Finish { session, done } => {
                    assert_eq!(session, expected_session);
                    finalized_for_worker.lock().unwrap().push(session);
                    done.send(StreamFinishStatus::Completed).unwrap();
                    expected_session += 1;
                }
            }
        }
    });

    let mut old_adapter = SessionAsrAdapter::start(samples_tx.clone(), 1);
    let old_port = old_adapter.sink();
    old_port.try_send(prepared_frame(0.25)).unwrap();
    old_port.close();

    let stop_lifecycle = Arc::clone(&lifecycle);
    let stop_samples_tx = samples_tx.clone();
    let stopper = std::thread::spawn(move || {
        let _stop = stop_lifecycle.begin_stop();
        old_adapter.join_after_capture().unwrap();
        old_adapter_drained_tx.send(()).unwrap();
        allow_old_finish_rx.recv().unwrap();
        let status = StreamFinisher::new(stop_samples_tx, 1).finish_session();
        assert_eq!(status, StreamFinishStatus::Completed);
        old_finish_acked_tx.send(()).unwrap();
    });

    old_adapter_drained_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    let start_lifecycle = Arc::clone(&lifecycle);
    let new_samples_tx = samples_tx;
    let starter = std::thread::spawn(move || {
        new_start_attempted_tx.send(()).unwrap();
        let _start = start_lifecycle.begin_start_with_wait_hook(|| {
            new_start_waiting_tx.send(()).unwrap();
        });
        let mut new_adapter = SessionAsrAdapter::start(new_samples_tx.clone(), 2);
        let new_port = new_adapter.sink();
        new_port.try_send(prepared_frame(0.75)).unwrap();
        new_port.close();
        new_adapter.join_after_capture().unwrap();
        assert_eq!(
            StreamFinisher::new(new_samples_tx, 2).finish_session(),
            StreamFinishStatus::Completed
        );
        new_start_complete_tx.send(()).unwrap();
    });

    new_start_attempted_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    new_start_waiting_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    allow_old_finish_tx.send(()).unwrap();
    old_finish_acked_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    new_start_complete_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    stopper.join().unwrap();
    starter.join().unwrap();
    recognizer.join().unwrap();
    assert_eq!(*finalized.lock().unwrap(), vec![1, 2]);
}

#[test]
fn stop_cancels_initializing_start_before_capture_and_releases_the_lifecycle_gate() {
    let runtime = Arc::new(LiveRuntime::new());
    let intent = runtime.capture_start_intent();
    let start_entered = Arc::new(Barrier::new(2));
    let release_start = Arc::new(Barrier::new(2));
    let capture_opened = Arc::new(AtomicBool::new(false));

    let starter = {
        let runtime = Arc::clone(&runtime);
        let start_entered = Arc::clone(&start_entered);
        let release_start = Arc::clone(&release_start);
        let capture_opened = Arc::clone(&capture_opened);
        std::thread::spawn(move || {
            runtime.run_start_lifecycle(intent, || {
                start_entered.wait();
                release_start.wait();
                if runtime.start_intent_is_current(intent) {
                    capture_opened.store(true, Ordering::SeqCst);
                }
            });
        })
    };

    start_entered.wait();
    runtime.cancel_pending_start();
    let stopper = {
        let runtime = Arc::clone(&runtime);
        std::thread::spawn(move || runtime.run_stop_lifecycle(|| {}))
    };
    release_start.wait();

    starter.join().unwrap();
    stopper.join().unwrap();
    assert!(!capture_opened.load(Ordering::SeqCst));
    let next_intent = runtime.capture_start_intent();
    assert_eq!(
        runtime.run_start_lifecycle(next_intent, || true),
        Some(true)
    );
}
