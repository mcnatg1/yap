use super::*;

#[test]
fn timed_out_recognizer_blocks_replacement_until_its_worker_is_reaped() {
    let mut inner = LiveRuntimeInner::for_test();
    let release_worker = Arc::new(Barrier::new(2));
    let worker_released = Arc::clone(&release_worker);
    let worker = std::thread::spawn(move || {
        worker_released.wait();
    });
    let (samples_tx, _samples_rx) = mpsc::sync_channel(1);
    inner.stream = Some(SessionStream {
        session: Arc::new(AtomicU64::new(1)),
        samples_tx,
        cancelled: Arc::new(AtomicBool::new(false)),
        worker: Some(worker),
        model_warmup: None,
    });

    inner.retire_stream_detached_reader();
    assert_eq!(
        inner.reap_retiring_stream(),
        Err("Previous live transcription is still stopping.".into())
    );

    release_worker.wait();
    let deadline = Instant::now() + Duration::from_secs(1);
    while inner
        .retiring_stream
        .as_ref()
        .and_then(|stream| stream.worker.as_ref())
        .is_some_and(|worker| !worker.is_finished())
    {
        assert!(
            Instant::now() < deadline,
            "retired recognizer did not finish"
        );
        std::thread::yield_now();
    }
    assert_eq!(inner.reap_retiring_stream(), Ok(()));
    assert!(inner.retiring_stream.is_none());
}

#[test]
fn idle_cleanup_does_not_join_a_still_stalled_recognizer() {
    let mut inner = LiveRuntimeInner::for_test();
    let release_worker = Arc::new(Barrier::new(2));
    let worker_released = Arc::clone(&release_worker);
    let worker = std::thread::spawn(move || {
        worker_released.wait();
    });
    let (samples_tx, _samples_rx) = mpsc::sync_channel(1);
    inner.retiring_stream = Some(SessionStream {
        session: Arc::new(AtomicU64::new(1)),
        samples_tx,
        cancelled: Arc::new(AtomicBool::new(true)),
        worker: Some(worker),
        model_warmup: None,
    });
    let (done_tx, done_rx) = mpsc::channel();
    let cleanup = std::thread::spawn(move || {
        inner.retire_stream();
        done_tx.send(()).unwrap();
        inner
    });

    let completed_without_joining = done_rx.recv_timeout(Duration::from_secs(1));
    release_worker.wait();
    let mut inner = cleanup.join().unwrap();

    assert!(completed_without_joining.is_ok());
    assert!(inner.retiring_stream.is_some());
    let deadline = Instant::now() + Duration::from_secs(1);
    while inner
        .retiring_stream
        .as_ref()
        .and_then(|stream| stream.worker.as_ref())
        .is_some_and(|worker| !worker.is_finished())
    {
        assert!(
            Instant::now() < deadline,
            "retired recognizer did not finish"
        );
        std::thread::yield_now();
    }
    assert_eq!(inner.reap_retiring_stream(), Ok(()));
    assert!(inner.retiring_stream.is_none());
}

#[test]
fn asr_adapter_forwards_the_last_accepted_frame_before_it_joins() {
    let (samples_tx, samples_rx) = mpsc::sync_channel(1);
    let mut adapter = SessionAsrAdapter::start(samples_tx, 7);
    let port = adapter.sink();
    port.try_send(prepared_frame(0.25)).unwrap();
    port.close();

    adapter.join_after_capture().unwrap();
    match samples_rx.recv_timeout(Duration::from_secs(1)).unwrap() {
        StreamMessage::Samples { session, samples } => {
            assert_eq!(session, 7);
            assert_eq!(samples, vec![0.25]);
        }
        StreamMessage::Finish { .. } => panic!("expected the accepted frame"),
    }
}

#[test]
fn pending_asr_adapter_keeps_bounded_pre_roll_until_the_model_is_ready() {
    let pending = PendingAsrAdapter::new();
    let port = pending.sink();
    port.try_send(prepared_frame(0.4)).unwrap();
    assert_eq!(port.high_water_mark(), 1);
    let (samples_tx, samples_rx) = mpsc::sync_channel(1);

    let mut adapter = pending.start(samples_tx, 11);
    port.close();
    adapter.join_after_capture().unwrap();

    match samples_rx.recv_timeout(Duration::from_secs(1)).unwrap() {
        StreamMessage::Samples { session, samples } => {
            assert_eq!(session, 11);
            assert_eq!(samples, vec![0.4]);
        }
        StreamMessage::Finish { .. } => panic!("expected queued pre-roll"),
    }
}

