use super::*;

#[test]
fn recording_continues_when_local_asr_is_absent() {
    let (ports, recording_rx, _) = ports(3, None);
    let mut coordinator = Coordinator::new(session(), track(), ports);
    let losses = LossAccumulator::new();

    coordinator.consume(&packet(0), &losses).unwrap();

    let frame = recv_recording_frame(&recording_rx);
    assert_eq!(frame.metadata.sample_rate_hz, 16_000);
    assert_eq!(frame.metadata.channels, 1);
    assert!(coordinator.outcome(SinkKind::LocalAsr).is_none());
}

#[test]
fn capture_boundary_rejects_each_non_finite_callback_as_an_exact_gap() {
    const FRAME_COUNT: usize = 320;
    const CHANNELS: usize = 2;
    const CALLBACK_SAMPLES: usize = FRAME_COUNT * CHANNELS;

    for malformed in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let (mut callback, capture) =
            new_callback_boundary(CHANNELS as u16, 16_000, CALLBACK_SAMPLES, 2, 0).unwrap();
        let (ports, recording_rx, local_asr_rx) = ports(4, Some(2));
        let mut coordinator = Coordinator::new(session(), track(), ports);

        let mut rejected = vec![0.25_f32; CALLBACK_SAMPLES];
        rejected[CALLBACK_SAMPLES / 2] = malformed;
        callback.write_f32_for_test(&rejected);
        callback.write_f32_for_test(&vec![0.5_f32; CALLBACK_SAMPLES]);

        let packet = capture.packets.recv().unwrap();
        assert_eq!(packet.source_position_frames, FRAME_COUNT as u64);
        assert!(packet.samples.iter().all(|sample| sample.is_finite()));
        assert!(matches!(
            capture.packets.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));

        coordinator.consume(&packet, &capture.losses).unwrap();

        let gap = recv_recording_gap(&recording_rx);
        assert_eq!(gap.source_position_frames, 0);
        assert_eq!(gap.dropped_frames, FRAME_COUNT as u64);
        assert_eq!(gap.start_ms, 0);
        assert_eq!(gap.duration_ms, 20);
        assert_eq!(gap.cause, GapCause::DeviceDiscontinuity);
        let recorded = recv_recording_frame(&recording_rx);
        let recognized = local_asr_rx
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert!(recorded.samples.iter().all(|sample| sample.is_finite()));
        assert!(recognized.samples.iter().all(|sample| sample.is_finite()));
        assert!(matches!(
            local_asr_rx
                .as_ref()
                .unwrap()
                .recv_timeout(Duration::from_millis(1)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
    }
}

#[test]
fn capture_boundary_normalizes_finite_f32_before_every_sink() {
    const FRAME_SAMPLES: usize = 320;
    let positive_denormal = f32::from_bits(1);
    let negative_denormal = -positive_denormal;
    let pattern = [
        -2.0,
        -1.0,
        negative_denormal,
        0.0,
        positive_denormal,
        1.0,
        2.0,
        0.25,
    ];
    let input = pattern
        .into_iter()
        .cycle()
        .take(FRAME_SAMPLES)
        .collect::<Vec<_>>();
    let expected = [-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.25];
    let (mut callback, capture) = new_callback_boundary(1, 16_000, FRAME_SAMPLES, 1, 0).unwrap();
    let (ports, recording_rx, local_asr_rx) = ports(3, Some(1));
    let mut coordinator = Coordinator::new(session(), track(), ports);

    callback.write_f32_for_test(&input);
    let packet = capture.packets.recv().unwrap();
    coordinator.consume(&packet, &capture.losses).unwrap();

    let recorded = recv_recording_frame(&recording_rx);
    let recognized = local_asr_rx
        .unwrap()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(&recorded.samples[..expected.len()], &expected);
    assert_eq!(&recognized.samples[..expected.len()], &expected);
    assert_eq!(capture.losses.drain(), Ok(None));
}

#[test]
fn stalled_asr_does_not_block_recording_or_callback_intake() {
    let (ports, recording_rx, local_asr_rx) = ports(4, Some(1));
    let coordinator = Coordinator::new(session(), track(), ports);
    let (completed_tx, completed_rx) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let mut coordinator = coordinator;
        let losses = LossAccumulator::new();
        coordinator.consume(&packet(0), &losses).unwrap();
        coordinator.consume(&packet(480), &losses).unwrap();
        completed_tx.send(coordinator).unwrap();
    });
    let coordinator = completed_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("stalled ASR blocked recording intake");
    worker.join().unwrap();

    assert_eq!(recv_recording_frame(&recording_rx).metadata.sequence, 0);
    assert_eq!(recv_recording_frame(&recording_rx).metadata.sequence, 1);
    assert_eq!(
        local_asr_rx
            .unwrap()
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .metadata
            .sequence,
        0
    );
    assert_eq!(
        coordinator
            .outcome(SinkKind::LocalAsr)
            .unwrap()
            .dropped_frames,
        1
    );
}

