use super::*;

#[test]
fn sustained_loss_past_run_and_ticket_capacity_never_panics_or_hangs() {
    let losses = LossAccumulator::new();
    let records = u64::try_from(super::REGISTRATION_TICKET_CAPACITY * 3).unwrap();

    for position in 0..records {
        losses.record(position, 1, GapCause::CallbackPoolExhausted);
    }

    assert_eq!(losses.drain(), Err(TimelineError::InvalidTiming));
    losses.record(records, 1, GapCause::CallbackPoolExhausted);
    assert_eq!(
        losses.drain(),
        Ok(Some(LossSnapshot {
            first_source_position_frames: records,
            dropped_frames: 1,
            cause: GapCause::CallbackPoolExhausted,
            generation: 1,
        }))
    );
}

#[test]
fn ticket_capacity_exhaustion_preserves_a_held_callback_across_the_old_reset_window() {
    let losses = Arc::new(LossAccumulator::new());
    let capacity = u64::try_from(super::REGISTRATION_TICKET_CAPACITY).unwrap();
    for position in 0..capacity - 1 {
        losses.record(position, 1, GapCause::CallbackPoolExhausted);
    }

    let registration_started = Arc::new(Barrier::new(2));
    let release_registration = Arc::new(Barrier::new(2));
    let held = {
        let losses = Arc::clone(&losses);
        let registration_started = Arc::clone(&registration_started);
        let release_registration = Arc::clone(&release_registration);
        thread::spawn(move || {
            losses.record_with_registration_hooks(
                capacity - 1,
                1,
                GapCause::CallbackPoolExhausted,
                || {
                    registration_started.wait();
                    release_registration.wait();
                },
                || {},
            );
        })
    };
    registration_started.wait();

    losses.record(capacity, 1, GapCause::CallbackPoolExhausted);
    assert_eq!(losses.try_drain(), Ok(super::TryDrain::Pending));
    assert_eq!(losses.active_generation.load(Ordering::SeqCst), 1);
    assert_eq!(losses.try_drain(), Ok(super::TryDrain::Pending));
    assert_eq!(losses.active_generation.load(Ordering::SeqCst), 1);

    release_registration.wait();
    held.join().unwrap();

    assert_eq!(losses.try_drain(), Err(TimelineError::InvalidTiming));
    assert_eq!(losses.registration_started.load(Ordering::SeqCst), capacity);
    assert_eq!(losses.registration_drained.load(Ordering::SeqCst), capacity);
    assert_eq!(
        losses.registration_completion_tickets
            [(capacity - 1) as usize % super::REGISTRATION_TICKET_CAPACITY]
            .load(Ordering::SeqCst),
        capacity
    );

    losses.record(capacity, 1, GapCause::CallbackPoolExhausted);
    assert_eq!(
        losses.try_drain(),
        Ok(super::TryDrain::Snapshot(LossSnapshot {
            first_source_position_frames: capacity - 1,
            dropped_frames: 2,
            cause: GapCause::CallbackPoolExhausted,
            generation: 1,
        }))
    );
    assert_eq!(losses.try_drain(), Ok(super::TryDrain::Empty));
}

#[test]
fn try_drain_returns_pending_without_waiting_for_a_pre_target_registration() {
    let losses = Arc::new(LossAccumulator::new());
    losses.record(0, 10, GapCause::CallbackPoolExhausted);
    let registration_started = Arc::new(Barrier::new(2));
    let release_registration = Arc::new(Barrier::new(2));

    let callback = {
        let losses = Arc::clone(&losses);
        let registration_started = Arc::clone(&registration_started);
        let release_registration = Arc::clone(&release_registration);
        thread::spawn(move || {
            losses.record_with_registration_hooks(
                10,
                10,
                GapCause::CallbackPoolExhausted,
                || {
                    registration_started.wait();
                    release_registration.wait();
                },
                || {},
            );
        })
    };
    registration_started.wait();

    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let coordinator = {
        let losses = Arc::clone(&losses);
        thread::spawn(move || result_tx.send(losses.try_drain()).unwrap())
    };
    let pending = result_rx.recv_timeout(std::time::Duration::from_secs(1));

    release_registration.wait();
    callback.join().unwrap();
    coordinator.join().unwrap();

    assert_eq!(pending.unwrap(), Ok(super::TryDrain::Pending));
    assert_eq!(
        losses.try_drain(),
        Ok(super::TryDrain::Snapshot(LossSnapshot {
            first_source_position_frames: 0,
            dropped_frames: 10,
            cause: GapCause::CallbackPoolExhausted,
            generation: 0,
        }))
    );
}

