use super::validation::{validate_chunk_references, validate_track_sources};
use crate::audio::frame::{
    AudioChunkEnvelope, AudioCodec, AudioFrame, AudioPurpose, CaptureChunkDescriptor,
    ChunkBuildContext, ChunkReplayKey, ContentIdentity, ManifestError, TrackConfigurationRevision,
    VadSegment,
};
use crate::audio::session::{CaptureTrackDescriptor, SessionId, SessionMode, SessionOrigin};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayDecision {
    Idempotent,
    Distinct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayConflict {
    SameKeyDifferentContent,
}

pub fn classify_replay(
    existing_key: &ChunkReplayKey,
    existing_content: &ContentIdentity,
    incoming_key: &ChunkReplayKey,
    incoming_content: &ContentIdentity,
) -> Result<ReplayDecision, ReplayConflict> {
    if existing_key != incoming_key {
        return Ok(ReplayDecision::Distinct);
    }
    if existing_content == incoming_content {
        Ok(ReplayDecision::Idempotent)
    } else {
        Err(ReplayConflict::SameKeyDifferentContent)
    }
}

pub const MANIFEST_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSessionEnvelope {
    pub schema_version: u16,
    pub session_id: SessionId,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub tracks: Vec<CaptureTrackDescriptor>,
    pub track_configuration_revisions: Vec<TrackConfigurationRevision>,
    pub started_at_ms: u64,
    pub sample_rate_hz: u32,
    pub chunks: Vec<CaptureChunkDescriptor>,
    pub degraded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkWindowConfig {
    pub target_window_ms: u32,
    pub max_window_ms: u32,
    pub tail_padding_ms: u32,
    pub preserve_silence_markers: bool,
}

pub struct AudioChunkEnvelopeBuilder<'a> {
    session_id: SessionId,
    context: ChunkBuildContext<'a>,
    purpose: AudioPurpose,
    codec: AudioCodec,
    frames: Vec<AudioFrame>,
}

impl<'a> AudioChunkEnvelopeBuilder<'a> {
    pub fn new(
        session_id: SessionId,
        context: ChunkBuildContext<'a>,
        purpose: AudioPurpose,
        codec: AudioCodec,
    ) -> Self {
        Self {
            session_id,
            context,
            purpose,
            codec,
            frames: Vec::new(),
        }
    }

    pub fn push(&mut self, frame: AudioFrame) -> Result<(), ManifestError> {
        let mut candidate = self.frames.clone();
        candidate.push(frame.clone());
        AudioChunkEnvelope::from_frames(
            self.session_id.clone(),
            ChunkBuildContext {
                owner_namespace: self.context.owner_namespace,
                session_mode: self.context.session_mode,
                session_origin: self.context.session_origin,
                track: self.context.track,
                route: self.context.route,
                audio_artifact_id: self.context.audio_artifact_id,
                encoded_audio: self.context.encoded_audio,
            },
            &candidate,
            self.codec,
            Vec::new(),
            self.purpose,
        )?;
        self.frames.push(frame);
        Ok(())
    }

    pub fn finish(
        self,
        vad_segments: Vec<VadSegment>,
    ) -> Result<AudioChunkEnvelope, ManifestError> {
        AudioChunkEnvelope::from_frames(
            self.session_id,
            self.context,
            &self.frames,
            self.codec,
            vad_segments,
            self.purpose,
        )
    }
}

pub struct AudioSessionEnvelopeBuilder {
    session_id: SessionId,
    session_mode: SessionMode,
    session_origin: SessionOrigin,
    tracks: Vec<CaptureTrackDescriptor>,
    started_at_ms: u64,
    sample_rate_hz: u32,
    track_configuration_revisions: Vec<TrackConfigurationRevision>,
    chunks: Vec<CaptureChunkDescriptor>,
    degraded: bool,
}

impl AudioSessionEnvelopeBuilder {
    pub fn new(
        session_id: SessionId,
        session_mode: SessionMode,
        session_origin: SessionOrigin,
        tracks: Vec<CaptureTrackDescriptor>,
        started_at_ms: u64,
        sample_rate_hz: u32,
    ) -> Self {
        Self {
            session_id,
            session_mode,
            session_origin,
            tracks,
            started_at_ms,
            sample_rate_hz,
            track_configuration_revisions: Vec::new(),
            chunks: Vec::new(),
            degraded: false,
        }
    }

    pub fn push_chunk(&mut self, chunk: AudioChunkEnvelope) {
        self.chunks.push(chunk.capture_descriptor());
    }

    pub fn push_track_configuration_revision(&mut self, revision: TrackConfigurationRevision) {
        self.track_configuration_revisions.push(revision);
    }

    pub fn mark_degraded(&mut self) {
        self.degraded = true;
    }

    pub fn finish(mut self) -> Result<AudioSessionEnvelope, ManifestError> {
        validate_track_sources(self.session_origin, &self.tracks)
            .map_err(|_| ManifestError::SessionMetadataMismatch)?;
        self.chunks.sort_by_key(|chunk| {
            (
                chunk.track_id.as_str().to_owned(),
                chunk.sequence_start,
                chunk.start_ms,
                chunk.chunk_id.clone(),
            )
        });
        validate_chunk_references(
            &self.session_id,
            self.session_mode,
            self.session_origin,
            &self.tracks,
            self.sample_rate_hz,
            &self.track_configuration_revisions,
            &self.chunks,
        )?;
        let degraded = self.degraded || self.chunks.iter().any(|chunk| !chunk.gaps.is_empty());

        Ok(AudioSessionEnvelope {
            schema_version: MANIFEST_SCHEMA_VERSION,
            session_id: self.session_id,
            session_mode: self.session_mode,
            session_origin: self.session_origin,
            tracks: self.tracks,
            track_configuration_revisions: self.track_configuration_revisions,
            started_at_ms: self.started_at_ms,
            sample_rate_hz: self.sample_rate_hz,
            chunks: self.chunks,
            degraded,
        })
    }
}

impl<'de> serde::Deserialize<'de> for AudioSessionEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        fn field_is_present<'de, D>(deserializer: D) -> Result<bool, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            <serde::de::IgnoredAny as serde::Deserialize>::deserialize(deserializer)?;
            Ok(true)
        }

        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SchemaOneEnvelope {
            schema_version: u16,
            session_id: SessionId,
            session_mode: SessionMode,
            session_origin: SessionOrigin,
            tracks: Vec<CaptureTrackDescriptor>,
            track_configuration_revisions: Vec<TrackConfigurationRevision>,
            #[serde(rename = "source", default, deserialize_with = "field_is_present")]
            source_present: bool,
            started_at_ms: u64,
            sample_rate_hz: u32,
            chunks: Vec<CaptureChunkDescriptor>,
            degraded: bool,
        }

        let schema_one = SchemaOneEnvelope::deserialize(deserializer)?;
        if schema_one.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(serde::de::Error::custom(
                "unsupported manifest schema version",
            ));
        }
        if schema_one.source_present {
            return Err(serde::de::Error::custom(
                "schema 1 manifests cannot use the source field",
            ));
        }

        let manifest = Self {
            schema_version: schema_one.schema_version,
            session_id: schema_one.session_id,
            session_mode: schema_one.session_mode,
            session_origin: schema_one.session_origin,
            tracks: schema_one.tracks,
            track_configuration_revisions: schema_one.track_configuration_revisions,
            started_at_ms: schema_one.started_at_ms,
            sample_rate_hz: schema_one.sample_rate_hz,
            chunks: schema_one.chunks,
            degraded: schema_one.degraded,
        };

        validate_track_sources(manifest.session_origin, &manifest.tracks)
            .map_err(serde::de::Error::custom)?;
        validate_chunk_references(
            &manifest.session_id,
            manifest.session_mode,
            manifest.session_origin,
            &manifest.tracks,
            manifest.sample_rate_hz,
            &manifest.track_configuration_revisions,
            &manifest.chunks,
        )
        .map_err(serde::de::Error::custom)?;
        if manifest.chunks.iter().any(|chunk| !chunk.gaps.is_empty()) && !manifest.degraded {
            return Err(serde::de::Error::custom(
                "manifests containing gaps must be degraded",
            ));
        }

        Ok(manifest)
    }
}
