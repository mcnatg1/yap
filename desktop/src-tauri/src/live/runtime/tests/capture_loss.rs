use super::*;

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
