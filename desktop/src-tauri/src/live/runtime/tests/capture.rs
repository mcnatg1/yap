use super::*;

#[test]
fn reserved_recording_session_is_canonical_for_worker_frames_gaps_and_commit() {
    let directory = std::env::temp_dir().join(format!(
        "yap-runtime-reserved-session-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&directory).ok();
    std::fs::create_dir_all(&directory).unwrap();
    let reservation = allocate_recording_session(&directory).unwrap();
    let recording_session_id = reservation.session_id().clone();
    let (recording_sink, recording_rx) =
        bounded_sink(SinkKind::Recording, RECORDING_QUEUE_CAPACITY);
    let recording =
        RecordingSinkHandle::spawn_reserved(reservation, recording_sink.clone(), recording_rx);
    let (local_asr, local_asr_rx) = bounded_sink(SinkKind::LocalAsr, 8);
    let coordinator = Arc::new(Mutex::new(capture_worker_coordinator(
        recording_session_id.clone(),
        recording_sink,
        local_asr,
    )));

    coordinator
        .lock()
        .unwrap()
        .consume(
            &CapturePacket {
                source_position_frames: 0,
                channels: 1,
                sample_rate_hz: 16_000,
                samples: vec![0.0; 4_000],
            },
            &LossAccumulator::new(),
        )
        .unwrap();
    let prepared = local_asr_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(prepared.metadata.session_id, recording_session_id);

    let mut crash_messages = Vec::new();
    assert!(!process_capture_loss(
        &coordinator,
        Ok(LossSnapshot {
            first_source_position_frames: 4_000,
            dropped_frames: 1_600,
            cause: GapCause::DeviceDiscontinuity,
            generation: 7,
        }),
        |message| crash_messages.push(message),
    ));
    assert!(crash_messages.is_empty());
    coordinator
        .lock()
        .unwrap()
        .consume(
            &CapturePacket {
                source_position_frames: 5_600,
                channels: 1,
                sample_rate_hz: 16_000,
                samples: vec![0.0; 400],
            },
            &LossAccumulator::new(),
        )
        .unwrap();
    coordinator.lock().unwrap().close();

    let result = recording.finalize().unwrap();
    assert_eq!(result.status, CaptureStatus::Complete, "{:?}", result.error);
    assert_eq!(result.session_id, recording_session_id);
    let commit: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.join(format!("live-{recording_session_id}.commit.json"))).unwrap(),
    )
    .unwrap();
    let sidecar: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.join(format!("live-{recording_session_id}.capture.json")))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(commit["sessionId"], recording_session_id.as_str());
    assert_eq!(sidecar["sessionId"], recording_session_id.as_str());
    assert_eq!(sidecar["sequenceCoverage"][0]["firstSequence"], 0);
    assert_eq!(sidecar["sequenceCoverage"][0]["lastSequence"], 1);
    assert_eq!(sidecar["timelineGaps"].as_array().unwrap().len(), 1);
    let gap = &sidecar["timelineGaps"][0];
    assert_eq!(gap["sessionId"], recording_session_id.as_str());
    assert_eq!(gap["trackId"], "live-microphone");
    assert_eq!(gap["startMs"], 250);
    assert_eq!(gap["durationMs"], 100);
    assert_eq!(gap["sourcePositionFrames"], 4_000);
    assert_eq!(gap["droppedFrames"], 1_600);
    assert_eq!(gap["cause"], "device_discontinuity");
    assert_eq!(gap["generation"], 7);
    let scan = scan_recordings(&directory).unwrap();
    assert_eq!(scan.complete.len(), 1);
    assert_eq!(scan.complete[0].manifest.session_id, recording_session_id);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn capture_packet_worker_returns_buffer_and_joins_after_disconnect() {
    let (packet_tx, packet_rx) = mpsc::sync_channel(1);
    let (returned_tx, returned_rx) = mpsc::sync_channel(8);
    let (error_tx, error_rx) = mpsc::sync_channel::<cpal::StreamError>(1);
    let ports = CapturePorts {
        packets: packet_rx,
        returned_buffers: returned_tx,
        losses: Arc::new(LossAccumulator::new()),
    };
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_capture_packet_loop(ports, error_rx, |_, _| false, |_| false, |_| false);
        done_tx.send(()).unwrap();
    });
    let mut samples = Vec::with_capacity(4);
    samples.extend([0.25, -0.25]);
    let allocation = samples.as_ptr();
    packet_tx
        .send(CapturePacket {
            source_position_frames: 0,
            channels: 2,
            sample_rate_hz: 48_000,
            samples,
        })
        .unwrap();

    let returned = returned_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(returned.as_ptr(), allocation);
    assert!(returned.is_empty());
    drop(packet_tx);
    drop(error_tx);

    done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
}

