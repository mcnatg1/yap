use sha2::{Digest, Sha256};

use crate::audio::session::{
    CaptureTrackDescriptor, OwnerNamespace, SessionId, SessionMode, SessionOrigin, TrackId,
    TrackSource,
};

use super::sample::{AudioFrame, AudioGap, ManifestError};

mod validation;

pub(crate) use validation::{
    track_source_matches_origin, validate_current_descriptor, validate_route,
};
use validation::{validate_frames, validate_vad_segments};

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

pub struct ChunkBuildContext<'a> {
    pub owner_namespace: &'a OwnerNamespace,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub track: &'a CaptureTrackDescriptor,
    pub route: AudioRoute,
    pub audio_artifact_id: &'a str,
    pub encoded_audio: &'a [u8],
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
