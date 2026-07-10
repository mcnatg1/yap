use std::collections::HashMap;
use std::hint::spin_loop;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use crate::audio::frame::{AudioFrame, AudioGap, GapCause, TrackConfigurationRevision};
use crate::audio::session::{SessionId, TrackId};

const NO_CAUSE: u8 = 0;
const LOSS_RUN_CAPACITY: usize = 64;
const REGISTRATION_TICKET_CAPACITY: usize = LOSS_RUN_CAPACITY * 4;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockMappingRevision {
    pub track_id: TrackId,
    pub revision: u32,
    pub source_position_frames: u64,
    pub session_time_ms: u64,
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

#[derive(Debug, Clone, PartialEq)]
pub enum TimelineEvent {
    TrackConfigured(TrackConfigurationRevision),
    ClockMapped(ClockMappingRevision),
    Frame(AudioFrame),
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
    coalescible_gap_event_index: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Timeline {
    session_id: SessionId,
    tracks: HashMap<TrackId, TrackTimeline>,
    events: Vec<TimelineEvent>,
}

impl Timeline {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            tracks: HashMap::new(),
            events: Vec::new(),
        }
    }

    pub fn events(&self) -> &[TimelineEvent] {
        &self.events
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
            coalescible_gap_event_index: None,
            configuration: configuration.clone(),
            clock: None,
        };
        self.tracks.insert(track_id, track);
        self.events
            .push(TimelineEvent::TrackConfigured(configuration));
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
        state.coalescible_gap_event_index = None;
        self.events.push(TimelineEvent::ClockMapped(mapping));
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
        state.coalescible_gap_event_index = None;

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
        self.events.push(TimelineEvent::Frame(frame.clone()));
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
        let coalescible_index = state.coalescible_gap_event_index.filter(|index| {
            matches!(
                self.events.get(*index),
                Some(TimelineEvent::Gap(previous)) if gaps_are_contiguous(previous, &gap)
            )
        });
        let emitted = if let Some(index) = coalescible_index {
            let Some(TimelineEvent::Gap(previous)) = self.events.get_mut(index) else {
                unreachable!("coalescible gap index must reference a gap event");
            };
            merge_gap(previous, &gap)?;
            previous.clone()
        } else {
            state.coalescible_gap_event_index = Some(self.events.len());
            self.events.push(TimelineEvent::Gap(gap.clone()));
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

struct DrainGuard<'a> {
    in_progress: &'a AtomicBool,
}

impl Drop for DrainGuard<'_> {
    fn drop(&mut self) {
        self.in_progress.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
pub struct LossAccumulator {
    // The full handoff generation, rather than only its low slot bit, prevents ABA.
    active_generation: AtomicU64,
    registration_started: AtomicU64,
    registration_completed: AtomicU64,
    registration_drained: AtomicU64,
    registration_completion_tickets: [AtomicU64; REGISTRATION_TICKET_CAPACITY],
    drain_in_progress: AtomicBool,
    slots: [LossSlot; 2],
}

impl LossAccumulator {
    pub const fn new() -> Self {
        Self {
            active_generation: AtomicU64::new(0),
            registration_started: AtomicU64::new(0),
            registration_completed: AtomicU64::new(0),
            registration_drained: AtomicU64::new(0),
            registration_completion_tickets: [const { AtomicU64::new(0) };
                REGISTRATION_TICKET_CAPACITY],
            drain_in_progress: AtomicBool::new(false),
            slots: [LossSlot::new(), LossSlot::new()],
        }
    }

    pub fn record(&self, source_position_frames: u64, dropped_frames: u64, cause: GapCause) {
        self.record_inner(source_position_frames, dropped_frames, cause, || {}, || {});
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
        if dropped_frames == 0 {
            return;
        }
        let started = self.registration_started.fetch_add(1, Ordering::SeqCst);
        assert_ne!(started, u64::MAX, "loss registration counter exhausted");
        let registration_floor = self.registration_drained.load(Ordering::SeqCst);
        let outstanding_registrations = started
            .checked_sub(registration_floor)
            .expect("loss registration tickets regressed");
        assert!(
            outstanding_registrations < REGISTRATION_TICKET_CAPACITY as u64,
            "loss registration ticket capacity exhausted"
        );
        after_entrant_registered();
        let generation = self.active_generation.load(Ordering::SeqCst);
        after_generation_read();
        let slot = &self.slots[(generation & 1) as usize];
        slot.writers.fetch_add(1, Ordering::Relaxed);
        let completed_ticket = started + 1;
        self.registration_completion_tickets[started as usize % REGISTRATION_TICKET_CAPACITY]
            .store(completed_ticket, Ordering::SeqCst);
        let completed = self.registration_completed.fetch_add(1, Ordering::SeqCst);
        assert_ne!(
            completed,
            u64::MAX,
            "loss registration completion counter exhausted"
        );

        slot.record(source_position_frames, dropped_frames, cause);
        slot.writers.fetch_sub(1, Ordering::Release);
    }

    pub fn drain(&self) -> Result<Option<LossSnapshot>, TimelineError> {
        self.drain_inner(|| {}, || {}, || {})
    }

    fn drain_inner<F, G, H>(
        &self,
        after_flip: F,
        after_registration_wait: G,
        on_contention: H,
    ) -> Result<Option<LossSnapshot>, TimelineError>
    where
        F: FnOnce(),
        G: FnOnce(),
        H: FnOnce(),
    {
        let _guard = self.acquire_drain(on_contention);
        let generation = self
            .active_generation
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_add(1)
            })
            .map_err(|_| TimelineError::InvalidTiming)?;
        let registration_floor = self.registration_drained.load(Ordering::SeqCst);
        let registration_target = self.registration_started.load(Ordering::SeqCst);
        let pending_registrations = registration_target
            .checked_sub(registration_floor)
            .ok_or(TimelineError::InvalidTiming)?;
        if pending_registrations > REGISTRATION_TICKET_CAPACITY as u64 {
            return Err(TimelineError::InvalidTiming);
        }
        after_flip();
        for ticket in registration_floor..registration_target {
            let expected = ticket.checked_add(1).ok_or(TimelineError::InvalidTiming)?;
            let completion = &self.registration_completion_tickets
                [ticket as usize % REGISTRATION_TICKET_CAPACITY];
            while completion.load(Ordering::SeqCst) != expected {
                spin_loop();
            }
        }
        self.registration_drained
            .store(registration_target, Ordering::SeqCst);
        after_registration_wait();
        let slot = &self.slots[(generation & 1) as usize];
        while slot.writers.load(Ordering::Acquire) != 0 {
            spin_loop();
        }
        slot.take(generation)
    }

    fn acquire_drain<F>(&self, on_contention: F) -> DrainGuard<'_>
    where
        F: FnOnce(),
    {
        if self
            .drain_in_progress
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            on_contention();
            while self
                .drain_in_progress
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                spin_loop();
            }
        }
        DrainGuard {
            in_progress: &self.drain_in_progress,
        }
    }

    #[cfg(test)]
    fn record_with_hook<F>(
        &self,
        source_position_frames: u64,
        dropped_frames: u64,
        cause: GapCause,
        after_writer_registered: F,
    ) where
        F: FnOnce(),
    {
        self.record_inner(
            source_position_frames,
            dropped_frames,
            cause,
            after_writer_registered,
            || {},
        );
    }

    #[cfg(test)]
    fn record_with_registration_hooks<F, G>(
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

    #[cfg(test)]
    fn registration_ticket_completed(&self, ticket: u64) -> bool {
        self.registration_completion_tickets[ticket as usize % REGISTRATION_TICKET_CAPACITY]
            .load(Ordering::SeqCst)
            == ticket + 1
    }

    #[cfg(test)]
    fn drain_with_hook<F>(&self, after_flip: F) -> Option<LossSnapshot>
    where
        F: FnOnce(),
    {
        self.drain_inner(after_flip, || {}, || {}).unwrap()
    }

    #[cfg(test)]
    fn drain_with_hooks<F, G>(
        &self,
        after_flip: F,
        on_contention: G,
    ) -> Result<Option<LossSnapshot>, TimelineError>
    where
        F: FnOnce(),
        G: FnOnce(),
    {
        self.drain_inner(after_flip, || {}, on_contention)
    }

    #[cfg(test)]
    fn drain_with_registration_hooks<F, G, H>(
        &self,
        after_flip: F,
        after_registration_wait: G,
        on_contention: H,
    ) -> Result<Option<LossSnapshot>, TimelineError>
    where
        F: FnOnce(),
        G: FnOnce(),
        H: FnOnce(),
    {
        self.drain_inner(after_flip, after_registration_wait, on_contention)
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
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::{
        ClockMappingRevision, LossAccumulator, LossSnapshot, Timeline, TimelineError, TimelineEvent,
    };
    use crate::audio::frame::{AudioGap, GapCause, TrackConfigurationRevision};
    use crate::audio::session::{SessionId, TrackId};

    fn session_id() -> SessionId {
        SessionId::new("s-timeline-test").unwrap()
    }

    fn track_id(value: &str) -> TrackId {
        TrackId::new(value).unwrap()
    }

    fn configured_timeline(track: &TrackId, sample_rate_hz: u32) -> Timeline {
        let mut timeline = Timeline::new(session_id());
        timeline
            .configure_track(
                TrackConfigurationRevision::new(track.clone(), 1, 0, sample_rate_hz).unwrap(),
            )
            .unwrap();
        timeline
            .map_clock(ClockMappingRevision::new(track.clone(), 1, 0, 0).unwrap())
            .unwrap();
        timeline
    }

    fn concurrently_record<const N: usize>(
        records: [(u64, u64, GapCause); N],
    ) -> Result<Option<LossSnapshot>, TimelineError> {
        let losses = Arc::new(LossAccumulator::new());
        let start = Arc::new(Barrier::new(N + 1));
        let writers = records.map(|(position, dropped, cause)| {
            let losses = Arc::clone(&losses);
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                losses.record(position, dropped, cause);
            })
        });

        start.wait();
        for writer in writers {
            writer.join().unwrap();
        }
        losses.drain()
    }

    #[test]
    fn source_positions_convert_to_monotonic_session_time() {
        let track = track_id("mic-1");
        let mut timeline = Timeline::new(session_id());
        timeline
            .configure_track(
                TrackConfigurationRevision::new(track.clone(), 1, 200, 48_000).unwrap(),
            )
            .unwrap();
        timeline
            .map_clock(ClockMappingRevision::new(track.clone(), 1, 9_600, 200).unwrap())
            .unwrap();

        let first = timeline.frame(&track, 12_000, 480, 1).unwrap();
        let second = timeline.frame(&track, 12_480, 480, 1).unwrap();

        assert_eq!((first.start_ms, first.duration_ms), (250, 10));
        assert_eq!((second.start_ms, second.duration_ms), (260, 10));
        assert_eq!(first.end_ms(), second.start_ms);
        assert!(matches!(
            timeline.events()[0],
            TimelineEvent::TrackConfigured(_)
        ));
        assert!(matches!(
            timeline.events()[1],
            TimelineEvent::ClockMapped(_)
        ));
        assert!(matches!(timeline.events()[2], TimelineEvent::Frame(_)));
    }

    #[test]
    fn frame_intervals_are_end_exclusive_without_rounding_drift() {
        let track = track_id("mic-1");
        let mut timeline = configured_timeline(&track, 3);

        let first = timeline.frame(&track, 1, 1, 1).unwrap();
        let second = timeline.frame(&track, 2, 1, 1).unwrap();

        assert_eq!((first.start_ms, first.duration_ms), (333, 333));
        assert_eq!((second.start_ms, second.duration_ms), (666, 334));
        assert_eq!(first.end_ms(), second.start_ms);
    }

    #[test]
    fn source_frame_overlap_is_rejected_even_when_milliseconds_do_not_overlap() {
        let track = track_id("mic-1");
        let mut timeline = configured_timeline(&track, 48_000);

        timeline.frame(&track, 0, 49, 1).unwrap();

        assert_eq!(
            timeline.frame(&track, 48, 48, 1),
            Err(TimelineError::InvalidTiming)
        );
    }

    #[test]
    fn clock_remap_cannot_regress_before_the_checked_source_frame_end() {
        let track = track_id("mic-1");
        let mut timeline = configured_timeline(&track, 48_000);
        timeline.frame(&track, 0, 49, 1).unwrap();

        assert_eq!(
            timeline.map_clock(ClockMappingRevision::new(track, 2, 48, 1).unwrap()),
            Err(TimelineError::InvalidTiming)
        );
    }

    #[test]
    fn frame_sequences_are_owned_per_track() {
        let mic = track_id("mic-1");
        let loopback = track_id("loopback-1");
        let mut timeline = Timeline::new(session_id());
        for track in [&mic, &loopback] {
            timeline
                .configure_track(
                    TrackConfigurationRevision::new(track.clone(), 1, 0, 16_000).unwrap(),
                )
                .unwrap();
            timeline
                .map_clock(ClockMappingRevision::new(track.clone(), 1, 0, 0).unwrap())
                .unwrap();
        }

        let mic_first = timeline.frame(&mic, 0, 160, 1).unwrap();
        let loopback_first = timeline.frame(&loopback, 0, 160, 2).unwrap();
        let mic_second = timeline.frame(&mic, 160, 160, 1).unwrap();

        assert_eq!(mic_first.sequence, 0);
        assert_eq!(loopback_first.sequence, 0);
        assert_eq!(mic_second.sequence, 1);
        assert_eq!(loopback_first.sample_count, 320);
    }

    #[test]
    fn contiguous_same_cause_gaps_coalesce() {
        let track = track_id("mic-1");
        let mut timeline = configured_timeline(&track, 16_000);

        timeline
            .gap(
                &track,
                LossSnapshot {
                    first_source_position_frames: 0,
                    dropped_frames: 160,
                    cause: GapCause::CallbackPoolExhausted,
                    generation: 0,
                },
            )
            .unwrap();
        let merged = timeline
            .gap(
                &track,
                LossSnapshot {
                    first_source_position_frames: 160,
                    dropped_frames: 320,
                    cause: GapCause::CallbackPoolExhausted,
                    generation: 1,
                },
            )
            .unwrap();

        assert_eq!(merged.source_position_frames, 0);
        assert_eq!(merged.dropped_frames, 480);
        assert_eq!(merged.duration_ms, 30);
        assert_eq!(merged.generation, 1);
        assert_eq!(
            timeline
                .events()
                .iter()
                .filter(|event| matches!(event, TimelineEvent::Gap(_)))
                .count(),
            1
        );
    }

    #[test]
    fn same_track_gaps_coalesce_across_other_tracks_and_empty_generations() {
        let mic = track_id("mic-1");
        let loopback = track_id("loopback-1");
        let mut timeline = Timeline::new(session_id());
        for track in [&mic, &loopback] {
            timeline
                .configure_track(
                    TrackConfigurationRevision::new(track.clone(), 1, 0, 16_000).unwrap(),
                )
                .unwrap();
            timeline
                .map_clock(ClockMappingRevision::new(track.clone(), 1, 0, 0).unwrap())
                .unwrap();
        }

        timeline
            .gap(
                &mic,
                LossSnapshot {
                    first_source_position_frames: 0,
                    dropped_frames: 160,
                    cause: GapCause::CallbackPoolExhausted,
                    generation: 0,
                },
            )
            .unwrap();
        timeline
            .gap(
                &loopback,
                LossSnapshot {
                    first_source_position_frames: 0,
                    dropped_frames: 160,
                    cause: GapCause::SinkUnavailable,
                    generation: 0,
                },
            )
            .unwrap();
        let merged = timeline
            .gap(
                &mic,
                LossSnapshot {
                    first_source_position_frames: 160,
                    dropped_frames: 160,
                    cause: GapCause::CallbackPoolExhausted,
                    generation: 2,
                },
            )
            .unwrap();

        assert_eq!(merged.dropped_frames, 320);
        assert_eq!(merged.generation, 2);
        assert_eq!(
            timeline
                .events()
                .iter()
                .filter(|event| matches!(event, TimelineEvent::Gap(gap) if gap.track_id == mic))
                .count(),
            1
        );
    }

    #[test]
    fn same_track_configuration_breaks_gap_coalescing() {
        let track = track_id("mic-1");
        let mut timeline = configured_timeline(&track, 16_000);
        timeline
            .gap(
                &track,
                LossSnapshot {
                    first_source_position_frames: 0,
                    dropped_frames: 160,
                    cause: GapCause::CallbackPoolExhausted,
                    generation: 0,
                },
            )
            .unwrap();
        timeline
            .configure_track(TrackConfigurationRevision::new(track.clone(), 2, 10, 16_000).unwrap())
            .unwrap();
        timeline
            .map_clock(ClockMappingRevision::new(track.clone(), 2, 160, 10).unwrap())
            .unwrap();

        timeline
            .gap(
                &track,
                LossSnapshot {
                    first_source_position_frames: 160,
                    dropped_frames: 160,
                    cause: GapCause::CallbackPoolExhausted,
                    generation: 1,
                },
            )
            .unwrap();

        assert_eq!(
            timeline
                .events()
                .iter()
                .filter(|event| matches!(event, TimelineEvent::Gap(_)))
                .count(),
            2
        );
    }

    #[test]
    fn gap_merge_checks_all_totals_before_mutating_the_existing_event() {
        let track = track_id("mic-1");
        let mut previous = AudioGap {
            session_id: session_id(),
            track_id: track.clone(),
            start_ms: 0,
            duration_ms: 1,
            source_position_frames: 0,
            dropped_frames: u64::MAX,
            cause: GapCause::CallbackPoolExhausted,
            generation: 0,
        };
        let next = AudioGap {
            session_id: session_id(),
            track_id: track,
            start_ms: 1,
            duration_ms: 1,
            source_position_frames: u64::MAX,
            dropped_frames: 1,
            cause: GapCause::CallbackPoolExhausted,
            generation: 1,
        };
        let unchanged = previous.clone();

        assert_eq!(
            super::merge_gap(&mut previous, &next),
            Err(TimelineError::InvalidTiming)
        );
        assert_eq!(previous, unchanged);
    }

    #[test]
    fn non_contiguous_or_different_cause_gaps_do_not_coalesce() {
        let track = track_id("mic-1");
        let mut timeline = configured_timeline(&track, 16_000);
        for snapshot in [
            LossSnapshot {
                first_source_position_frames: 0,
                dropped_frames: 160,
                cause: GapCause::CallbackPoolExhausted,
                generation: 0,
            },
            LossSnapshot {
                first_source_position_frames: 320,
                dropped_frames: 160,
                cause: GapCause::CallbackPoolExhausted,
                generation: 1,
            },
            LossSnapshot {
                first_source_position_frames: 480,
                dropped_frames: 160,
                cause: GapCause::OversizedCallback,
                generation: 2,
            },
        ] {
            timeline.gap(&track, snapshot).unwrap();
        }

        let gaps = timeline
            .events()
            .iter()
            .filter_map(|event| match event {
                TimelineEvent::Gap(gap) => Some(gap),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(gaps.len(), 3);
        assert_eq!(gaps[0].dropped_frames, 160);
        assert_eq!(gaps[1].source_position_frames, 320);
        assert_eq!(gaps[2].cause, GapCause::OversizedCallback);
    }

    #[test]
    fn invalid_or_wrapping_source_timing_is_reported_explicitly() {
        let track = track_id("mic-1");
        let mut timeline = configured_timeline(&track, 1);

        assert_eq!(
            timeline.frame(&track, u64::MAX, 1, 1),
            Err(TimelineError::InvalidTiming)
        );
        assert_eq!(
            timeline.gap(
                &track,
                LossSnapshot {
                    first_source_position_frames: 0,
                    dropped_frames: u64::from(u32::MAX) + 1,
                    cause: GapCause::DeviceDiscontinuity,
                    generation: 0,
                },
            ),
            Err(TimelineError::InvalidTiming)
        );
    }

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
    fn callback_registers_once_when_a_drain_flips_during_registration() {
        let losses = Arc::new(LossAccumulator::new());
        let registration_reached = Arc::new(Barrier::new(2));
        let release_registration = Arc::new(Barrier::new(2));
        let drain_flipped = Arc::new(Barrier::new(2));
        let registrations = Arc::new(AtomicUsize::new(0));

        let callback = {
            let losses = Arc::clone(&losses);
            let registration_reached = Arc::clone(&registration_reached);
            let release_registration = Arc::clone(&release_registration);
            let registrations = Arc::clone(&registrations);
            thread::spawn(move || {
                losses.record_with_hook(640, 160, GapCause::CallbackPoolExhausted, || {
                    if registrations.fetch_add(1, Ordering::SeqCst) == 0 {
                        registration_reached.wait();
                        release_registration.wait();
                    }
                });
            })
        };

        registration_reached.wait();
        let coordinator = {
            let losses = Arc::clone(&losses);
            let drain_flipped = Arc::clone(&drain_flipped);
            thread::spawn(move || {
                losses.drain_with_hook(|| {
                    drain_flipped.wait();
                })
            })
        };
        drain_flipped.wait();
        release_registration.wait();

        callback.join().unwrap();
        assert_eq!(coordinator.join().unwrap(), None);
        assert_eq!(registrations.load(Ordering::SeqCst), 1);
        assert_eq!(
            losses.drain(),
            Ok(Some(LossSnapshot {
                first_source_position_frames: 640,
                dropped_frames: 160,
                cause: GapCause::CallbackPoolExhausted,
                generation: 1,
            }))
        );
    }

    #[test]
    fn concurrent_drains_serialize_before_reusing_a_slot() {
        let losses = Arc::new(LossAccumulator::new());
        losses.record(320, 160, GapCause::CallbackPoolExhausted);
        let first_flipped = Arc::new(Barrier::new(2));
        let release_first = Arc::new(Barrier::new(2));
        let contention_reached = Arc::new(Barrier::new(2));
        let release_contender = Arc::new(Barrier::new(2));

        let first = {
            let losses = Arc::clone(&losses);
            let first_flipped = Arc::clone(&first_flipped);
            let release_first = Arc::clone(&release_first);
            thread::spawn(move || {
                losses.drain_with_hooks(
                    || {
                        first_flipped.wait();
                        release_first.wait();
                    },
                    || {},
                )
            })
        };
        first_flipped.wait();

        let second = {
            let losses = Arc::clone(&losses);
            let contention_reached = Arc::clone(&contention_reached);
            let release_contender = Arc::clone(&release_contender);
            thread::spawn(move || {
                losses.drain_with_hooks(
                    || {},
                    || {
                        contention_reached.wait();
                        release_contender.wait();
                    },
                )
            })
        };
        contention_reached.wait();
        release_first.wait();
        release_contender.wait();

        assert_eq!(
            first.join().unwrap(),
            Ok(Some(LossSnapshot {
                first_source_position_frames: 320,
                dropped_frames: 160,
                cause: GapCause::CallbackPoolExhausted,
                generation: 0,
            }))
        );
        assert_eq!(second.join().unwrap(), Ok(None));
    }

    #[test]
    fn post_snapshot_callback_does_not_delay_the_old_slot_drain() {
        let losses = Arc::new(LossAccumulator::new());
        losses.record(0, 10, GapCause::CallbackPoolExhausted);
        let registration_target_snapshotted = Arc::new(Barrier::new(2));
        let release_coordinator = Arc::new(Barrier::new(2));
        let post_snapshot_started = Arc::new(Barrier::new(2));
        let release_post_snapshot = Arc::new(Barrier::new(2));
        let registration_target_completed = Arc::new(Barrier::new(2));

        let coordinator = {
            let losses = Arc::clone(&losses);
            let registration_target_snapshotted = Arc::clone(&registration_target_snapshotted);
            let release_coordinator = Arc::clone(&release_coordinator);
            let registration_target_completed = Arc::clone(&registration_target_completed);
            thread::spawn(move || {
                losses.drain_with_registration_hooks(
                    || {
                        registration_target_snapshotted.wait();
                        release_coordinator.wait();
                    },
                    || {
                        registration_target_completed.wait();
                    },
                    || {},
                )
            })
        };
        registration_target_snapshotted.wait();

        let callback = {
            let losses = Arc::clone(&losses);
            let post_snapshot_started = Arc::clone(&post_snapshot_started);
            let release_post_snapshot = Arc::clone(&release_post_snapshot);
            thread::spawn(move || {
                losses.record_with_hook(10, 10, GapCause::CallbackPoolExhausted, || {
                    post_snapshot_started.wait();
                    release_post_snapshot.wait();
                });
            })
        };
        post_snapshot_started.wait();
        release_coordinator.wait();
        registration_target_completed.wait();

        assert_eq!(
            coordinator.join().unwrap(),
            Ok(Some(LossSnapshot {
                first_source_position_frames: 0,
                dropped_frames: 10,
                cause: GapCause::CallbackPoolExhausted,
                generation: 0,
            }))
        );
        release_post_snapshot.wait();
        callback.join().unwrap();
        assert_eq!(
            losses.drain(),
            Ok(Some(LossSnapshot {
                first_source_position_frames: 10,
                dropped_frames: 10,
                cause: GapCause::CallbackPoolExhausted,
                generation: 1,
            }))
        );
    }

    #[test]
    fn later_registration_completion_cannot_mask_an_earlier_ticket() {
        let losses = Arc::new(LossAccumulator::new());
        let old_generation_read = Arc::new(Barrier::new(2));
        let release_old_registration = Arc::new(Barrier::new(2));

        let old_callback = {
            let losses = Arc::clone(&losses);
            let old_generation_read = Arc::clone(&old_generation_read);
            let release_old_registration = Arc::clone(&release_old_registration);
            thread::spawn(move || {
                losses.record_with_registration_hooks(
                    0,
                    10,
                    GapCause::CallbackPoolExhausted,
                    || {},
                    || {
                        old_generation_read.wait();
                        release_old_registration.wait();
                    },
                );
            })
        };
        old_generation_read.wait();
        let registration_target = losses.registration_started.load(Ordering::SeqCst);
        losses.active_generation.store(1, Ordering::SeqCst);

        losses.record(10, 10, GapCause::CallbackPoolExhausted);

        assert_eq!(registration_target, 1);
        assert_eq!(losses.registration_completed.load(Ordering::SeqCst), 1);
        assert!(!losses.registration_ticket_completed(0));

        release_old_registration.wait();
        old_callback.join().unwrap();
        assert!(losses.registration_ticket_completed(0));
    }

    #[test]
    #[should_panic(expected = "loss registration counter exhausted")]
    fn registration_started_overflow_fails_fast() {
        let losses = LossAccumulator::new();
        losses
            .registration_started
            .store(u64::MAX, Ordering::Relaxed);

        losses.record(0, 1, GapCause::CallbackPoolExhausted);
    }

    #[test]
    #[should_panic(expected = "loss registration completion counter exhausted")]
    fn registration_completed_overflow_fails_fast() {
        let losses = LossAccumulator::new();
        losses
            .registration_completed
            .store(u64::MAX, Ordering::Relaxed);

        losses.record(0, 1, GapCause::CallbackPoolExhausted);
    }

    #[test]
    fn callback_updates_racing_a_drain_survive_in_the_next_generation() {
        let losses = Arc::new(LossAccumulator::new());
        let writer_registered = Arc::new(Barrier::new(2));
        let release_writer = Arc::new(Barrier::new(2));
        let drain_flipped = Arc::new(Barrier::new(2));

        let callback = {
            let losses = Arc::clone(&losses);
            let writer_registered = Arc::clone(&writer_registered);
            let release_writer = Arc::clone(&release_writer);
            thread::spawn(move || {
                let mut first_attempt = true;
                losses.record_with_hook(640, 160, GapCause::CallbackPoolExhausted, || {
                    if first_attempt {
                        first_attempt = false;
                        writer_registered.wait();
                        release_writer.wait();
                    }
                });
            })
        };

        writer_registered.wait();
        let coordinator = {
            let losses = Arc::clone(&losses);
            let drain_flipped = Arc::clone(&drain_flipped);
            thread::spawn(move || {
                losses.drain_with_hook(|| {
                    drain_flipped.wait();
                })
            })
        };

        drain_flipped.wait();
        release_writer.wait();

        callback.join().unwrap();
        assert_eq!(coordinator.join().unwrap(), None);
        assert_eq!(
            losses.drain(),
            Ok(Some(LossSnapshot {
                first_source_position_frames: 640,
                dropped_frames: 160,
                cause: GapCause::CallbackPoolExhausted,
                generation: 1,
            }))
        );
        assert_eq!(losses.drain(), Ok(None));
    }

    #[test]
    fn draining_an_empty_accumulator_returns_none() {
        let losses = LossAccumulator::new();

        assert_eq!(losses.drain(), Ok(None));
        losses.record(10, 0, GapCause::SinkUnavailable);
        assert_eq!(losses.drain(), Ok(None));
    }
}