#[test]
fn one_sink_failure_does_not_close_other_sinks() {
    let (ports, recording_rx, local_asr_rx) = ports(3, Some(1));
    let mut coordinator = Coordinator::new(session(), track(), ports);
    let losses = LossAccumulator::new();
    coordinator.close_sink(SinkKind::LocalAsr);
    drop(local_asr_rx);

    coordinator.consume(&packet(0), &losses).unwrap();

    assert_eq!(recv_recording_frame(&recording_rx).metadata.sequence, 0);
    assert!(!coordinator.outcome(SinkKind::Recording).unwrap().closed);
    assert!(coordinator.outcome(SinkKind::LocalAsr).unwrap().closed);
}

#[test]
fn finalization_closes_every_sink_exactly_once() {
    let (ports, _, _) = ports(1, Some(1));
    let mut coordinator = Coordinator::new(session(), track(), ports);

    coordinator.close();
    coordinator.close();

    for outcome in coordinator.outcomes() {
        assert!(outcome.closed);
        assert_eq!(coordinator.close_count(outcome.kind), 1);
    }
}

#[test]
fn composed_result_marks_only_the_failed_or_degraded_sinks() {
    let (ports, recording_rx, local_asr_rx) = ports(3, Some(1));
    let mut coordinator = Coordinator::new(session(), track(), ports);
    let losses = LossAccumulator::new();
    coordinator.close_sink(SinkKind::LocalAsr);
    drop(local_asr_rx);

    coordinator.consume(&packet(0), &losses).unwrap();
    drop(recording_rx);

    let recording = coordinator.outcome(SinkKind::Recording).unwrap();
    let asr = coordinator.outcome(SinkKind::LocalAsr).unwrap();
    assert_eq!(recording.dropped_frames, 0);
    assert_eq!(recording.error, None);
    assert!(asr.closed);
    assert!(asr.error.is_some());
}

#[test]
fn queue_capacities_and_high_water_marks_are_visible() {
    assert_eq!(RECORDING_QUEUE_CAPACITY, 128);
    assert_eq!(LOCAL_ASR_QUEUE_CAPACITY, 64);
    assert_eq!(EVIDENCE_QUEUE_CAPACITY, 32);
    assert_eq!(SERVER_TRANSPORT_QUEUE_CAPACITY, 64);

    let (ports, _recording_rx, _) = ports(2, None);
    let mut coordinator = Coordinator::new(session(), track(), ports);
    let losses = LossAccumulator::new();
    coordinator.consume(&packet(0), &losses).unwrap();
    coordinator.consume(&packet(480), &losses).unwrap();

    assert_eq!(coordinator.high_water_mark(SinkKind::Recording), Some(2));
}