#[test]
fn held_post_flip_entrant_cannot_delay_the_fixed_pending_old_generation() {
    let losses = Arc::new(LossAccumulator::new());
    let pre_flip_generation_read = Arc::new(Barrier::new(2));
    let release_pre_flip = Arc::new(Barrier::new(2));

    let pre_flip = {
        let losses = Arc::clone(&losses);
        let pre_flip_generation_read = Arc::clone(&pre_flip_generation_read);
        let release_pre_flip = Arc::clone(&release_pre_flip);
        thread::spawn(move || {
            losses.record_with_registration_hooks(
                0,
                10,
                GapCause::CallbackPoolExhausted,
                || {},
                || {
                    pre_flip_generation_read.wait();
                    release_pre_flip.wait();
                },
            );
        })
    };
    pre_flip_generation_read.wait();
    assert_eq!(losses.try_drain(), Ok(super::TryDrain::Pending));

    let post_flip_started = Arc::new(Barrier::new(2));
    let release_post_flip = Arc::new(Barrier::new(2));
    let post_flip = {
        let losses = Arc::clone(&losses);
        let post_flip_started = Arc::clone(&post_flip_started);
        let release_post_flip = Arc::clone(&release_post_flip);
        thread::spawn(move || {
            losses.record_with_registration_hooks(
                10,
                10,
                GapCause::CallbackPoolExhausted,
                || {
                    post_flip_started.wait();
                    release_post_flip.wait();
                },
                || {},
            );
        })
    };
    post_flip_started.wait();
    release_pre_flip.wait();
    pre_flip.join().unwrap();

    assert_eq!(
        losses.try_drain(),
        Ok(super::TryDrain::Snapshot(LossSnapshot {
            first_source_position_frames: 0,
            dropped_frames: 10,
            cause: GapCause::CallbackPoolExhausted,
            generation: 0,
        }))
    );

    release_post_flip.wait();
    post_flip.join().unwrap();

    assert_eq!(
        losses.try_drain(),
        Ok(super::TryDrain::Snapshot(LossSnapshot {
            first_source_position_frames: 10,
            dropped_frames: 10,
            cause: GapCause::CallbackPoolExhausted,
            generation: 1,
        }))
    );
    assert_eq!(losses.try_drain(), Ok(super::TryDrain::Empty));
}

#[test]
fn try_drain_does_not_flip_a_second_generation_while_pending() {
    let losses = Arc::new(LossAccumulator::new());
    let registration_started = Arc::new(Barrier::new(2));
    let release_registration = Arc::new(Barrier::new(2));
    let callback = {
        let losses = Arc::clone(&losses);
        let registration_started = Arc::clone(&registration_started);
        let release_registration = Arc::clone(&release_registration);
        thread::spawn(move || {
            losses.record_with_registration_hooks(
                0,
                10,
                GapCause::CallbackPoolExhausted,
                || {
                    registration_started.wait();
                    release_registration.wait();
                },
                || {},
            );
        })
    };
    registration_started.wait();

    assert_eq!(losses.try_drain(), Ok(super::TryDrain::Pending));
    assert_eq!(losses.active_generation.load(Ordering::SeqCst), 1);
    assert_eq!(losses.try_drain(), Ok(super::TryDrain::Pending));
    assert_eq!(losses.active_generation.load(Ordering::SeqCst), 1);

    release_registration.wait();
    callback.join().unwrap();
    assert_eq!(losses.try_drain(), Ok(super::TryDrain::Empty));
}

#[test]
fn concurrent_try_drain_returns_pending_while_the_coordinator_is_contended() {
    let losses = LossAccumulator::new();
    let _coordinator = losses.coordinator.lock().unwrap();

    assert_eq!(losses.try_drain(), Ok(super::TryDrain::Pending));
}

#[test]
fn poisoned_coordinator_mutex_returns_invalid_timing_without_panicking() {
    let losses = Arc::new(LossAccumulator::new());
    let poisoner = {
        let losses = Arc::clone(&losses);
        thread::spawn(move || {
            let _coordinator = losses.coordinator.lock().unwrap();
            panic!("synthetic coordinator poison");
        })
    };
    assert!(poisoner.join().is_err());

    assert_eq!(losses.try_drain(), Err(TimelineError::InvalidTiming));
}

#[test]
fn draining_an_empty_accumulator_returns_none() {
    let losses = LossAccumulator::new();

    assert_eq!(losses.drain(), Ok(None));
    losses.record(10, 0, GapCause::SinkUnavailable);
    assert_eq!(losses.drain(), Ok(None));
}