#[test]
fn stalled_recognizer_times_out_stop_without_enqueuing_finish() {
    let (samples_tx, samples_rx) = mpsc::sync_channel(1);
    samples_tx
        .try_send(StreamMessage::Samples {
            session: 7,
            samples: vec![0.0],
        })
        .unwrap();
    let mut adapter = SessionAsrAdapter::start(samples_tx.clone(), 7);
    let port = adapter.sink();
    port.try_send(prepared_frame(0.25)).unwrap();
    port.close();
    let finisher = StreamFinisher {
        samples_tx,
        session: 7,
    };

    let started = Instant::now();
    let status = stop_after_capture_for_test(&mut adapter, &finisher, Duration::from_millis(25));

    assert_eq!(status, StreamFinishStatus::TimedOut);
    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(!adapter.retains_cleanup_ownership());
    assert!(matches!(
        samples_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        StreamMessage::Samples { .. }
    ));
    assert!(matches!(
        samples_rx.recv_timeout(Duration::from_millis(25)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
}

#[test]
fn reaper_spawn_failure_retains_adapter_ownership_and_reports_a_bounded_stop() {
    let (samples_tx, samples_rx) = mpsc::sync_channel(1);
    samples_tx
        .try_send(StreamMessage::Samples {
            session: 7,
            samples: vec![0.0],
        })
        .unwrap();
    let completion_gate = Arc::new(Barrier::new(2));
    let mut adapter = SessionAsrAdapter::start_with_completion_gate_for_test(
        samples_tx.clone(),
        7,
        Arc::clone(&completion_gate),
    );
    let port = adapter.sink();
    port.try_send(prepared_frame(0.25)).unwrap();
    port.close();
    let finisher = StreamFinisher {
        samples_tx,
        session: 7,
    };

    set_reaper_spawn_failure_for_test();
    let started = Instant::now();
    let status = stop_after_capture_for_test(&mut adapter, &finisher, Duration::from_millis(25));

    assert_eq!(status, StreamFinishStatus::TimedOut);
    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(adapter.retains_cleanup_ownership_for_test());
    assert!(matches!(
        samples_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        StreamMessage::Samples { .. }
    ));
    assert!(matches!(
        samples_rx.recv_timeout(Duration::from_millis(25)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));

    completion_gate.wait();
    adapter.cancel_and_join().unwrap();
}

#[test]
fn two_capture_sessions_use_fresh_asr_ports_and_finish_each_once_in_fifo_order() {
    let (samples_tx, samples_rx) = mpsc::sync_channel(8);
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let delivered_for_worker = Arc::clone(&delivered);
    let recognizer = std::thread::spawn(move || {
        let mut finishes = 0;
        while finishes < 2 {
            match samples_rx.recv_timeout(Duration::from_secs(1)).unwrap() {
                StreamMessage::Samples { session, samples } => {
                    delivered_for_worker
                        .lock()
                        .unwrap()
                        .push((session, samples));
                }
                StreamMessage::Finish { session, done } => {
                    delivered_for_worker
                        .lock()
                        .unwrap()
                        .push((session, Vec::new()));
                    finishes += 1;
                    done.send(StreamFinishStatus::Completed).unwrap();
                }
            }
        }
    });

    let mut first = SessionAsrAdapter::start(samples_tx.clone(), 1);
    let first_port = first.sink();
    first_port.try_send(prepared_frame(0.25)).unwrap();
    first_port.close();
    first.join_after_capture().unwrap();
    assert_eq!(
        StreamFinisher {
            samples_tx: samples_tx.clone(),
            session: 1,
        }
        .finish_session(),
        StreamFinishStatus::Completed
    );
    assert_eq!(first_port.outcome().accepted_frames, 1);
    assert_eq!(first_port.outcome().dropped_frames, 0);
    assert_eq!(first_port.outcome().error, None);

    let mut second = SessionAsrAdapter::start(samples_tx.clone(), 2);
    let second_port = second.sink();
    assert!(matches!(
        first_port.try_send(prepared_frame(0.5)),
        Err(crate::audio::coordinator::SinkSendError::Closed)
    ));
    second_port.try_send(prepared_frame(0.75)).unwrap();
    second_port.close();
    second.join_after_capture().unwrap();
    assert_eq!(
        StreamFinisher {
            samples_tx,
            session: 2,
        }
        .finish_session(),
        StreamFinishStatus::Completed
    );
    assert_eq!(second_port.outcome().accepted_frames, 1);
    assert_eq!(second_port.outcome().dropped_frames, 0);
    assert_eq!(second_port.outcome().error, None);

    recognizer.join().unwrap();
    assert_eq!(
        *delivered.lock().unwrap(),
        vec![
            (1, vec![0.25]),
            (1, Vec::new()),
            (2, vec![0.75]),
            (2, Vec::new()),
        ]
    );
}