#[test]
fn capture_packet_loop_drains_loss_on_timeout_without_packets() {
    let (packet_tx, packet_rx) = mpsc::sync_channel(1);
    let (returned_tx, _) = mpsc::sync_channel(8);
    let (error_tx, error_rx) = mpsc::sync_channel::<cpal::StreamError>(1);
    let losses = Arc::new(LossAccumulator::new());
    let ports = CapturePorts {
        packets: packet_rx,
        returned_buffers: returned_tx,
        losses: Arc::clone(&losses),
    };
    let (loss_tx, loss_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_capture_packet_loop_with_timeout(
            ports,
            error_rx,
            Duration::from_millis(1),
            |_, _| false,
            |_| false,
            move |loss| loss_tx.send(loss).is_err(),
        );
    });

    std::thread::sleep(Duration::from_millis(5));
    losses.record(240, 160, crate::audio::frame::GapCause::SinkUnavailable);
    let snapshot = loss_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.first_source_position_frames, 240);
    assert_eq!(snapshot.dropped_frames, 160);
    assert_eq!(
        snapshot.cause,
        crate::audio::frame::GapCause::SinkUnavailable
    );

    drop(packet_tx);
    drop(error_tx);
    worker.join().unwrap();
}

#[test]
fn capture_packet_loop_disconnects_while_a_loss_drain_is_pending() {
    let losses = Arc::new(LossAccumulator::new());
    let registration_started = Arc::new(Barrier::new(2));
    let release_registration = Arc::new(Barrier::new(2));
    let callback = {
        let losses = Arc::clone(&losses);
        let registration_started = Arc::clone(&registration_started);
        let release_registration = Arc::clone(&release_registration);
        std::thread::spawn(move || {
            losses.record_with_registration_hooks(
                0,
                1,
                crate::audio::frame::GapCause::SinkUnavailable,
                || {
                    registration_started.wait();
                    release_registration.wait();
                },
                || {},
            );
        })
    };
    registration_started.wait();

    let (packet_tx, packet_rx) = mpsc::sync_channel(1);
    let (returned_tx, _) = mpsc::sync_channel(8);
    let (error_tx, error_rx) = mpsc::sync_channel::<cpal::StreamError>(1);
    let ports = CapturePorts {
        packets: packet_rx,
        returned_buffers: returned_tx,
        losses,
    };
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_capture_packet_loop_with_timeout(
            ports,
            error_rx,
            Duration::from_secs(1),
            |_, _| false,
            |_| false,
            |_| false,
        );
        done_tx.send(()).unwrap();
    });

    drop(packet_tx);
    drop(error_tx);
    let exited = done_rx.recv_timeout(Duration::from_secs(1));

    release_registration.wait();
    callback.join().unwrap();
    worker.join().unwrap();
    assert!(exited.is_ok());
}

#[test]
fn accumulator_error_degrades_recording_before_runtime_exit() {
    let (packet_tx, packet_rx) = mpsc::sync_channel(1);
    let (returned_tx, _) = mpsc::sync_channel(1);
    let (error_tx, error_rx) = mpsc::sync_channel::<cpal::StreamError>(1);
    let losses = Arc::new(LossAccumulator::new());
    losses.invalidate();
    let ports = CapturePorts {
        packets: packet_rx,
        returned_buffers: returned_tx,
        losses,
    };
    let (coordinator, recording, _recording_rx) = capture_loss_coordinator();
    drop(packet_tx);
    drop(error_tx);

    run_capture_packet_loop_with_timeout(
        ports,
        error_rx,
        Duration::from_millis(1),
        |_, _| false,
        |_| false,
        |loss| process_capture_loss(&coordinator, loss, |_| {}),
    );

    assert_eq!(
        recording.outcome().error.as_deref(),
        Some("Capture loss timing failed: InvalidTiming")
    );
}

