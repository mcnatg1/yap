use super::*;

#[test]
fn source_positions_and_losses_leave_a_timeline_gap() {
    let (ports, recording_rx, _) = ports(4, None);
    let mut coordinator = Coordinator::new(session(), track(), ports);
    let losses = Arc::new(LossAccumulator::new());
    losses.record(0, 480, GapCause::SinkUnavailable);

    coordinator.consume(&packet(480), &losses).unwrap();

    let gap = recv_recording_gap(&recording_rx);
    let frame = recv_recording_frame(&recording_rx);
    assert_eq!(frame.metadata.start_ms, 10);
    assert_eq!((gap.start_ms, gap.duration_ms), (0, 10));
}

#[test]
fn initial_losses_use_the_first_packet_clock_without_inventing_elapsed_time() {
    let (ports, recording_rx, _) = ports(4, None);
    let mut coordinator = Coordinator::new(session(), track(), ports);
    let losses = LossAccumulator::new();
    losses.record(1_000, 480, GapCause::SinkUnavailable);

    coordinator.consume(&packet(1_480), &losses).unwrap();

    let gap = recv_recording_gap(&recording_rx);
    let frame = recv_recording_frame(&recording_rx);
    assert_eq!(frame.metadata.start_ms, 10);
    assert_eq!((gap.start_ms, gap.duration_ms), (0, 10));
    assert_eq!(gap.source_position_frames, 1_000);
}

#[test]
fn configuration_rejection_without_pending_loss_degrades_recording() {
    let (directory, mut coordinator, recording) =
        persistent_coordinator("configuration-rejection-no-loss");
    let rejection = coordinator
        .consume(
            &CapturePacket {
                sample_rate_hz: 0,
                ..packet(0)
            },
            &LossAccumulator::new(),
        )
        .unwrap_err();

    assert_rejection_is_recording_terminal(
        directory,
        coordinator,
        recording,
        rejection,
        "Invalid microphone configuration.",
    );
}

#[test]
fn malformed_packet_rejection_degrades_recording() {
    let (directory, mut coordinator, recording) =
        persistent_coordinator("malformed-packet-rejection");
    let rejection = coordinator
        .consume(
            &CapturePacket {
                source_position_frames: 0,
                channels: 2,
                sample_rate_hz: 48_000,
                samples: vec![0.0; 3],
            },
            &LossAccumulator::new(),
        )
        .unwrap_err();

    assert_rejection_is_recording_terminal(
        directory,
        coordinator,
        recording,
        rejection,
        "Invalid captured audio packet.",
    );
}

#[test]
fn timeline_frame_rejection_degrades_recording() {
    let (directory, mut coordinator, recording) =
        persistent_coordinator("timeline-frame-rejection");
    coordinator
        .consume(&packet(0), &LossAccumulator::new())
        .unwrap();
    let rejection = coordinator
        .consume(&packet(0), &LossAccumulator::new())
        .unwrap_err();

    assert_rejection_is_recording_terminal(
        directory,
        coordinator,
        recording,
        rejection,
        "Capture timeline frame failed: InvalidTiming",
    );
}

#[test]
fn timeline_frame_end_overflow_degrades_recording() {
    let (directory, mut coordinator, recording) =
        persistent_coordinator("timeline-frame-end-overflow");
    coordinator
        .ensure_configuration(&packet(0), 0, u64::MAX - 1)
        .unwrap();
    let rejection = coordinator
        .consume(&packet(0), &LossAccumulator::new())
        .unwrap_err();

    assert_rejection_is_recording_terminal(
        directory,
        coordinator,
        recording,
        rejection,
        "Capture timeline frame failed: InvalidTiming",
    );
}

#[test]
fn pending_loss_followed_by_configuration_failure_cannot_publish_complete() {
    let directory = std::env::temp_dir().join(format!(
        "yap-pending-loss-configuration-failure-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&directory).ok();
    std::fs::create_dir_all(&directory).unwrap();
    let session = SessionId::new("pending-loss-configuration-failure").unwrap();
    let (ports, recording_rx, _) = ports(RECORDING_QUEUE_CAPACITY, None);
    let recording = RecordingSinkHandle::spawn(
        directory.clone(),
        session.clone(),
        ports.recording.clone(),
        recording_rx,
    );
    let mut coordinator = Coordinator::new(session.clone(), track(), ports);

    coordinator
        .consume_loss(LossSnapshot {
            first_source_position_frames: 0,
            dropped_frames: 1_600,
            cause: GapCause::CallbackPoolExhausted,
            generation: 1,
        })
        .unwrap();
    assert!(coordinator
        .consume(
            &CapturePacket {
                source_position_frames: 1_600,
                channels: 2,
                sample_rate_hz: 0,
                samples: vec![0.0; 960],
            },
            &LossAccumulator::new(),
        )
        .is_err());
    coordinator
        .consume(&packet(1_600), &LossAccumulator::new())
        .unwrap();
    let degradation = coordinator.outcome(SinkKind::Recording).unwrap().error;
    coordinator.close();
    let result = recording.finalize().unwrap();

    assert_eq!(
        degradation.as_deref(),
        Some("Invalid microphone configuration.")
    );
    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
    assert!(
        std::fs::metadata(directory.join(format!("live-{session}.wav.part")))
            .unwrap()
            .len()
            > 44
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn loss_application_failure_cannot_publish_complete() {
    let directory = std::env::temp_dir().join(format!(
        "yap-loss-application-failure-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&directory).ok();
    std::fs::create_dir_all(&directory).unwrap();
    let session = SessionId::new("loss-application-failure").unwrap();
    let (ports, recording_rx, _) = ports(RECORDING_QUEUE_CAPACITY, None);
    let recording = RecordingSinkHandle::spawn(
        directory.clone(),
        session.clone(),
        ports.recording.clone(),
        recording_rx,
    );
    let mut coordinator = Coordinator::new(session.clone(), track(), ports);

    coordinator
        .consume(&packet(0), &LossAccumulator::new())
        .unwrap();
    assert!(coordinator
        .consume_loss(LossSnapshot {
            first_source_position_frames: 0,
            dropped_frames: 480,
            cause: GapCause::DeviceDiscontinuity,
            generation: 1,
        })
        .is_err());
    coordinator
        .consume(&packet(480), &LossAccumulator::new())
        .unwrap();
    let degradation = coordinator.outcome(SinkKind::Recording).unwrap().error;
    coordinator.close();
    let result = recording.finalize().unwrap();

    assert_eq!(
        degradation.as_deref(),
        Some("Capture timeline gap failed: InvalidTiming")
    );
    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
    assert!(
        std::fs::metadata(directory.join(format!("live-{session}.wav.part")))
            .unwrap()
            .len()
            > 44
    );
    std::fs::remove_dir_all(directory).unwrap();
}
