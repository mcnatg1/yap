use super::clock::TimelineError;
use crate::audio::frame::GapCause;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};

const NO_CAUSE: u8 = 0;
pub(super) const LOSS_RUN_CAPACITY: usize = 64;
pub(super) const REGISTRATION_TICKET_CAPACITY: usize = LOSS_RUN_CAPACITY * 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LossSnapshot {
    pub first_source_position_frames: u64,
    pub dropped_frames: u64,
    pub cause: GapCause,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TryDrain {
    Pending,
    Empty,
    Snapshot(LossSnapshot),
}

#[derive(Debug)]
struct LossRunCell {
    source_position_frames: AtomicU64,
    dropped_frames: AtomicU64,
    cause: AtomicU8,
    published: AtomicBool,
}

impl LossRunCell {
    pub(super) const fn new() -> Self {
        Self {
            source_position_frames: AtomicU64::new(0),
            dropped_frames: AtomicU64::new(0),
            cause: AtomicU8::new(NO_CAUSE),
            published: AtomicBool::new(false),
        }
    }

    fn publish(&self, source_position_frames: u64, dropped_frames: u64, cause: GapCause) {
        self.source_position_frames
            .store(source_position_frames, Ordering::Relaxed);
        self.dropped_frames.store(dropped_frames, Ordering::Relaxed);
        self.cause.store(encode_cause(cause), Ordering::Relaxed);
        self.published.store(true, Ordering::Release);
    }

    fn take(&self) -> Option<RawLossRun> {
        if !self.published.load(Ordering::Acquire) {
            return None;
        }
        let run = RawLossRun {
            source_position_frames: self.source_position_frames.load(Ordering::Relaxed),
            dropped_frames: self.dropped_frames.load(Ordering::Relaxed),
            cause: self.cause.load(Ordering::Relaxed),
        };
        self.published.store(false, Ordering::Relaxed);
        Some(run)
    }
}

#[derive(Debug, Clone, Copy)]
struct RawLossRun {
    source_position_frames: u64,
    dropped_frames: u64,
    cause: u8,
}

const EMPTY_LOSS_RUN: RawLossRun = RawLossRun {
    source_position_frames: 0,
    dropped_frames: 0,
    cause: NO_CAUSE,
};

#[derive(Debug)]
pub(super) struct LossSlot {
    pub(super) writers: AtomicUsize,
    pub(super) claimed_runs: AtomicUsize,
    runs: [LossRunCell; LOSS_RUN_CAPACITY],
    invalid: AtomicBool,
}

impl LossSlot {
    pub(super) const fn new() -> Self {
        Self {
            writers: AtomicUsize::new(0),
            claimed_runs: AtomicUsize::new(0),
            runs: [const { LossRunCell::new() }; LOSS_RUN_CAPACITY],
            invalid: AtomicBool::new(false),
        }
    }

    pub(super) fn record(&self, source_position_frames: u64, dropped_frames: u64, cause: GapCause) {
        let run_index = self.claimed_runs.fetch_add(1, Ordering::Relaxed);
        if run_index >= LOSS_RUN_CAPACITY {
            self.invalid.store(true, Ordering::Relaxed);
            return;
        }
        if source_position_frames.checked_add(dropped_frames).is_none() {
            self.invalid.store(true, Ordering::Relaxed);
        }
        self.runs[run_index].publish(source_position_frames, dropped_frames, cause);
    }

    pub(super) fn take(&self, generation: u64) -> Result<Option<LossSnapshot>, TimelineError> {
        let claimed_runs = self.claimed_runs.swap(0, Ordering::Relaxed);
        let mut invalid =
            self.invalid.swap(false, Ordering::Relaxed) || claimed_runs > LOSS_RUN_CAPACITY;
        let mut runs = [EMPTY_LOSS_RUN; LOSS_RUN_CAPACITY];
        for (index, cell) in self.runs.iter().enumerate() {
            let published = cell.take();
            if index < claimed_runs.min(LOSS_RUN_CAPACITY) {
                match published {
                    Some(run) => runs[index] = run,
                    None => invalid = true,
                }
            } else if published.is_some() {
                invalid = true;
            }
        }
        if invalid {
            return Err(TimelineError::InvalidTiming);
        }
        if claimed_runs == 0 {
            return Ok(None);
        }

        let runs = &mut runs[..claimed_runs];
        runs.sort_unstable_by_key(|run| run.source_position_frames);
        let first_source_position_frames = runs[0].source_position_frames;
        let encoded_cause = runs[0].cause;
        let cause = decode_cause(encoded_cause).ok_or(TimelineError::InvalidTiming)?;
        let mut expected_position = first_source_position_frames;
        let mut dropped_frames = 0_u64;
        for run in runs {
            if run.dropped_frames == 0
                || run.cause != encoded_cause
                || run.source_position_frames != expected_position
            {
                return Err(TimelineError::InvalidTiming);
            }
            expected_position = run
                .source_position_frames
                .checked_add(run.dropped_frames)
                .ok_or(TimelineError::InvalidTiming)?;
            dropped_frames = dropped_frames
                .checked_add(run.dropped_frames)
                .ok_or(TimelineError::InvalidTiming)?;
        }

        Ok(Some(LossSnapshot {
            first_source_position_frames,
            dropped_frames,
            cause,
            generation,
        }))
    }
}

fn encode_cause(cause: GapCause) -> u8 {
    match cause {
        GapCause::CallbackPoolExhausted => 1,
        GapCause::OversizedCallback => 2,
        GapCause::DeviceDiscontinuity => 3,
        GapCause::SinkUnavailable => 4,
    }
}

fn decode_cause(encoded: u8) -> Option<GapCause> {
    match encoded {
        1 => Some(GapCause::CallbackPoolExhausted),
        2 => Some(GapCause::OversizedCallback),
        3 => Some(GapCause::DeviceDiscontinuity),
        4 => Some(GapCause::SinkUnavailable),
        _ => None,
    }
}
