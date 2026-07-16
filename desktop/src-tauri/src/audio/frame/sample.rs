use std::sync::Arc;

use crate::audio::session::{SessionId, TrackId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapCause {
    CallbackPoolExhausted,
    OversizedCallback,
    DeviceDiscontinuity,
    SinkUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioGap {
    pub session_id: SessionId,
    pub track_id: TrackId,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub source_position_frames: u64,
    pub dropped_frames: u64,
    pub cause: GapCause,
    pub generation: u64,
}

impl AudioGap {
    pub fn end_ms(&self) -> Option<u64> {
        self.start_ms.checked_add(u64::from(self.duration_ms))
    }

    pub(super) fn covers(
        &self,
        session_id: &SessionId,
        track_id: &TrackId,
        start_ms: u64,
        end_ms: u64,
    ) -> bool {
        self.session_id == *session_id
            && self.track_id == *track_id
            && self.end_ms().is_some_and(|gap_end_ms| {
                self.start_ms == start_ms && gap_end_ms == end_ms && gap_end_ms > self.start_ms
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackConfigurationRevision {
    pub(crate) track_id: TrackId,
    pub(crate) revision: u32,
    pub(crate) effective_at_ms: u64,
    pub(crate) sample_rate_hz: u32,
}

impl TrackConfigurationRevision {
    pub fn new(
        track_id: TrackId,
        revision: u32,
        effective_at_ms: u64,
        sample_rate_hz: u32,
    ) -> Result<Self, ManifestError> {
        if revision == 0 || sample_rate_hz == 0 {
            return Err(ManifestError::InvalidConfigurationRevision);
        }
        Ok(Self {
            track_id,
            revision,
            effective_at_ms,
            sample_rate_hz,
        })
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrackConfigurationRevisionWire {
    track_id: TrackId,
    revision: u32,
    effective_at_ms: u64,
    sample_rate_hz: u32,
}

impl<'de> serde::Deserialize<'de> for TrackConfigurationRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = TrackConfigurationRevisionWire::deserialize(deserializer)?;
        Self::new(
            wire.track_id,
            wire.revision,
            wire.effective_at_ms,
            wire.sample_rate_hz,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    EmptyFrames,
    SessionMismatch,
    TrackMismatch,
    InvalidFrameTiming,
    OverlappingFrameTiming,
    SequenceDiscontinuity,
    TimingDiscontinuity,
    MixedSampleRates,
    MissingConversionRevision,
    InvalidConfigurationRevision,
    InvalidRouteForOrigin,
    InvalidArtifactId,
    EmptyEncodedAudio,
    InvalidVadTiming,
    InvalidGapTiming,
    DurationOverflow,
    SessionTrackReferenceMismatch,
    SessionMetadataMismatch,
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ManifestError {}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioFrame {
    pub session_id: SessionId,
    pub track_id: TrackId,
    pub sequence: u64,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub sample_count: usize,
}

#[derive(Debug, Clone)]
pub struct PreparedFrame {
    pub metadata: AudioFrame,
    pub samples: Arc<[f32]>,
}

impl AudioFrame {
    pub fn duration_ms_from_samples(sample_count: usize, sample_rate_hz: u32) -> u32 {
        if sample_rate_hz == 0 {
            return 0;
        }

        ((sample_count as u128) * 1_000 / u128::from(sample_rate_hz)) as u32
    }

    pub fn end_ms(&self) -> u64 {
        self.start_ms.saturating_add(u64::from(self.duration_ms))
    }

    pub(crate) fn checked_end_ms(&self) -> Result<u64, ManifestError> {
        if self.duration_ms == 0 {
            return Err(ManifestError::InvalidFrameTiming);
        }
        self.start_ms
            .checked_add(u64::from(self.duration_ms))
            .ok_or(ManifestError::DurationOverflow)
    }
}
