use super::*;

#[test]
fn saturated_handoff_reports_the_exact_dropped_interval() {
    let losses = LossAccumulator::new();
    losses.record(320, 160, GapCause::CallbackPoolExhausted);
    losses.record(480, 320, GapCause::CallbackPoolExhausted);

    assert_eq!(
        losses.drain(),
        Ok(Some(LossSnapshot {
            first_source_position_frames: 320,
            dropped_frames: 480,
            cause: GapCause::CallbackPoolExhausted,
            generation: 0,
        }))
    );
}

#[test]
fn unwound_ticket_is_terminal_invalid_instead_of_pending_forever() {
    let losses = LossAccumulator::new();

    assert!(catch_unwind(AssertUnwindSafe(|| {
        losses.record_with_registration_hooks(
            320,
            160,
            GapCause::CallbackPoolExhausted,
            || panic!("synthetic callback unwind"),
            || {},
        );
    }))
    .is_err());

    assert_eq!(losses.try_drain(), Err(TimelineError::InvalidTiming));
}

#[test]
fn reversed_position_contiguous_writers_report_the_exact_union() {
    assert_eq!(
        concurrently_record([
            (480, 320, GapCause::CallbackPoolExhausted),
            (320, 160, GapCause::CallbackPoolExhausted),
        ]),
        Ok(Some(LossSnapshot {
            first_source_position_frames: 320,
            dropped_frames: 480,
            cause: GapCause::CallbackPoolExhausted,
            generation: 0,
        }))
    );
}

#[test]
fn non_contiguous_writers_never_fabricate_the_missing_interval() {
    assert_eq!(
        concurrently_record([
            (320, 160, GapCause::CallbackPoolExhausted),
            (640, 160, GapCause::CallbackPoolExhausted),
        ]),
        Err(TimelineError::InvalidTiming)
    );
}

#[test]
fn overlapping_writers_invalidate_the_snapshot() {
    assert_eq!(
        concurrently_record([
            (320, 320, GapCause::CallbackPoolExhausted),
            (480, 320, GapCause::CallbackPoolExhausted),
        ]),
        Err(TimelineError::InvalidTiming)
    );
}

#[test]
fn different_writer_causes_invalidate_the_snapshot() {
    assert_eq!(
        concurrently_record([
            (320, 160, GapCause::CallbackPoolExhausted),
            (480, 160, GapCause::SinkUnavailable),
        ]),
        Err(TimelineError::InvalidTiming)
    );
}

#[test]
fn overlap_and_hole_cannot_cancel_into_an_exact_span() {
    assert_eq!(
        concurrently_record([
            (0, 10, GapCause::CallbackPoolExhausted),
            (5, 10, GapCause::CallbackPoolExhausted),
            (20, 5, GapCause::CallbackPoolExhausted),
        ]),
        Err(TimelineError::InvalidTiming)
    );
}

#[test]
fn reversed_order_contiguous_multi_writer_run_is_exact() {
    assert_eq!(
        concurrently_record([
            (20, 5, GapCause::DeviceDiscontinuity),
            (10, 10, GapCause::DeviceDiscontinuity),
            (0, 10, GapCause::DeviceDiscontinuity),
        ]),
        Ok(Some(LossSnapshot {
            first_source_position_frames: 0,
            dropped_frames: 25,
            cause: GapCause::DeviceDiscontinuity,
            generation: 0,
        }))
    );
}

#[test]
fn loss_run_capacity_exhaustion_is_invalid_timing() {
    let losses = LossAccumulator::new();
    let capacity = u64::try_from(super::LOSS_RUN_CAPACITY).unwrap();
    for position in 0..=capacity {
        losses.record(position, 1, GapCause::CallbackPoolExhausted);
    }

    assert_eq!(losses.drain(), Err(TimelineError::InvalidTiming));
}

#[test]
fn checked_end_and_dropped_sum_overflow_invalidate_the_snapshot() {
    let end_overflow = LossAccumulator::new();
    end_overflow.record(u64::MAX - 1, 2, GapCause::DeviceDiscontinuity);
    assert_eq!(end_overflow.drain(), Err(TimelineError::InvalidTiming));

    let sum_overflow = LossAccumulator::new();
    sum_overflow.record(0, u64::MAX, GapCause::DeviceDiscontinuity);
    sum_overflow.record(0, 1, GapCause::DeviceDiscontinuity);
    assert_eq!(sum_overflow.drain(), Err(TimelineError::InvalidTiming));
}

#[test]
fn registration_counter_exhaustion_is_permanently_invalid() {
    let losses = LossAccumulator::new();
    losses
        .registration_started
        .store(u64::MAX, Ordering::Relaxed);

    losses.record(0, 1, GapCause::CallbackPoolExhausted);
    assert_eq!(losses.drain(), Err(TimelineError::InvalidTiming));

    losses.record(1, 1, GapCause::CallbackPoolExhausted);
    assert_eq!(losses.drain(), Err(TimelineError::InvalidTiming));
}

#[test]
fn generation_exhaustion_is_terminal_without_reusing_slots() {
    let losses = LossAccumulator::new();
    losses.active_generation.store(u64::MAX, Ordering::SeqCst);

    assert_eq!(losses.try_drain(), Err(TimelineError::InvalidTiming));
    assert_eq!(losses.active_generation.load(Ordering::SeqCst), u64::MAX);

    let registration_hook_called = AtomicBool::new(false);
    let generation_hook_called = AtomicBool::new(false);
    losses.record_with_registration_hooks(
        0,
        1,
        GapCause::CallbackPoolExhausted,
        || registration_hook_called.store(true, Ordering::SeqCst),
        || generation_hook_called.store(true, Ordering::SeqCst),
    );

    assert!(!registration_hook_called.load(Ordering::SeqCst));
    assert!(!generation_hook_called.load(Ordering::SeqCst));
    assert_eq!(losses.registration_started.load(Ordering::SeqCst), 0);
    assert_eq!(losses.slots[0].claimed_runs.load(Ordering::SeqCst), 0);
    assert_eq!(losses.slots[1].claimed_runs.load(Ordering::SeqCst), 0);
    assert_eq!(losses.try_drain(), Err(TimelineError::InvalidTiming));
    assert_eq!(losses.drain(), Err(TimelineError::InvalidTiming));
    assert_eq!(losses.active_generation.load(Ordering::SeqCst), u64::MAX);
}

#[test]
fn generation_can_advance_to_max_once_before_becoming_terminal() {
    let losses = LossAccumulator::new();
    losses
        .active_generation
        .store(u64::MAX - 1, Ordering::SeqCst);
    losses.record(0, 1, GapCause::CallbackPoolExhausted);

    assert_eq!(
        losses.drain(),
        Ok(Some(LossSnapshot {
            first_source_position_frames: 0,
            dropped_frames: 1,
            cause: GapCause::CallbackPoolExhausted,
            generation: u64::MAX - 1,
        }))
    );
    assert_eq!(losses.active_generation.load(Ordering::SeqCst), u64::MAX);

    losses.record(1, 1, GapCause::CallbackPoolExhausted);
    assert_eq!(losses.try_drain(), Err(TimelineError::InvalidTiming));
    assert_eq!(losses.active_generation.load(Ordering::SeqCst), u64::MAX);
}
