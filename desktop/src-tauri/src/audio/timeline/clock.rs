use crate::audio::frame::{AudioGap, PreparedFrame, TrackConfigurationRevision};
use crate::audio::session::TrackId;

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
    pub(super) mapping: ClockMappingRevision,
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
