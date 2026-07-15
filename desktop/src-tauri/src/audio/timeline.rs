use std::collections::HashMap;
#[cfg(test)]
use std::hint::spin_loop;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, TryLockError};

use crate::audio::frame::{
    AudioFrame, AudioGap, GapCause, PreparedFrame, TrackConfigurationRevision,
};
use crate::audio::session::{SessionId, TrackId};

const NO_CAUSE: u8 = 0;
const LOSS_RUN_CAPACITY: usize = 64;
const REGISTRATION_TICKET_CAPACITY: usize = LOSS_RUN_CAPACITY * 4;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockMappingRevision {
    pub track_id: TrackId,
    pub revision: u32,
    pub source_position_frames: u64,
    pub session_time_ms: u64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClockMappingRevisionWire {
    track_id: TrackId,
    revision: u32,
    source_position_frames: u64,
    session_time_ms: u64,
}

impl<'de> serde::Deserialize<'de> for ClockMappingRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ClockMappingRevisionWire::deserialize(deserializer)?;
        Self::new(
            wire.track_id,
            wire.revision,
            wire.source_position_frames,
            wire.session_time_ms,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ClockMappingRevision {
    pub fn new(
        track_id: TrackId,
        revision: u32,
        source_position_frames: u64,
        session_time_ms: u64,
    ) -> Result<Self, TimelineError> {
        if revision == 0 {
            return Err(TimelineError::InvalidRevision);
        }
        Ok(Self {
            track_id,
            revision,
            source_position_frames,
            session_time_ms,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingRevisionTransition {
    pub(crate) configuration: TrackConfigurationRevision,
    pub(crate) clock_mapping: ClockMappingRevision,
}

impl RecordingRevisionTransition {
    pub fn new(
        configuration: TrackConfigurationRevision,
        clock_mapping: ClockMappingRevision,
    ) -> Result<Self, TimelineError> {
        if configuration.track_id != clock_mapping.track_id
            || configuration.revision != clock_mapping.revision
            || configuration.effective_at_ms != clock_mapping.session_time_ms
        {
            return Err(TimelineError::InvalidRevision);
        }
        Ok(Self {
            configuration,
            clock_mapping,
        })
    }
}

/// Ordered input accepted by the durable recording writer.
///
/// Frames carry PCM, while control events preserve the coordinator's exact
/// source timeline without making other sinks consume recording metadata.
#[derive(Debug, Clone)]
pub enum RecordingInput {
    PreparedFrame(PreparedFrame),
    RevisionTransition(RecordingRevisionTransition),
    Gap(AudioGap),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineError {
    InvalidRevision,
    RevisionRegression,
    MissingTrackConfiguration,
    MissingClockMapping,
    InvalidTiming,
    SequenceOverflow,
    GenerationRegression,
    DrainIncomplete,
}

impl std::fmt::Display for TimelineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for TimelineError {}

#[derive(Debug, Clone)]
pub struct SessionClock {
    mapping: ClockMappingRevision,
    sample_rate_hz: u32,
}

impl SessionClock {
    pub fn new(mapping: ClockMappingRevision, sample_rate_hz: u32) -> Result<Self, TimelineError> {
        if sample_rate_hz == 0 {
            return Err(TimelineError::InvalidTiming);
        }
        Ok(Self {
            mapping,
            sample_rate_hz,
        })
    }

    pub fn interval_ms(
        &self,
        source_position_frames: u64,
        frame_count: u64,
    ) -> Result<(u64, u32), TimelineError> {
        if frame_count == 0 {
            return Err(TimelineError::InvalidTiming);
        }
        let end_source_position = source_position_frames
            .checked_add(frame_count)
            .ok_or(TimelineError::InvalidTiming)?;
        let start_ms = self.position_ms(source_position_frames)?;
        let end_ms = self.position_ms(end_source_position)?;
        let duration_ms = end_ms
            .checked_sub(start_ms)
            .and_then(|duration| u32::try_from(duration).ok())
            .filter(|duration| *duration > 0)
            .ok_or(TimelineError::InvalidTiming)?;
        Ok((start_ms, duration_ms))
    }

    fn position_ms(&self, source_position_frames: u64) -> Result<u64, TimelineError> {
        let relative_frames = source_position_frames
            .checked_sub(self.mapping.source_position_frames)
            .ok_or(TimelineError::InvalidTiming)?;
        let relative_ms = u128::from(relative_frames)
            .checked_mul(1_000)
            .ok_or(TimelineError::InvalidTiming)?
            / u128::from(self.sample_rate_hz);
        let session_time_ms = u128::from(self.mapping.session_time_ms)
            .checked_add(relative_ms)
            .ok_or(TimelineError::InvalidTiming)?;
        u64::try_from(session_time_ms).map_err(|_| TimelineError::InvalidTiming)
    }
}

#[derive(Debug, Clone)]
struct TrackTimeline {
    configuration: TrackConfigurationRevision,
    clock: Option<SessionClock>,
    next_sequence: u64,
    last_end_ms: Option<u64>,
    last_end_source_position_frames: Option<u64>,
    last_gap_generation: Option<u64>,
    coalescible_gap: Option<AudioGap>,
}

#[derive(Debug, Clone)]
pub struct Timeline {
    session_id: SessionId,
    tracks: HashMap<TrackId, TrackTimeline>,
}

impl Timeline {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            tracks: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_metadata_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn configure_track(
        &mut self,
        configuration: TrackConfigurationRevision,
    ) -> Result<(), TimelineError> {
        let track_id = configuration.track_id.clone();
        if let Some(current) = self.tracks.get(&track_id) {
            if configuration.revision <= current.configuration.revision
                || configuration.effective_at_ms < current.configuration.effective_at_ms
            {
                return Err(TimelineError::RevisionRegression);
            }
        }

        let prior = self.tracks.remove(&track_id);
        let track = TrackTimeline {
            next_sequence: prior.as_ref().map_or(0, |state| state.next_sequence),
            last_end_ms: prior.as_ref().and_then(|state| state.last_end_ms),
            last_end_source_position_frames: prior
                .as_ref()
                .and_then(|state| state.last_end_source_position_frames),
            last_gap_generation: prior.and_then(|state| state.last_gap_generation),
            coalescible_gap: None,
            configuration: configuration.clone(),
            clock: None,
        };
        self.tracks.insert(track_id, track);
        Ok(())
    }

    pub fn map_clock(&mut self, mapping: ClockMappingRevision) -> Result<(), TimelineError> {
        let state = self
            .tracks
            .get_mut(&mapping.track_id)
            .ok_or(TimelineError::MissingTrackConfiguration)?;
        if state
            .clock
            .as_ref()
            .is_some_and(|clock| mapping.revision <= clock.mapping.revision)
        {
            return Err(TimelineError::RevisionRegression);
        }
        if state
            .last_end_ms
            .is_some_and(|last_end_ms| mapping.session_time_ms < last_end_ms)
            || state
                .last_end_source_position_frames
                .is_some_and(|last_end| mapping.source_position_frames < last_end)
        {
            return Err(TimelineError::InvalidTiming);
        }
        state.clock = Some(SessionClock::new(
            mapping.clone(),
            state.configuration.sample_rate_hz,
        )?);
        state.coalescible_gap = None;
        Ok(())
    }

    pub fn frame(
        &mut self,
        track_id: &TrackId,
        source_position_frames: u64,
        frame_count: u64,
        channels: u16,
    ) -> Result<AudioFrame, TimelineError> {
        if channels == 0 {
            return Err(TimelineError::InvalidTiming);
        }
        let state = self
            .tracks
            .get_mut(track_id)
            .ok_or(TimelineError::MissingTrackConfiguration)?;
        let clock = state
            .clock
            .as_ref()
            .ok_or(TimelineError::MissingClockMapping)?;
        let end_source_position_frames = source_position_frames
            .checked_add(frame_count)
            .ok_or(TimelineError::InvalidTiming)?;
        if state
            .last_end_source_position_frames
            .is_some_and(|last_end| source_position_frames < last_end)
        {
            return Err(TimelineError::InvalidTiming);
        }
        let (start_ms, duration_ms) = clock.interval_ms(source_position_frames, frame_count)?;
        let end_ms = start_ms
            .checked_add(u64::from(duration_ms))
            .ok_or(TimelineError::InvalidTiming)?;
        if state
            .last_end_ms
            .is_some_and(|last_end_ms| start_ms < last_end_ms)
        {
            return Err(TimelineError::InvalidTiming);
        }
        let sample_count = usize::try_from(frame_count)
            .ok()
            .and_then(|count| count.checked_mul(usize::from(channels)))
            .ok_or(TimelineError::InvalidTiming)?;
        let sequence = state.next_sequence;
        state.next_sequence = sequence
            .checked_add(1)
            .ok_or(TimelineError::SequenceOverflow)?;
        state.last_end_ms = Some(end_ms);
        state.last_end_source_position_frames = Some(end_source_position_frames);
        state.coalescible_gap = None;

        let frame = AudioFrame {
            session_id: self.session_id.clone(),
            track_id: track_id.clone(),
            sequence,
            sample_rate_hz: state.configuration.sample_rate_hz,
            channels,
            start_ms,
            duration_ms,
            sample_count,
        };
        Ok(frame)
    }

    pub fn gap(
        &mut self,
        track_id: &TrackId,
        loss: LossSnapshot,
    ) -> Result<AudioGap, TimelineError> {
        if loss.dropped_frames == 0 {
            return Err(TimelineError::InvalidTiming);
        }
        let state = self
            .tracks
            .get_mut(track_id)
            .ok_or(TimelineError::MissingTrackConfiguration)?;
        if state
            .last_gap_generation
            .is_some_and(|generation| loss.generation <= generation)
        {
            return Err(TimelineError::GenerationRegression);
        }
        let clock = state
            .clock
            .as_ref()
            .ok_or(TimelineError::MissingClockMapping)?;
        let end_source_position_frames = loss
            .first_source_position_frames
            .checked_add(loss.dropped_frames)
            .ok_or(TimelineError::InvalidTiming)?;
        if state
            .last_end_source_position_frames
            .is_some_and(|last_end| loss.first_source_position_frames < last_end)
        {
            return Err(TimelineError::InvalidTiming);
        }
        let (start_ms, duration_ms) =
            clock.interval_ms(loss.first_source_position_frames, loss.dropped_frames)?;
        let end_ms = start_ms
            .checked_add(u64::from(duration_ms))
            .ok_or(TimelineError::InvalidTiming)?;
        if state
            .last_end_ms
            .is_some_and(|last_end_ms| start_ms < last_end_ms)
        {
            return Err(TimelineError::InvalidTiming);
        }

        let gap = AudioGap {
            session_id: self.session_id.clone(),
            track_id: track_id.clone(),
            start_ms,
            duration_ms,
            source_position_frames: loss.first_source_position_frames,
            dropped_frames: loss.dropped_frames,
            cause: loss.cause,
            generation: loss.generation,
        };
        let emitted = if let Some(previous) = state
            .coalescible_gap
            .as_mut()
            .filter(|previous| gaps_are_contiguous(previous, &gap))
        {
            merge_gap(previous, &gap)?;
            previous.clone()
        } else {
            state.coalescible_gap = Some(gap.clone());
            gap
        };
        state.last_gap_generation = Some(loss.generation);
        state.last_end_ms = Some(end_ms);
        state.last_end_source_position_frames = Some(end_source_position_frames);
        Ok(emitted)
    }
}

fn gaps_are_contiguous(previous: &AudioGap, next: &AudioGap) -> bool {
    previous.session_id == next.session_id
        && previous.track_id == next.track_id
        && previous.cause == next.cause
        && next.generation > previous.generation
        && previous
            .source_position_frames
            .checked_add(previous.dropped_frames)
            .is_some_and(|end| end == next.source_position_frames)
        && previous.end_ms().is_some_and(|end| end == next.start_ms)
}

fn merge_gap(previous: &mut AudioGap, next: &AudioGap) -> Result<(), TimelineError> {
    let duration_ms = previous
        .duration_ms
        .checked_add(next.duration_ms)
        .ok_or(TimelineError::InvalidTiming)?;
    let dropped_frames = previous
        .dropped_frames
        .checked_add(next.dropped_frames)
        .ok_or(TimelineError::InvalidTiming)?;

    previous.duration_ms = duration_ms;
    previous.dropped_frames = dropped_frames;
    previous.generation = next.generation;
    Ok(())
}

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
    const fn new() -> Self {
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
struct LossSlot {
    writers: AtomicUsize,
    claimed_runs: AtomicUsize,
    runs: [LossRunCell; LOSS_RUN_CAPACITY],
    invalid: AtomicBool,
}

impl LossSlot {
    const fn new() -> Self {
        Self {
            writers: AtomicUsize::new(0),
            claimed_runs: AtomicUsize::new(0),
            runs: [const { LossRunCell::new() }; LOSS_RUN_CAPACITY],
            invalid: AtomicBool::new(false),
        }
    }

    fn record(&self, source_position_frames: u64, dropped_frames: u64, cause: GapCause) {
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

    fn take(&self, generation: u64) -> Result<Option<LossSnapshot>, TimelineError> {
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

#[derive(Debug)]
struct PendingDrain {
    generation: u64,
    registration_floor: u64,
    registration_target: u64,
    registration_invalid: bool,
    counter_exhausted: bool,
}

#[derive(Debug, Default)]
struct LossDrainCoordinator {
    pending: Option<PendingDrain>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistrationReadiness {
    Ready,
    Pending,
    Invalid,
}

#[derive(Debug)]
pub struct LossAccumulator {
    // The full handoff generation, rather than only its low slot bit, prevents ABA.
    active_generation: AtomicU64,
    registration_started: AtomicU64,
    registration_drained: AtomicU64,
    registration_completion_tickets: [AtomicU64; REGISTRATION_TICKET_CAPACITY],
    registration_aborted_tickets: [AtomicU64; REGISTRATION_TICKET_CAPACITY],
    invalid: AtomicBool,
    registration_counter_exhausted: AtomicBool,
    generation_exhausted: AtomicBool,
    terminal_invalid_reported: AtomicBool,
    coordinator: Mutex<LossDrainCoordinator>,
    slots: [LossSlot; 2],
}

impl LossAccumulator {
    pub const fn new() -> Self {
        Self {
            active_generation: AtomicU64::new(0),
            registration_started: AtomicU64::new(0),
            registration_drained: AtomicU64::new(0),
            registration_completion_tickets: [const { AtomicU64::new(0) };
                REGISTRATION_TICKET_CAPACITY],
            registration_aborted_tickets: [const { AtomicU64::new(0) };
                REGISTRATION_TICKET_CAPACITY],
            invalid: AtomicBool::new(false),
            registration_counter_exhausted: AtomicBool::new(false),
            generation_exhausted: AtomicBool::new(false),
            terminal_invalid_reported: AtomicBool::new(false),
            coordinator: Mutex::new(LossDrainCoordinator { pending: None }),
            slots: [LossSlot::new(), LossSlot::new()],
        }
    }

    pub fn record(&self, source_position_frames: u64, dropped_frames: u64, cause: GapCause) {
        self.record_inner(source_position_frames, dropped_frames, cause, || {}, || {});
    }

    pub fn invalidate(&self) {
        self.invalid.store(true, Ordering::SeqCst);
    }

    fn record_inner<F, G>(
        &self,
        source_position_frames: u64,
        dropped_frames: u64,
        cause: GapCause,
        after_entrant_registered: F,
        after_generation_read: G,
    ) where
        F: FnOnce(),
        G: FnOnce(),
    {
        if dropped_frames == 0 || self.generation_exhausted.load(Ordering::SeqCst) {
            return;
        }
        let Some(started) = self.reserve_registration_ticket() else {
            return;
        };
        let mut registration = RegistrationTicket::new(self, started);
        after_entrant_registered();
        let generation = self.active_generation.load(Ordering::SeqCst);
        after_generation_read();
        let slot = &self.slots[(generation & 1) as usize];
        let writer = LossWriter::new(slot);
        slot.record(source_position_frames, dropped_frames, cause);
        drop(writer);
        registration.commit();
    }

    fn reserve_registration_ticket(&self) -> Option<u64> {
        loop {
            let started = self.registration_started.load(Ordering::SeqCst);
            let registration_floor = self.registration_drained.load(Ordering::SeqCst);
            let Some(outstanding_registrations) = started.checked_sub(registration_floor) else {
                self.invalidate();
                return None;
            };
            if started == u64::MAX {
                self.registration_counter_exhausted
                    .store(true, Ordering::SeqCst);
                self.invalidate();
                return None;
            }
            if outstanding_registrations >= REGISTRATION_TICKET_CAPACITY as u64 {
                self.invalidate();
                return None;
            }
            if self
                .registration_started
                .compare_exchange_weak(started, started + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Some(started);
            }
        }
    }

    pub fn try_drain(&self) -> Result<TryDrain, TimelineError> {
        if self.generation_exhausted.load(Ordering::SeqCst) {
            return Err(TimelineError::InvalidTiming);
        }

        let mut coordinator = match self.coordinator.try_lock() {
            Ok(coordinator) => coordinator,
            Err(TryLockError::WouldBlock) => return Ok(TryDrain::Pending),
            Err(TryLockError::Poisoned(poisoned)) => {
                self.invalidate();
                poisoned.into_inner()
            }
        };

        if self.generation_exhausted.load(Ordering::SeqCst) {
            return Err(TimelineError::InvalidTiming);
        }

        if self.terminal_invalid_reported.load(Ordering::SeqCst) && coordinator.pending.is_none() {
            return Err(TimelineError::InvalidTiming);
        }

        if coordinator.pending.is_none() {
            let generation = self.advance_generation()?;
            let registration_floor = self.registration_drained.load(Ordering::SeqCst);
            let registration_target = self.registration_started.load(Ordering::SeqCst);
            let registration_invalid = registration_target
                .checked_sub(registration_floor)
                .is_none_or(|pending| pending > REGISTRATION_TICKET_CAPACITY as u64);
            if registration_invalid {
                self.invalidate();
            }
            coordinator.pending = Some(PendingDrain {
                generation,
                registration_floor,
                registration_target,
                registration_invalid,
                counter_exhausted: self.registration_counter_exhausted.load(Ordering::SeqCst),
            });
        }

        let pending = coordinator
            .pending
            .as_ref()
            .expect("pending drain is initialized before polling");
        let registration_readiness = if pending.registration_invalid {
            RegistrationReadiness::Invalid
        } else {
            self.registrations_are_ready(pending)
        };
        if registration_readiness == RegistrationReadiness::Pending {
            return Ok(TryDrain::Pending);
        }
        let slot = &self.slots[(pending.generation & 1) as usize];
        if slot.writers.load(Ordering::Acquire) != 0 {
            return Ok(TryDrain::Pending);
        }

        let pending = coordinator
            .pending
            .take()
            .expect("pending drain remains owned by the coordinator");
        let snapshot = slot.take(pending.generation);
        let counter_exhausted =
            pending.counter_exhausted || self.registration_counter_exhausted.load(Ordering::SeqCst);
        if pending.registration_invalid || registration_readiness == RegistrationReadiness::Invalid
        {
            self.registration_drained
                .store(pending.registration_target, Ordering::SeqCst);
            self.invalid.store(false, Ordering::SeqCst);
            if counter_exhausted {
                self.terminal_invalid_reported.store(true, Ordering::SeqCst);
            }
            return Err(TimelineError::InvalidTiming);
        }

        self.registration_drained
            .store(pending.registration_target, Ordering::SeqCst);
        if counter_exhausted {
            self.terminal_invalid_reported.store(true, Ordering::SeqCst);
            return Err(TimelineError::InvalidTiming);
        }
        if self.invalid.swap(false, Ordering::SeqCst) {
            Err(TimelineError::InvalidTiming)
        } else {
            match snapshot? {
                Some(snapshot) => Ok(TryDrain::Snapshot(snapshot)),
                None => Ok(TryDrain::Empty),
            }
        }
    }

    #[cfg(test)]
    pub fn drain(&self) -> Result<Option<LossSnapshot>, TimelineError> {
        loop {
            match self.try_drain()? {
                TryDrain::Pending => spin_loop(),
                TryDrain::Empty => return Ok(None),
                TryDrain::Snapshot(snapshot) => return Ok(Some(snapshot)),
            }
        }
    }

    fn registrations_are_ready(&self, pending: &PendingDrain) -> RegistrationReadiness {
        for ticket in pending.registration_floor..pending.registration_target {
            let expected = ticket + 1;
            let index = ticket as usize % REGISTRATION_TICKET_CAPACITY;
            let completion = &self.registration_completion_tickets[index];
            if completion.load(Ordering::SeqCst) == expected {
                continue;
            }
            if self.registration_aborted_tickets[index].load(Ordering::SeqCst) == expected {
                return RegistrationReadiness::Invalid;
            }
            return RegistrationReadiness::Pending;
        }
        RegistrationReadiness::Ready
    }

    fn advance_generation(&self) -> Result<u64, TimelineError> {
        let generation = self.active_generation.load(Ordering::SeqCst);
        match generation.checked_add(1) {
            Some(next_generation) => {
                self.active_generation
                    .store(next_generation, Ordering::SeqCst);
                Ok(generation)
            }
            None => {
                self.generation_exhausted.store(true, Ordering::SeqCst);
                self.invalidate();
                Err(TimelineError::InvalidTiming)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn record_with_registration_hooks<F, G>(
        &self,
        source_position_frames: u64,
        dropped_frames: u64,
        cause: GapCause,
        after_started: F,
        after_generation_read: G,
    ) where
        F: FnOnce(),
        G: FnOnce(),
    {
        self.record_inner(
            source_position_frames,
            dropped_frames,
            cause,
            after_started,
            after_generation_read,
        );
    }
}

struct RegistrationTicket<'a> {
    losses: &'a LossAccumulator,
    ticket: u64,
    committed: bool,
}

impl<'a> RegistrationTicket<'a> {
    fn new(losses: &'a LossAccumulator, ticket: u64) -> Self {
        Self {
            losses,
            ticket,
            committed: false,
        }
    }

    fn commit(&mut self) {
        let completed_ticket = self.ticket + 1;
        self.losses.registration_completion_tickets
            [self.ticket as usize % REGISTRATION_TICKET_CAPACITY]
            .store(completed_ticket, Ordering::SeqCst);
        self.committed = true;
    }
}

impl Drop for RegistrationTicket<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let aborted_ticket = self.ticket + 1;
            self.losses.registration_aborted_tickets
                [self.ticket as usize % REGISTRATION_TICKET_CAPACITY]
                .store(aborted_ticket, Ordering::SeqCst);
            self.losses.invalidate();
        }
    }
}

struct LossWriter<'a> {
    slot: &'a LossSlot,
}

impl<'a> LossWriter<'a> {
    fn new(slot: &'a LossSlot) -> Self {
        slot.writers.fetch_add(1, Ordering::Relaxed);
        Self { slot }
    }
}

impl Drop for LossWriter<'_> {
    fn drop(&mut self) {
        self.slot.writers.fetch_sub(1, Ordering::Release);
    }
}

impl Default for LossAccumulator {
    fn default() -> Self {
        Self::new()
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

#[cfg(test)]
mod tests;