#[test]
fn queue_accounting_reserves_before_publish_and_rolls_back_failed_sends() {
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
    let published = Arc::new(Barrier::new(2));
    let release_sender = Arc::new(Barrier::new(2));
    let pause_once = Arc::new(std::sync::atomic::AtomicBool::new(true));
    sink.set_after_publish_hook_for_test({
        let published = Arc::clone(&published);
        let release_sender = Arc::clone(&release_sender);
        let pause_once = Arc::clone(&pause_once);
        Arc::new(move || {
            if pause_once.swap(false, std::sync::atomic::Ordering::SeqCst) {
                published.wait();
                release_sender.wait();
            }
        })
    });
    let sender = sink.clone();
    let worker = std::thread::spawn(move || sender.try_send(1_u8));

    published.wait();
    assert_eq!(receiver.recv_timeout(Duration::from_secs(1)).unwrap(), 1);
    release_sender.wait();
    assert!(worker.join().unwrap().is_ok());
    assert_eq!(sink.queued_frames_for_test(), 0);
    assert_eq!(sink.high_water_mark(), 1);

    assert!(sink.try_send(2).is_ok());
    assert!(matches!(sink.try_send(3), Err(super::SinkSendError::Full)));
    assert_eq!(sink.queued_frames_for_test(), 1);
    assert_eq!(sink.high_water_mark(), 1);
    assert_eq!(sink.outcome().accepted_frames, 2);
    assert_eq!(sink.outcome().dropped_frames, 1);
    drop(receiver);
    assert!(matches!(
        sink.try_send(4),
        Err(super::SinkSendError::Closed)
    ));
    assert_eq!(sink.queued_frames_for_test(), 0);
    assert_eq!(sink.high_water_mark(), 1);

    let (capacity_sink, _capacity_receiver) = bounded_sink(SinkKind::Recording, 2);
    assert!(capacity_sink.try_send(1).is_ok());
    assert!(capacity_sink.try_send(2).is_ok());
    assert!(matches!(
        capacity_sink.try_send(3),
        Err(super::SinkSendError::Full)
    ));
    assert_eq!(capacity_sink.high_water_mark(), 2);
    assert_eq!(capacity_sink.outcome().accepted_frames, 2);
}

#[test]
fn receive_claim_keeps_depth_and_high_water_at_capacity_during_a_send_interleaving() {
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
    sink.try_send(1_u8).unwrap();

    let received = Arc::new(Barrier::new(2));
    let release_receiver = Arc::new(Barrier::new(2));
    receiver.set_after_receive_hook_for_test({
        let received = Arc::clone(&received);
        let release_receiver = Arc::clone(&release_receiver);
        Arc::new(move || {
            received.wait();
            release_receiver.wait();
        })
    });
    let receiver_worker = std::thread::spawn(move || receiver.recv_timeout(Duration::from_secs(1)));

    received.wait();
    assert_eq!(sink.queued_frames_for_test(), 0);
    sink.try_send(2_u8).unwrap();
    assert_eq!(sink.queued_frames_for_test(), 1);
    assert_eq!(sink.high_water_mark(), 1);
    release_receiver.wait();

    assert_eq!(receiver_worker.join().unwrap().unwrap(), 1);
    assert_eq!(sink.high_water_mark(), 1);
}

#[test]
fn cloned_producers_preserve_exact_high_water_and_roll_back_failed_sends() {
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 2);
    let start = Arc::new(Barrier::new(3));
    let complete = Arc::new(Barrier::new(3));
    let first = sink.clone();
    let second = sink.clone();
    let first_start = Arc::clone(&start);
    let first_complete = Arc::clone(&complete);
    let first_worker = std::thread::spawn(move || {
        first_start.wait();
        let result = first.try_send(1_u8);
        first_complete.wait();
        result
    });
    let second_start = Arc::clone(&start);
    let second_complete = Arc::clone(&complete);
    let second_worker = std::thread::spawn(move || {
        second_start.wait();
        let result = second.try_send(2_u8);
        second_complete.wait();
        result
    });

    start.wait();
    complete.wait();
    assert!(first_worker.join().unwrap().is_ok());
    assert!(second_worker.join().unwrap().is_ok());
    assert_eq!(sink.high_water_mark(), 2);
    assert_eq!(sink.queued_frames_for_test(), 2);
    let mut received = [
        receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
        receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
    ];
    received.sort_unstable();
    assert_eq!(received, [1, 2]);
    assert_eq!(sink.queued_frames_for_test(), 0);

    let (full_sink, _full_receiver) = bounded_sink(SinkKind::Recording, 1);
    assert!(full_sink.try_send(1_u8).is_ok());
    assert!(matches!(
        full_sink.try_send(2_u8),
        Err(super::SinkSendError::Full)
    ));
    assert_eq!(full_sink.queued_frames_for_test(), 1);
    assert_eq!(full_sink.high_water_mark(), 1);

    let (disconnected_sink, disconnected_receiver) = bounded_sink(SinkKind::Recording, 1);
    drop(disconnected_receiver);
    assert!(matches!(
        disconnected_sink.try_send(1_u8),
        Err(super::SinkSendError::Closed)
    ));
    assert_eq!(disconnected_sink.queued_frames_for_test(), 0);
    assert_eq!(disconnected_sink.high_water_mark(), 0);
}
