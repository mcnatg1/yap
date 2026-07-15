use super::*;

#[test]
fn sustained_preconfiguration_loss_is_bounded_and_finalizes_partial_with_audio() {
    const EXPECTED_PENDING_LOSS_BOUND: usize = 64;
    const LOSS_COUNT: usize = EXPECTED_PENDING_LOSS_BOUND * 4;
    const LOSS_FRAMES: u64 = 240;

    let directory =
        std::env::temp_dir().join(format!("yap-bounded-pending-loss-{}", std::process::id()));
    std::fs::remove_dir_all(&directory).ok();
    std::fs::create_dir_all(&directory).unwrap();
    let session = SessionId::new("bounded-pending-loss").unwrap();
    let (ports, recording_rx, _) = ports(RECORDING_QUEUE_CAPACITY, None);
    let recording = RecordingSinkHandle::spawn(
        directory.clone(),
        session.clone(),
        ports.recording.clone(),
        recording_rx,
    );
    let mut coordinator = Coordinator::new(session.clone(), track(), ports);

    for index in 0..LOSS_COUNT {
        coordinator
            .consume_loss(LossSnapshot {
                first_source_position_frames: index as u64 * LOSS_FRAMES,
                dropped_frames: LOSS_FRAMES,
                cause: if index.is_multiple_of(2) {
                    GapCause::CallbackPoolExhausted
                } else {
                    GapCause::DeviceDiscontinuity
                },
                generation: index as u64 + 1,
            })
            .unwrap();
    }
    let pending_count = coordinator.pending_losses.len();
    let degraded = coordinator.outcome(SinkKind::Recording).unwrap().error;
    coordinator
        .consume(
            &packet(LOSS_COUNT as u64 * LOSS_FRAMES),
            &LossAccumulator::new(),
        )
        .unwrap();
    let degradation_after_audio = coordinator.outcome(SinkKind::Recording).unwrap().error;
    coordinator.close();
    let result = recording.finalize().unwrap();

    assert!(pending_count <= EXPECTED_PENDING_LOSS_BOUND);
    assert_eq!(
        degraded.as_deref(),
        Some("recording pending-loss capacity exhausted")
    );
    assert_eq!(degradation_after_audio, degraded);
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
fn coordinator_loss_drain_has_a_fixed_pending_retry_bound() {
    let losses = Arc::new(LossAccumulator::new());
    let registration_started = Arc::new(Barrier::new(2));
    let release_registration = Arc::new(Barrier::new(2));
    let loss_writer = {
        let losses = Arc::clone(&losses);
        let registration_started = Arc::clone(&registration_started);
        let release_registration = Arc::clone(&release_registration);
        std::thread::spawn(move || {
            losses.record_with_registration_hooks(
                0,
                480,
                GapCause::SinkUnavailable,
                || {
                    registration_started.wait();
                    release_registration.wait();
                },
                || {},
            );
        })
    };
    registration_started.wait();
    let (ports, _recording_rx, _) = ports(4, None);
    let coordinator = Coordinator::new(session(), track(), ports);
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let worker = {
        let losses = Arc::clone(&losses);
        std::thread::spawn(move || {
            let mut coordinator = coordinator;
            let result = coordinator.consume(&packet(480), &losses);
            result_tx.send(result).unwrap();
            coordinator
        })
    };

    let observed = result_rx.recv_timeout(Duration::from_millis(250));
    release_registration.wait();
    loss_writer.join().unwrap();
    let coordinator = worker.join().unwrap();

    let error = observed
        .expect("coordinator loss drain must stop at its retry bound")
        .unwrap_err();
    assert_eq!(error, "Capture loss drain did not quiesce.");
    assert_eq!(
        coordinator
            .outcome(SinkKind::Recording)
            .unwrap()
            .error
            .as_deref(),
        Some(error.as_str())
    );
}

#[test]
fn sustained_frames_stream_to_disk_with_constant_timeline_retention() {
    const FRAME_COUNT: usize = 1_024;

    let directory = std::env::temp_dir().join(format!(
        "yap-constant-timeline-retention-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&directory).ok();
    std::fs::create_dir_all(&directory).unwrap();
    let session = SessionId::new("constant-timeline-retention").unwrap();
    let (recording_sink, recording_rx) = bounded_sink(SinkKind::Recording, FRAME_COUNT + 2);
    let recording = RecordingSinkHandle::spawn(
        directory.clone(),
        session.clone(),
        recording_sink.clone(),
        recording_rx,
    );
    let mut coordinator = Coordinator::new(
        session.clone(),
        track(),
        CoordinatorPorts {
            recording: recording_sink,
            local_asr: None,
            speaker_evidence: None,
            server_transport: None,
        },
    );

    for index in 0..FRAME_COUNT {
        coordinator
            .consume(&packet(index as u64 * 480), &LossAccumulator::new())
            .unwrap();
    }
    let retained = retained_timeline_metadata(&coordinator);
    let outcome = coordinator.outcome(SinkKind::Recording).unwrap();
    coordinator.close();
    let result = recording.finalize().unwrap();
    let audio_bytes = std::fs::metadata(directory.join(format!("live-{session}.wav")))
        .unwrap()
        .len();
    std::fs::remove_dir_all(directory).unwrap();

    assert!(retained <= 1, "retained timeline metadata={retained}");
    assert_eq!(outcome.dropped_frames, 0);
    assert_eq!(result.status, CaptureStatus::Complete);
    assert!(audio_bytes > 44);
}

#[test]
fn losses_before_a_rate_change_use_the_old_clock_before_new_revisions() {
    let (ports, recording_rx, _) = ports(8, None);
    let mut coordinator = Coordinator::new(session(), track(), ports);
    let losses = LossAccumulator::new();

    coordinator.consume(&packet(0), &losses).unwrap();
    losses.record(480, 480, GapCause::SinkUnavailable);
    coordinator
        .consume(
            &CapturePacket {
                sample_rate_hz: 44_100,
                ..packet(960)
            },
            &losses,
        )
        .unwrap();

    let inputs = recv_recording_inputs(&recording_rx, 5);
    let gap_index = inputs
        .iter()
        .position(|input| matches!(input, RecordingInput::Gap(gap) if gap.start_ms == 10 && gap.duration_ms == 10))
        .unwrap();
    let new_revision_index = inputs
        .iter()
        .position(|input| matches!(input, RecordingInput::RevisionTransition(transition) if transition.configuration.revision == 2 && transition.configuration.effective_at_ms == 20 && transition.configuration.sample_rate_hz == 44_100 && transition.clock_mapping.source_position_frames == 960 && transition.clock_mapping.session_time_ms == 20))
        .unwrap();
    assert!(gap_index < new_revision_index);
    assert!(inputs.iter().any(
        |input| matches!(input, RecordingInput::PreparedFrame(frame) if frame.metadata.sequence == 1 && frame.metadata.start_ms == 20)
    ));
}

#[test]
fn pending_loss_registration_blocks_rate_change_until_the_old_clock_applies_it() {
    let (ports, recording_rx, _) = ports(8, None);
    let mut coordinator = Coordinator::new(session(), track(), ports);
    let losses = Arc::new(LossAccumulator::new());
    coordinator.consume(&packet(0), &losses).unwrap();

    let registration_started = Arc::new(Barrier::new(2));
    let release_registration = Arc::new(Barrier::new(2));
    let loss_writer = {
        let losses = Arc::clone(&losses);
        let registration_started = Arc::clone(&registration_started);
        let release_registration = Arc::clone(&release_registration);
        std::thread::spawn(move || {
            losses.record_with_registration_hooks(
                480,
                480,
                GapCause::SinkUnavailable,
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

    let rate_change_pending = Arc::new(Barrier::new(2));
    coordinator.set_loss_pending_hook_for_test({
        let rate_change_pending = Arc::clone(&rate_change_pending);
        Arc::new(move || {
            rate_change_pending.wait();
        })
    });
    std::thread::scope(|scope| {
        let consume = scope.spawn(|| {
            coordinator.consume(
                &CapturePacket {
                    sample_rate_hz: 44_100,
                    ..packet(960)
                },
                &losses,
            )
        });
        rate_change_pending.wait();
        release_registration.wait();
        consume.join().unwrap().unwrap();
    });
    loss_writer.join().unwrap();

    let inputs = recv_recording_inputs(&recording_rx, 5);
    let gap_index = inputs
        .iter()
        .position(|input| matches!(input, RecordingInput::Gap(gap) if gap.start_ms == 10 && gap.duration_ms == 10))
        .unwrap();
    let new_revision_index = inputs
        .iter()
        .position(|input| matches!(input, RecordingInput::RevisionTransition(transition) if transition.configuration.revision == 2))
        .unwrap();
    assert!(gap_index < new_revision_index);
}

#[test]
fn resampler_resets_follow_emitted_revision_events() {
    let (ports, _recording_rx, _) = ports(2, None);
    let mut coordinator = Coordinator::new(session(), track(), ports);
    let losses = LossAccumulator::new();

    coordinator.consume(&packet(0), &losses).unwrap();
    coordinator
        .consume(
            &CapturePacket {
                sample_rate_hz: 44_100,
                ..packet(480)
            },
            &losses,
        )
        .unwrap();

    assert_eq!(
        coordinator.revision_events(),
        &[
            RevisionEvent::TrackConfigured(1),
            RevisionEvent::ClockMapped(1),
            RevisionEvent::ResamplerReset {
                track_revision: 1,
                clock_revision: 1,
            },
            RevisionEvent::TrackConfigured(2),
            RevisionEvent::ClockMapped(2),
            RevisionEvent::ResamplerReset {
                track_revision: 2,
                clock_revision: 2,
            },
        ]
    );
}

#[test]
fn sink_workers_shutdown_after_ports_close() {
    let (ports, recording_rx, _) = ports(1, None);
    let mut coordinator = Coordinator::new(session(), track(), ports);
    let worker =
        std::thread::spawn(
            move || {
                while recording_rx.recv_timeout(Duration::from_millis(10)).is_ok() {}
            },
        );

    coordinator.close();

    worker.join().unwrap();
}
