use super::{
    build_manifest_windows, classify_replay, AudioChunkEnvelopeBuilder, AudioSessionEnvelope,
    AudioSessionEnvelopeBuilder, ChunkWindowConfig, ReplayConflict, MANIFEST_SCHEMA_VERSION,
};
use crate::audio::frame::{
    AudioCodec, AudioFrame, AudioGap, AudioPurpose, AudioRoute, ChunkBuildContext, GapCause,
    PreparedFrame, VadSegment,
};
use crate::audio::session::{
    CaptureSource, CaptureTrackDescriptor, SessionId, SessionMode, SessionOrigin, TrackId,
    TrackSource,
};
use crate::audio::vad::{VadDecision, VadKind};

fn frame(
    session_number: u64,
    sequence: u64,
    start_ms: u64,
    duration_ms: u32,
    sample_rate_hz: u32,
) -> AudioFrame {
    AudioFrame {
        session_id: session_id(session_number),
        track_id: track_id(),
        sequence,
        sample_rate_hz,
        channels: 1,
        start_ms,
        duration_ms,
        sample_count: 320,
    }
}

fn session_id(value: u64) -> SessionId {
    SessionId::new(format!("s-{value}")).unwrap()
}

fn track_id() -> TrackId {
    TrackId::new("mic-1").unwrap()
}

fn track_descriptor() -> CaptureTrackDescriptor {
    CaptureTrackDescriptor::from_selector(
        track_id(),
        TrackSource::Captured {
            source: CaptureSource::Microphone,
        },
        "install-id",
        "0:Built-in Microphone",
    )
}

fn schema_one_manifest_json() -> serde_json::Value {
    serde_json::to_value(AudioSessionEnvelope {
        schema_version: MANIFEST_SCHEMA_VERSION,
        session_id: session_id(7),
        session_mode: SessionMode::Dictation,
        session_origin: SessionOrigin::LiveCapture,
        tracks: vec![track_descriptor()],
        track_configuration_revisions: Vec::new(),
        started_at_ms: 0,
        sample_rate_hz: 16_000,
        chunks: Vec::new(),
        degraded: false,
    })
    .unwrap()
}

fn chunk_context<'a>(
    owner_namespace: &'a crate::audio::session::OwnerNamespace,
    track: &'a CaptureTrackDescriptor,
    encoded_audio: &'a [u8],
) -> ChunkBuildContext<'a> {
    ChunkBuildContext {
        owner_namespace,
        session_mode: SessionMode::Dictation,
        session_origin: SessionOrigin::LiveCapture,
        track,
        route: AudioRoute::ServerBatch,
        audio_artifact_id: "artifact-1",
        encoded_audio,
    }
}

fn chunk_builder(session_number: u64, purpose: AudioPurpose) -> AudioChunkEnvelopeBuilder<'static> {
    let owner_namespace = Box::leak(Box::new(
        crate::audio::session::OwnerNamespace::local("legacy-window").unwrap(),
    ));
    let track = Box::leak(Box::new(track_descriptor()));
    let encoded_audio = Box::leak(Box::new([1_u8, 2, 3]));
    AudioChunkEnvelopeBuilder::new(
        session_id(session_number),
        chunk_context(owner_namespace, track, encoded_audio),
        purpose,
        AudioCodec::PcmS16Le,
    )
}

fn window_config(preserve_silence_markers: bool) -> ChunkWindowConfig {
    ChunkWindowConfig {
        target_window_ms: 40,
        max_window_ms: 80,
        tail_padding_ms: 20,
        preserve_silence_markers,
    }
}

fn windows(
    session_id: u64,
    frames: &[AudioFrame],
    vad: &[VadDecision],
    purpose: AudioPurpose,
    codec: AudioCodec,
    config: ChunkWindowConfig,
) -> Vec<crate::audio::frame::AudioChunkEnvelope> {
    let prepared = frames
        .iter()
        .cloned()
        .map(|metadata| {
            let samples = vec![0.0; metadata.sample_count];
            prepared_frame(metadata, &samples)
        })
        .collect::<Vec<_>>();
    build_manifest_windows(session_id, &prepared, vad, purpose, codec, config).unwrap()
}

fn prepared_frame(metadata: AudioFrame, samples: &[f32]) -> PreparedFrame {
    PreparedFrame {
        metadata,
        samples: std::sync::Arc::from(samples),
    }
}

mod builder;
mod contract_validation;
mod schema_contract;
mod windows_core;
mod windows_integrity;
