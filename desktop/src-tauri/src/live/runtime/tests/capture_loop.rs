use super::*;

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
