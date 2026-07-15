use super::*;

#[test]
fn guarded_capture_packet_worker_reports_a_synthetic_panic_and_exits() {
    let (crash_tx, crash_rx) = mpsc::channel();
    let (recording, _recording_rx) = bounded_sink(SinkKind::Recording, 1);
    let recording_for_worker = recording.clone();
    let worker = std::thread::spawn(move || {
        run_guarded_capture_packet_worker(
            &recording_for_worker,
            || panic!("synthetic packet worker panic"),
            move |message| crash_tx.send(message).unwrap(),
        );
    });

    assert_eq!(
        crash_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        CAPTURE_WORKER_FAILURE
    );
    assert_eq!(
        recording.outcome().error.as_deref(),
        Some(CAPTURE_WORKER_FAILURE)
    );
    worker.join().unwrap();
}

#[test]
fn guarded_capture_worker_panic_after_pcm_is_a_stable_partial_capture() {
    let directory =
        std::env::temp_dir().join(format!("yap-runtime-guarded-panic-{}", std::process::id()));
    std::fs::remove_dir_all(&directory).ok();
    std::fs::create_dir_all(&directory).unwrap();
    let session_id = SessionId::new("s-runtime-guarded-panic").unwrap();
    let (recording_sink, recording_rx) =
        bounded_sink(SinkKind::Recording, RECORDING_QUEUE_CAPACITY);
    let recording = RecordingSinkHandle::spawn(
        directory.clone(),
        session_id.clone(),
        recording_sink.clone(),
        recording_rx,
    );
    let (local_asr, _local_asr_rx) = bounded_sink(SinkKind::LocalAsr, 8);
    let mut coordinator =
        capture_worker_coordinator(session_id.clone(), recording_sink.clone(), local_asr);
    let (crash_tx, crash_rx) = mpsc::channel();

    run_guarded_capture_packet_worker(
        &recording_sink,
        || {
            coordinator
                .consume(
                    &CapturePacket {
                        source_position_frames: 0,
                        channels: 1,
                        sample_rate_hz: 16_000,
                        samples: vec![0.25; 400],
                    },
                    &LossAccumulator::new(),
                )
                .unwrap();
            panic!("synthetic packet worker panic after accepted PCM");
        },
        move |message| crash_tx.send(message).unwrap(),
    );
    coordinator.close();

    let result = recording.finalize().unwrap();
    let worker_failure = CAPTURE_WORKER_FAILURE;
    assert_eq!(
        crash_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        worker_failure
    );
    assert_eq!(
        recording_sink.outcome().error.as_deref(),
        Some(worker_failure)
    );
    assert_eq!(result.status, CaptureStatus::Partial);
    assert_eq!(result.error.as_deref(), Some(worker_failure));
    assert!(result.committed.is_none());
    assert_eq!(
        std::fs::metadata(directory.join(format!("live-{session_id}.wav.part")))
            .unwrap()
            .len(),
        844
    );
    let scan = scan_recordings(&directory).unwrap();
    assert!(scan.complete.is_empty());
    assert_eq!(scan.partial.len(), 1);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stale_pcm_is_discarded_after_session_changes() {
    assert!(should_accept_stream_samples(2, 2, 2));
    assert!(should_accept_stream_samples(2, 2 | CRASH_CLAIM_BIT, 2));
    assert!(!should_accept_stream_samples(1, 2, 2));
    assert!(!should_accept_stream_samples(2, 0, 2));
    assert!(!should_accept_stream_samples(2, 2, 0));
}

#[test]
fn stale_capture_install_is_rejected_after_stop_or_new_session() {
    assert!(should_install_capture(2, 2, 2, false));
    assert!(!should_install_capture(2, 2, 0, false));
    assert!(!should_install_capture(2, 3, 2, false));
    assert!(!should_install_capture(2, 2, 2, true));
}

#[test]
fn local_asr_degradation_is_marked_once_without_stopping_recording() {
    let degradation_reported = AtomicBool::new(false);

    assert!(mark_local_asr_degraded_once(&degradation_reported));
    assert!(!mark_local_asr_degraded_once(&degradation_reported));
    assert!(degradation_reported.load(Ordering::SeqCst));
}
