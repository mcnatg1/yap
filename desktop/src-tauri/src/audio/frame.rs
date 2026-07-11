use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::audio::session::{
    CaptureTrackDescriptor, OwnerNamespace, SessionId, SessionMode, SessionOrigin, TrackId,
    TrackSource,
};

pub const CHUNK_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkReplayKey {
    pub schema_version: u16,
    pub owner_namespace: OwnerNamespace,
    pub session_id: SessionId,
    pub track_id: TrackId,
    pub sequence_start: u64,
    pub sequence_end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentIdentity {
    pub sha256: String,
    pub byte_length: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioRoute {
    LocalFallback,
    ServerLive,
    ServerBatch,
}

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

    fn covers(
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

pub struct ChunkBuildContext<'a> {
    pub owner_namespace: &'a OwnerNamespace,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub track: &'a CaptureTrackDescriptor,
    pub route: AudioRoute,
    pub audio_artifact_id: &'a str,
    pub encoded_audio: &'a [u8],
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    PcmS16Le,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioPurpose {
    LocalFallback,
    CaptureEnvelope,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VadSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub kind: crate::audio::vad::VadKind,
    pub rms: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryMetadata {
    pub idempotency_key: String,
    pub attempt: u16,
    pub max_attempts: u16,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureChunkDescriptor {
    pub replay_key: ChunkReplayKey,
    pub content_identity: ContentIdentity,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub track_source: TrackSource,
    pub route: AudioRoute,
    pub audio_artifact_id: String,
    pub session_id: SessionId,
    pub track_id: TrackId,
    pub chunk_id: String,
    pub sequence_start: u64,
    pub sequence_end: u64,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub sample_rate_hz: u32,
    pub codec: AudioCodec,
    pub vad_segments: Vec<VadSegment>,
    pub gaps: Vec<AudioGap>,
    pub purpose: AudioPurpose,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunkEnvelope {
    pub replay_key: ChunkReplayKey,
    pub content_identity: ContentIdentity,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub track_source: TrackSource,
    pub route: AudioRoute,
    pub audio_artifact_id: String,
    pub session_id: SessionId,
    pub track_id: TrackId,
    pub chunk_id: String,
    pub sequence_start: u64,
    pub sequence_end: u64,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub sample_rate_hz: u32,
    pub codec: AudioCodec,
    pub vad_segments: Vec<VadSegment>,
    pub gaps: Vec<AudioGap>,
    pub purpose: AudioPurpose,
    pub retry: RetryMetadata,
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

impl AudioChunkEnvelope {
    pub fn from_frames(
        session_id: SessionId,
        context: ChunkBuildContext<'_>,
        frames: &[AudioFrame],
        codec: AudioCodec,
        vad_segments: Vec<VadSegment>,
        purpose: AudioPurpose,
    ) -> Result<Self, ManifestError> {
        Self::from_frames_with_continuity(
            session_id,
            context,
            frames,
            codec,
            vad_segments,
            Vec::new(),
            purpose,
        )
    }

    pub fn from_frames_with_continuity(
        session_id: SessionId,
        context: ChunkBuildContext<'_>,
        frames: &[AudioFrame],
        codec: AudioCodec,
        vad_segments: Vec<VadSegment>,
        gaps: Vec<AudioGap>,
        purpose: AudioPurpose,
    ) -> Result<Self, ManifestError> {
        let first = frames.first().ok_or(ManifestError::EmptyFrames)?;
        if context.audio_artifact_id.is_empty() {
            return Err(ManifestError::InvalidArtifactId);
        }
        if context.encoded_audio.is_empty() {
            return Err(ManifestError::EmptyEncodedAudio);
        }
        validate_route(context.session_origin, context.route, purpose)?;
        validate_frames(&session_id, &context.track.track_id, frames, &gaps)?;

        let end_ms = frames
            .last()
            .expect("non-empty frames checked above")
            .checked_end_ms()?;
        let duration_ms = end_ms
            .checked_sub(first.start_ms)
            .and_then(|duration| u32::try_from(duration).ok())
            .ok_or(ManifestError::DurationOverflow)?;
        validate_vad_segments(first.start_ms, end_ms, &vad_segments)?;
        let replay_key = ChunkReplayKey {
            schema_version: CHUNK_SCHEMA_VERSION,
            owner_namespace: context.owner_namespace.clone(),
            session_id: session_id.clone(),
            track_id: context.track.track_id.clone(),
            sequence_start: first.sequence,
            sequence_end: frames
                .last()
                .expect("non-empty frames checked above")
                .sequence,
        };
        let content_identity = content_identity(context.encoded_audio);
        let chunk_id = chunk_id_from_replay_key(&replay_key);

        Ok(Self {
            replay_key,
            content_identity,
            session_mode: context.session_mode,
            session_origin: context.session_origin,
            track_source: context.track.source.clone(),
            route: context.route,
            audio_artifact_id: context.audio_artifact_id.into(),
            session_id,
            track_id: context.track.track_id.clone(),
            chunk_id: chunk_id.clone(),
            sequence_start: first.sequence,
            sequence_end: frames
                .last()
                .expect("non-empty frames checked above")
                .sequence,
            start_ms: first.start_ms,
            duration_ms,
            sample_rate_hz: first.sample_rate_hz,
            codec,
            vad_segments,
            gaps,
            purpose,
            retry: RetryMetadata {
                idempotency_key: chunk_id,
                attempt: 1,
                max_attempts: 1,
            },
        })
    }

    pub fn capture_descriptor(&self) -> CaptureChunkDescriptor {
        CaptureChunkDescriptor {
            replay_key: self.replay_key.clone(),
            content_identity: self.content_identity.clone(),
            session_mode: self.session_mode,
            session_origin: self.session_origin,
            track_source: self.track_source.clone(),
            route: self.route,
            audio_artifact_id: self.audio_artifact_id.clone(),
            session_id: self.session_id.clone(),
            track_id: self.track_id.clone(),
            chunk_id: self.chunk_id.clone(),
            sequence_start: self.sequence_start,
            sequence_end: self.sequence_end,
            start_ms: self.start_ms,
            duration_ms: self.duration_ms,
            sample_rate_hz: self.sample_rate_hz,
            codec: self.codec,
            vad_segments: self.vad_segments.clone(),
            gaps: self.gaps.clone(),
            purpose: self.purpose,
        }
    }
}

impl ContentIdentity {
    pub(crate) fn is_valid_sha256(&self) -> bool {
        self.sha256.len() == 64 && self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    }
}

fn content_identity(encoded_audio: &[u8]) -> ContentIdentity {
    let digest = Sha256::digest(encoded_audio);
    ContentIdentity {
        sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        byte_length: encoded_audio.len() as u64,
    }
}

pub(crate) fn chunk_id_from_replay_key(key: &ChunkReplayKey) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"yap.chunk-replay-key.v1\0");
    hasher.update(key.schema_version.to_be_bytes());
    update_length_prefixed(&mut hasher, key.owner_namespace.as_str().as_bytes());
    update_length_prefixed(&mut hasher, key.session_id.as_str().as_bytes());
    update_length_prefixed(&mut hasher, key.track_id.as_str().as_bytes());
    hasher.update(key.sequence_start.to_be_bytes());
    hasher.update(key.sequence_end.to_be_bytes());
    let digest = hasher.finalize();
    format!(
        "chunk-{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn update_length_prefixed(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn validate_frames(
    session_id: &SessionId,
    track_id: &TrackId,
    frames: &[AudioFrame],
    gaps: &[AudioGap],
) -> Result<(), ManifestError> {
    let first = frames.first().ok_or(ManifestError::EmptyFrames)?;
    validate_gaps(session_id, track_id, frames, gaps)?;
    let mut previous: Option<&AudioFrame> = None;
    for frame in frames {
        if frame.session_id != *session_id {
            return Err(ManifestError::SessionMismatch);
        }
        if frame.track_id != *track_id {
            return Err(ManifestError::TrackMismatch);
        }
        let frame_end_ms = frame.checked_end_ms()?;
        if frame.sample_rate_hz != first.sample_rate_hz {
            return Err(ManifestError::MixedSampleRates);
        }
        if let Some(previous) = previous {
            let previous_end_ms = previous.checked_end_ms()?;
            if frame.sequence <= previous.sequence {
                return Err(ManifestError::SequenceDiscontinuity);
            }
            if frame.start_ms < previous_end_ms {
                return Err(ManifestError::OverlappingFrameTiming);
            }
            let sequence_is_contiguous = frame.sequence == previous.sequence.saturating_add(1);
            let timing_is_contiguous = frame.start_ms == previous_end_ms;
            if (!sequence_is_contiguous || !timing_is_contiguous)
                && !gaps
                    .iter()
                    .any(|gap| gap.covers(session_id, track_id, previous_end_ms, frame.start_ms))
            {
                return Err(if !sequence_is_contiguous {
                    ManifestError::SequenceDiscontinuity
                } else {
                    ManifestError::TimingDiscontinuity
                });
            }
        }
        let _ = frame_end_ms;
        previous = Some(frame);
    }
    Ok(())
}

fn validate_vad_segments(
    chunk_start_ms: u64,
    chunk_end_ms: u64,
    vad_segments: &[VadSegment],
) -> Result<(), ManifestError> {
    if vad_segments.iter().any(|segment| {
        segment.end_ms <= segment.start_ms
            || segment.start_ms < chunk_start_ms
            || segment.end_ms > chunk_end_ms
    }) {
        return Err(ManifestError::InvalidVadTiming);
    }
    Ok(())
}

fn validate_gaps(
    session_id: &SessionId,
    track_id: &TrackId,
    frames: &[AudioFrame],
    gaps: &[AudioGap],
) -> Result<(), ManifestError> {
    if gaps.iter().any(|gap| {
        gap.session_id != *session_id
            || gap.track_id != *track_id
            || gap.duration_ms == 0
            || gap.dropped_frames == 0
            || gap.end_ms().is_none()
            || frames.iter().any(|frame| {
                let frame_end_ms = frame.checked_end_ms().unwrap_or(u64::MAX);
                gap.start_ms < frame_end_ms
                    && gap
                        .end_ms()
                        .is_some_and(|gap_end_ms| gap_end_ms > frame.start_ms)
            })
    }) {
        return Err(ManifestError::InvalidGapTiming);
    }
    Ok(())
}

pub(crate) fn validate_route(
    origin: SessionOrigin,
    route: AudioRoute,
    purpose: AudioPurpose,
) -> Result<(), ManifestError> {
    if origin == SessionOrigin::ImportedFile
        && (route == AudioRoute::LocalFallback || purpose == AudioPurpose::LocalFallback)
    {
        return Err(ManifestError::InvalidRouteForOrigin);
    }
    Ok(())
}

pub(crate) fn track_source_matches_origin(origin: SessionOrigin, source: &TrackSource) -> bool {
    matches!(
        (origin, source),
        (SessionOrigin::LiveCapture, TrackSource::Captured { .. })
            | (SessionOrigin::ImportedFile, TrackSource::Imported { .. })
    )
}

pub(crate) fn validate_current_descriptor(
    descriptor: &CaptureChunkDescriptor,
) -> Result<(), ManifestError> {
    if descriptor.replay_key.schema_version != CHUNK_SCHEMA_VERSION
        || !descriptor.content_identity.is_valid_sha256()
        || descriptor.content_identity.byte_length == 0
        || descriptor.replay_key.session_id != descriptor.session_id
        || descriptor.replay_key.track_id != descriptor.track_id
        || descriptor.replay_key.sequence_start != descriptor.sequence_start
        || descriptor.replay_key.sequence_end != descriptor.sequence_end
        || descriptor.chunk_id != chunk_id_from_replay_key(&descriptor.replay_key)
        || descriptor.audio_artifact_id.is_empty()
        || descriptor.sequence_end < descriptor.sequence_start
        || descriptor.duration_ms == 0
        || descriptor.sample_rate_hz == 0
        || !track_source_matches_origin(descriptor.session_origin, &descriptor.track_source)
    {
        return Err(ManifestError::SessionTrackReferenceMismatch);
    }
    validate_route(
        descriptor.session_origin,
        descriptor.route,
        descriptor.purpose,
    )?;
    let chunk_end_ms = descriptor
        .start_ms
        .checked_add(u64::from(descriptor.duration_ms))
        .ok_or(ManifestError::DurationOverflow)?;
    validate_vad_segments(descriptor.start_ms, chunk_end_ms, &descriptor.vad_segments)?;
    let mut internal_gaps = Vec::new();
    for gap in &descriptor.gaps {
        let gap_end_ms = gap.end_ms().ok_or(ManifestError::InvalidGapTiming)?;
        let is_internal = gap.start_ms >= descriptor.start_ms && gap_end_ms <= chunk_end_ms;
        let is_preceding = gap_end_ms == descriptor.start_ms;
        if gap.session_id != descriptor.session_id
            || gap.track_id != descriptor.track_id
            || gap.duration_ms == 0
            || gap.dropped_frames == 0
            || (!is_internal && !is_preceding)
        {
            return Err(ManifestError::InvalidGapTiming);
        }
        if is_internal {
            internal_gaps.push((gap.start_ms, gap_end_ms));
        }
    }
    validate_internal_gap_union(descriptor, &internal_gaps)?;
    Ok(())
}

fn validate_internal_gap_union(
    descriptor: &CaptureChunkDescriptor,
    internal_gaps: &[(u64, u64)],
) -> Result<(), ManifestError> {
    let mut gaps = internal_gaps.to_vec();
    gaps.sort_unstable();

    let mut union_duration = 0_u64;
    let mut current: Option<(u64, u64)> = None;
    for (start_ms, end_ms) in gaps {
        match current {
            Some((_, current_end_ms)) if start_ms < current_end_ms => {
                return Err(ManifestError::InvalidGapTiming);
            }
            Some((current_start_ms, current_end_ms)) if start_ms == current_end_ms => {
                current = Some((current_start_ms, end_ms));
            }
            Some((current_start_ms, current_end_ms)) => {
                union_duration = union_duration
                    .checked_add(current_end_ms - current_start_ms)
                    .ok_or(ManifestError::DurationOverflow)?;
                current = Some((start_ms, end_ms));
            }
            None => current = Some((start_ms, end_ms)),
        }
    }
    if let Some((start_ms, end_ms)) = current {
        union_duration = union_duration
            .checked_add(end_ms - start_ms)
            .ok_or(ManifestError::DurationOverflow)?;
    }
    if union_duration >= u64::from(descriptor.duration_ms) {
        return Err(ManifestError::InvalidGapTiming);
    }
    if descriptor.vad_segments.iter().any(|segment| {
        internal_gaps
            .iter()
            .any(|(start_ms, end_ms)| segment.start_ms < *end_ms && segment.end_ms > *start_ms)
    }) {
        return Err(ManifestError::InvalidGapTiming);
    }
    Ok(())
}

impl<'de> serde::Deserialize<'de> for CaptureChunkDescriptor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SchemaOneDescriptor {
            replay_key: ChunkReplayKey,
            content_identity: ContentIdentity,
            session_mode: SessionMode,
            session_origin: SessionOrigin,
            track_source: TrackSource,
            route: AudioRoute,
            audio_artifact_id: String,
            session_id: SessionId,
            track_id: TrackId,
            chunk_id: String,
            sequence_start: u64,
            sequence_end: u64,
            start_ms: u64,
            duration_ms: u32,
            sample_rate_hz: u32,
            codec: AudioCodec,
            vad_segments: Vec<VadSegment>,
            gaps: Vec<AudioGap>,
            purpose: AudioPurpose,
        }

        let schema_one = SchemaOneDescriptor::deserialize(deserializer)?;

        let descriptor = Self {
            replay_key: schema_one.replay_key,
            content_identity: schema_one.content_identity,
            session_mode: schema_one.session_mode,
            session_origin: schema_one.session_origin,
            track_source: schema_one.track_source,
            route: schema_one.route,
            audio_artifact_id: schema_one.audio_artifact_id,
            session_id: schema_one.session_id,
            track_id: schema_one.track_id,
            chunk_id: schema_one.chunk_id,
            sequence_start: schema_one.sequence_start,
            sequence_end: schema_one.sequence_end,
            start_ms: schema_one.start_ms,
            duration_ms: schema_one.duration_ms,
            sample_rate_hz: schema_one.sample_rate_hz,
            codec: schema_one.codec,
            vad_segments: schema_one.vad_segments,
            gaps: schema_one.gaps,
            purpose: schema_one.purpose,
        };
        validate_current_descriptor(&descriptor).map_err(serde::de::Error::custom)?;
        Ok(descriptor)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AudioChunkEnvelope, AudioCodec, AudioFrame, AudioPurpose, AudioRoute, ChunkBuildContext,
        ChunkReplayKey, ContentIdentity, PreparedFrame, VadSegment,
    };
    use crate::audio::{
        manifest::{classify_replay, ReplayConflict, ReplayDecision},
        session::{
            CaptureSource, CaptureTrackDescriptor, OwnerNamespace, SessionId, SessionMode,
            SessionOrigin, TrackId, TrackSource,
        },
        vad::VadKind,
    };

    fn frame(sequence: u64, start_ms: u64, duration_ms: u32, sample_count: usize) -> AudioFrame {
        AudioFrame {
            session_id: SessionId::new("s-test").unwrap(),
            track_id: TrackId::new("mic-1").unwrap(),
            sequence,
            sample_rate_hz: 16_000,
            channels: 1,
            start_ms,
            duration_ms,
            sample_count,
        }
    }

    fn context<'a>(
        owner_namespace: &'a OwnerNamespace,
        track: &'a CaptureTrackDescriptor,
        audio: &'a [u8],
    ) -> ChunkBuildContext<'a> {
        ChunkBuildContext {
            owner_namespace,
            session_mode: SessionMode::Dictation,
            session_origin: SessionOrigin::LiveCapture,
            track,
            route: AudioRoute::ServerBatch,
            audio_artifact_id: "audio-1",
            encoded_audio: audio,
        }
    }

    fn replay_key(sequence_start: u64, sequence_end: u64) -> ChunkReplayKey {
        ChunkReplayKey {
            schema_version: 1,
            owner_namespace: OwnerNamespace::local("install-1").unwrap(),
            session_id: SessionId::new("s-test").unwrap(),
            track_id: TrackId::new("mic-1").unwrap(),
            sequence_start,
            sequence_end,
        }
    }

    fn content_identity(hash: &str) -> ContentIdentity {
        ContentIdentity {
            sha256: hash.into(),
            byte_length: 4,
        }
    }

    fn current_descriptor_json() -> serde_json::Value {
        let owner = OwnerNamespace::local("install-1").unwrap();
        let track = CaptureTrackDescriptor::from_selector(
            TrackId::new("mic-1").unwrap(),
            TrackSource::Captured {
                source: CaptureSource::Microphone,
            },
            "install-1",
            "mic",
        );
        let descriptor = AudioChunkEnvelope::from_frames(
            SessionId::new("s-test").unwrap(),
            context(&owner, &track, b"audio"),
            &[frame(1, 0, 20, 320)],
            AudioCodec::PcmS16Le,
            Vec::new(),
            AudioPurpose::CaptureEnvelope,
        )
        .unwrap()
        .capture_descriptor();

        serde_json::to_value(descriptor).unwrap()
    }

    fn incomplete_chunk_json() -> serde_json::Value {
        serde_json::json!({
            "sessionId": "s-test",
            "chunkId": "old-chunk-1",
            "sequenceStart": 1,
            "startMs": 0,
            "durationMs": 20,
            "sampleRateHz": 16_000,
            "codec": "pcm_s16_le",
            "vadSegments": [],
            "purpose": "captureEnvelope"
        })
    }

    #[test]
    fn same_key_and_hash_is_idempotent() {
        let key = replay_key(1, 2);
        assert_eq!(
            classify_replay(
                &key,
                &content_identity("aaaa"),
                &key,
                &content_identity("aaaa")
            ),
            Ok(ReplayDecision::Idempotent)
        );
    }

    #[test]
    fn same_key_and_different_hash_is_a_conflict() {
        let key = replay_key(1, 2);
        assert_eq!(
            classify_replay(
                &key,
                &content_identity("aaaa"),
                &key,
                &content_identity("bbbb")
            ),
            Err(ReplayConflict::SameKeyDifferentContent)
        );
    }

    #[test]
    fn different_keys_with_the_same_hash_remain_distinct() {
        assert_eq!(
            classify_replay(
                &replay_key(1, 2),
                &content_identity("aaaa"),
                &replay_key(3, 4),
                &content_identity("aaaa"),
            ),
            Ok(ReplayDecision::Distinct)
        );
    }

    #[test]
    fn duration_ms_from_samples_uses_session_relative_sample_math() {
        assert_eq!(AudioFrame::duration_ms_from_samples(320, 16_000), 20);
        assert_eq!(AudioFrame::duration_ms_from_samples(16_000, 16_000), 1_000);
        assert_eq!(AudioFrame::duration_ms_from_samples(0, 16_000), 0);
    }

    #[test]
    fn end_ms_uses_saturating_frame_coverage() {
        assert_eq!(frame(11, u64::MAX - 5, 10, 320).end_ms(), u64::MAX);
    }

    #[test]
    fn from_frames_rejects_empty_or_mixed_track_lists() {
        let owner = OwnerNamespace::local("install-1").unwrap();
        let track = CaptureTrackDescriptor::from_selector(
            TrackId::new("mic-1").unwrap(),
            TrackSource::Captured {
                source: CaptureSource::Microphone,
            },
            "install-1",
            "mic",
        );
        assert!(AudioChunkEnvelope::from_frames(
            SessionId::new("s-test").unwrap(),
            context(&owner, &track, b"audio"),
            &[],
            AudioCodec::PcmS16Le,
            Vec::new(),
            AudioPurpose::LocalFallback,
        )
        .is_err());

        let mut mixed = vec![frame(1, 0, 20, 320), frame(2, 20, 20, 320)];
        mixed[1].track_id = TrackId::new("mic-2").unwrap();
        assert!(AudioChunkEnvelope::from_frames(
            SessionId::new("s-test").unwrap(),
            context(&owner, &track, b"audio"),
            &mixed,
            AudioCodec::PcmS16Le,
            Vec::new(),
            AudioPurpose::LocalFallback,
        )
        .is_err());
    }

    #[test]
    fn from_frames_rejects_empty_encoded_audio() {
        let owner = OwnerNamespace::local("install-1").unwrap();
        let track = CaptureTrackDescriptor::from_selector(
            TrackId::new("mic-1").unwrap(),
            TrackSource::Captured {
                source: CaptureSource::Microphone,
            },
            "install-1",
            "mic",
        );

        assert!(AudioChunkEnvelope::from_frames(
            SessionId::new("s-test").unwrap(),
            context(&owner, &track, &[]),
            &[frame(1, 0, 20, 320)],
            AudioCodec::PcmS16Le,
            Vec::new(),
            AudioPurpose::CaptureEnvelope,
        )
        .is_err());
    }

    #[test]
    fn from_frames_rejects_intra_chunk_rate_changes() {
        let owner = OwnerNamespace::local("install-1").unwrap();
        let track = CaptureTrackDescriptor::from_selector(
            TrackId::new("mic-1").unwrap(),
            TrackSource::Captured {
                source: CaptureSource::Microphone,
            },
            "install-1",
            "mic",
        );
        let mut frames = vec![frame(1, 0, 20, 320), frame(2, 20, 20, 160)];
        frames[1].sample_rate_hz = 8_000;

        assert!(AudioChunkEnvelope::from_frames_with_continuity(
            SessionId::new("s-test").unwrap(),
            context(&owner, &track, b"audio"),
            &frames,
            AudioCodec::PcmS16Le,
            Vec::new(),
            Vec::new(),
            AudioPurpose::CaptureEnvelope,
        )
        .is_err());
    }

    #[test]
    fn from_frames_builds_key_derived_chunk_and_separate_content_identity() {
        let owner = OwnerNamespace::local("install-1").unwrap();
        let track = CaptureTrackDescriptor::from_selector(
            TrackId::new("mic-1").unwrap(),
            TrackSource::Captured {
                source: CaptureSource::Microphone,
            },
            "install-1",
            "mic",
        );
        let envelope = AudioChunkEnvelope::from_frames(
            SessionId::new("s-test").unwrap(),
            context(&owner, &track, b"audio-bytes"),
            &[frame(11, 100, 20, 320), frame(12, 120, 20, 320)],
            AudioCodec::PcmS16Le,
            vec![VadSegment {
                start_ms: 100,
                end_ms: 140,
                kind: VadKind::Speech,
                rms: 0.42,
            }],
            AudioPurpose::CaptureEnvelope,
        )
        .unwrap();

        assert_eq!(envelope.chunk_id, envelope.retry.idempotency_key);
        assert_eq!(envelope.chunk_id.len(), 70);
        assert!(envelope.chunk_id.starts_with("chunk-"));
        assert_eq!(envelope.content_identity.byte_length, 11);
        assert_eq!(envelope.content_identity.sha256.len(), 64);
        assert!(!envelope
            .chunk_id
            .contains(&envelope.content_identity.sha256));
    }

    #[test]
    fn capture_chunk_descriptor_serialization_excludes_transport_retry_metadata() {
        let owner = OwnerNamespace::local("install-1").unwrap();
        let track = CaptureTrackDescriptor::from_selector(
            TrackId::new("mic-1").unwrap(),
            TrackSource::Captured {
                source: CaptureSource::Microphone,
            },
            "install-1",
            "mic",
        );
        let envelope = AudioChunkEnvelope::from_frames(
            SessionId::new("s-test").unwrap(),
            context(&owner, &track, b"audio"),
            &[frame(1, 0, 20, 320)],
            AudioCodec::PcmS16Le,
            Vec::new(),
            AudioPurpose::CaptureEnvelope,
        )
        .unwrap();

        let value = serde_json::to_value(envelope.capture_descriptor()).unwrap();
        assert!(value.get("retry").is_none());
        assert_eq!(value["contentIdentity"]["byteLength"], 5);
        assert_eq!(value["replayKey"]["ownerNamespace"], "local:install-1");
    }

    #[test]
    fn schema_one_descriptor_round_trips_unchanged() {
        let value = current_descriptor_json();
        let descriptor =
            serde_json::from_value::<super::CaptureChunkDescriptor>(value.clone()).unwrap();

        assert_eq!(serde_json::to_value(descriptor).unwrap(), value);
    }

    #[test]
    fn descriptor_missing_replay_schema_version_is_rejected() {
        let mut value = current_descriptor_json();
        value["replayKey"]
            .as_object_mut()
            .unwrap()
            .remove("schemaVersion");

        assert!(serde_json::from_value::<super::CaptureChunkDescriptor>(value).is_err());
    }

    #[test]
    fn descriptor_replay_schema_zero_is_rejected() {
        let mut value = current_descriptor_json();
        value["replayKey"]["schemaVersion"] = serde_json::json!(0);

        assert!(serde_json::from_value::<super::CaptureChunkDescriptor>(value).is_err());
    }

    #[test]
    fn descriptor_unknown_replay_schema_version_is_rejected() {
        let mut value = current_descriptor_json();
        value["replayKey"]["schemaVersion"] = serde_json::json!(2);

        assert!(serde_json::from_value::<super::CaptureChunkDescriptor>(value).is_err());
    }

    #[test]
    fn numeric_chunk_session_ids_are_rejected() {
        let mut value = current_descriptor_json();
        value["sessionId"] = serde_json::json!(7);

        assert!(serde_json::from_value::<super::CaptureChunkDescriptor>(value).is_err());
    }

    #[test]
    fn numeric_replay_key_session_ids_are_rejected() {
        let mut value = current_descriptor_json();
        value["replayKey"]["sessionId"] = serde_json::json!(7);

        assert!(serde_json::from_value::<super::CaptureChunkDescriptor>(value).is_err());
    }

    #[test]
    fn incomplete_chunk_payloads_are_rejected() {
        let value = incomplete_chunk_json();

        assert!(serde_json::from_value::<super::CaptureChunkDescriptor>(value).is_err());
    }

    #[test]
    fn current_descriptor_json_rejects_local_contract_violations() {
        let owner = OwnerNamespace::local("install-1").unwrap();
        let track = CaptureTrackDescriptor::from_selector(
            TrackId::new("mic-1").unwrap(),
            TrackSource::Captured {
                source: CaptureSource::Microphone,
            },
            "install-1",
            "mic",
        );
        let descriptor = AudioChunkEnvelope::from_frames(
            SessionId::new("s-test").unwrap(),
            context(&owner, &track, b"audio"),
            &[frame(1, 0, 20, 320)],
            AudioCodec::PcmS16Le,
            Vec::new(),
            AudioPurpose::CaptureEnvelope,
        )
        .unwrap()
        .capture_descriptor();
        let value = serde_json::to_value(descriptor).unwrap();

        let mut bad_chunk_id = value.clone();
        bad_chunk_id["chunkId"] = serde_json::json!("chunk-tampered");
        assert!(serde_json::from_value::<super::CaptureChunkDescriptor>(bad_chunk_id).is_err());

        let mut bad_rate = value.clone();
        bad_rate["sampleRateHz"] = serde_json::json!(0);
        assert!(serde_json::from_value::<super::CaptureChunkDescriptor>(bad_rate).is_err());

        let mut bad_vad = value.clone();
        bad_vad["vadSegments"] = serde_json::json!([{
            "startMs": 0,
            "endMs": 21,
            "kind": "speech",
            "rms": 0.3
        }]);
        assert!(serde_json::from_value::<super::CaptureChunkDescriptor>(bad_vad).is_err());

        let mut full_gap = value;
        full_gap["gaps"] = serde_json::json!([{
            "sessionId": "s-test",
            "trackId": "mic-1",
            "startMs": 0,
            "durationMs": 20,
            "sourcePositionFrames": 0,
            "droppedFrames": 320,
            "cause": "sink_unavailable",
            "generation": 1
        }]);
        assert!(serde_json::from_value::<super::CaptureChunkDescriptor>(full_gap).is_err());
    }

    #[test]
    fn chunk_ids_are_collision_safe_for_hyphenated_replay_key_components() {
        fn envelope(install_id: &str, session: &str) -> super::AudioChunkEnvelope {
            let owner = OwnerNamespace::local(install_id).unwrap();
            let track = CaptureTrackDescriptor::from_selector(
                TrackId::new("d").unwrap(),
                TrackSource::Captured {
                    source: CaptureSource::Microphone,
                },
                install_id,
                "device",
            );
            let frame = AudioFrame {
                session_id: SessionId::new(session).unwrap(),
                track_id: TrackId::new("d").unwrap(),
                sequence: 1,
                sample_rate_hz: 16_000,
                channels: 1,
                start_ms: 0,
                duration_ms: 20,
                sample_count: 320,
            };
            AudioChunkEnvelope::from_frames(
                SessionId::new(session).unwrap(),
                ChunkBuildContext {
                    owner_namespace: &owner,
                    session_mode: SessionMode::Dictation,
                    session_origin: SessionOrigin::LiveCapture,
                    track: &track,
                    route: AudioRoute::ServerBatch,
                    audio_artifact_id: "audio-1",
                    encoded_audio: b"audio",
                },
                &[frame],
                AudioCodec::PcmS16Le,
                Vec::new(),
                AudioPurpose::CaptureEnvelope,
            )
            .unwrap()
        }

        let first = envelope("a", "b-c");
        let second = envelope("a-b", "c");
        assert_ne!(first.replay_key, second.replay_key);
        assert_ne!(first.chunk_id, second.chunk_id);
        assert_eq!(first.chunk_id, envelope("a", "b-c").chunk_id);
        assert_eq!(
            first.chunk_id,
            "chunk-cb1347611f88d88fa7cd97221d31d48e862b1ac4cb06728580901c6a129cdc8a"
        );
        assert!(first.chunk_id.starts_with("chunk-"));
    }

    #[test]
    fn prepared_frames_keep_samples_out_of_serializable_metadata() {
        let metadata = frame(1, 0, 20, 320);
        let prepared = PreparedFrame {
            metadata: metadata.clone(),
            samples: std::sync::Arc::from([0.0_f32, 0.25_f32]),
        };
        let value = serde_json::to_value(metadata).unwrap();
        assert!(value.get("samples").is_none());
        assert_eq!(prepared.samples.len(), 2);
        assert_eq!(prepared.metadata.track_id.as_str(), "mic-1");
    }

    #[test]
    fn configuration_revision_json_cannot_bypass_field_validation() {
        let invalid = serde_json::json!({
            "trackId": "mic-1",
            "revision": 0,
            "effectiveAtMs": 20,
            "sampleRateHz": 0
        });

        assert!(serde_json::from_value::<super::TrackConfigurationRevision>(invalid).is_err());
    }
}
