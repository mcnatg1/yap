use super::{
    clock::{ClockMappingRevision, SessionClock, TimelineError},
    loss_buffer::LossSnapshot,
};
use crate::audio::frame::{AudioFrame, AudioGap, TrackConfigurationRevision};
use crate::audio::session::{SessionId, TrackId};
use std::collections::HashMap;

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

pub(super) fn merge_gap(previous: &mut AudioGap, next: &AudioGap) -> Result<(), TimelineError> {
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