#[test]
fn pending_loss_at_shutdown_is_bounded_and_degrades_recording() {
    let losses = Arc::new(LossAccumulator::new());
    let registration_started = Arc::new(Barrier::new(2));
    let release_registration = Arc::new(Barrier::new(2));
    let callback = {
        let losses = Arc::clone(&losses);
        let registration_started = Arc::clone(&registration_started);
        let release_registration = Arc::clone(&release_registration);
        std::thread::spawn(move || {
            losses.record_with_registration_hooks(
                0,
                1,
                crate::audio::frame::GapCause::SinkUnavailable,
                || {
                    registration_started.wait();
                },
                || {
                    release_registration.wait();
                },
            );
        })
    };
    registration_started.wait();
    let (packet_tx, packet_rx) = mpsc::sync_channel(1);
    let (returned_tx, _) = mpsc::sync_channel(1);
    let (error_tx, error_rx) = mpsc::sync_channel::<cpal::StreamError>(1);
    let ports = CapturePorts {
        packets: packet_rx,
        returned_buffers: returned_tx,
        losses,
    };
    let (coordinator, recording, _recording_rx) = capture_loss_coordinator();
    let (done_tx, done_rx) = mpsc::channel();
    drop(packet_tx);
    drop(error_tx);
    let worker = std::thread::spawn(move || {
        run_capture_packet_loop_with_timeout(
            ports,
            error_rx,
            Duration::from_millis(1),
            |_, _| false,
            |_| false,
            |loss| process_capture_loss(&coordinator, loss, |_| {}),
        );
        done_tx.send(()).unwrap();
    });

    let exited = done_rx.recv_timeout(Duration::from_secs(1));
    release_registration.wait();
    callback.join().unwrap();
    worker.join().unwrap();

    assert!(
        exited.is_ok(),
        "shutdown loss drain must have a fixed wait bound"
    );
    assert_eq!(
        recording.outcome().error.as_deref(),
        Some("Capture loss timing failed: DrainIncomplete")
    );
}

#[test]
fn final_loss_drain_failure_degrades_recording() {
    let (packet_tx, packet_rx) = mpsc::sync_channel(1);
    let (returned_tx, _) = mpsc::sync_channel(1);
    let (error_tx, error_rx) = mpsc::sync_channel::<cpal::StreamError>(1);
    let ports = CapturePorts {
        packets: packet_rx,
        returned_buffers: returned_tx,
        losses: Arc::new(LossAccumulator::new()),
    };
    packet_tx
        .send(CapturePacket {
            source_position_frames: 0,
            channels: 1,
            sample_rate_hz: 16_000,
            samples: vec![0.0],
        })
        .unwrap();
    drop(packet_tx);
    drop(error_tx);
    let (coordinator, recording, _recording_rx) = capture_loss_coordinator();

    run_capture_packet_loop_with_timeout(
        ports,
        error_rx,
        Duration::from_millis(1),
        |_, losses| {
            losses.invalidate();
            true
        },
        |_| false,
        |loss| process_capture_loss(&coordinator, loss, |_| {}),
    );

    assert_eq!(
        recording.outcome().error.as_deref(),
        Some("Capture loss timing failed: InvalidTiming")
    );
}

#[test]
fn capture_packet_loop_periodically_drains_sustained_losses_with_honest_positions() {
    let (mut callback, ports) = new_callback_boundary(2, 48_000, 2, 0, 1_000).unwrap();
    let (error_tx, error_rx) = mpsc::sync_channel::<cpal::StreamError>(1);
    let (loss_tx, loss_rx) = mpsc::channel();
    let (packet_started_tx, packet_started_rx) = mpsc::channel();
    let (release_packet_tx, release_packet_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_capture_packet_loop_with_timeout(
            ports,
            error_rx,
            Duration::from_millis(1),
            move |_, _| {
                packet_started_tx.send(()).unwrap();
                release_packet_rx.recv().unwrap();
                false
            },
            |_| false,
            move |loss| loss_tx.send(loss).is_err(),
        );
    });

    let mut next_source_position = 1_000_u64;
    for _ in 0..64 {
        loop {
            callback.write_f32_for_test(&[0.0_f32, 0.0]);
            next_source_position += 1;
            if packet_started_rx
                .recv_timeout(Duration::from_millis(10))
                .is_ok()
            {
                break;
            }
            let snapshot = loss_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap();
            assert_eq!(
                snapshot.first_source_position_frames,
                next_source_position - 1
            );
            assert_eq!(snapshot.dropped_frames, 1);
        }

        let first_lost_position = next_source_position;
        for _ in 0..8 {
            callback.write_f32_for_test(&[0.0_f32, 0.0]);
            next_source_position += 1;
        }
        release_packet_tx.send(()).unwrap();
        let snapshot = loss_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.first_source_position_frames, first_lost_position);
        assert_eq!(snapshot.dropped_frames, 8);
        assert_eq!(
            snapshot.cause,
            crate::audio::frame::GapCause::SinkUnavailable
        );
    }

    drop(callback);
    drop(error_tx);
    worker.join().unwrap();
}

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
