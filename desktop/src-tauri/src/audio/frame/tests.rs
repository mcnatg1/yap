mod descriptor;
mod envelope;
mod identity;
mod sample;

use super::{
    AudioChunkEnvelope, AudioCodec, AudioFrame, AudioPurpose, AudioRoute, ChunkBuildContext,
};
use crate::audio::session::{
    CaptureSource, CaptureTrackDescriptor, OwnerNamespace, SessionId, SessionMode, SessionOrigin,
    TrackId, TrackSource,
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

fn track(install_id: &str, track_id: &str) -> CaptureTrackDescriptor {
    CaptureTrackDescriptor::from_selector(
        TrackId::new(track_id).unwrap(),
        TrackSource::Captured {
            source: CaptureSource::Microphone,
        },
        install_id,
        "mic",
    )
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

fn current_descriptor_json() -> serde_json::Value {
    let owner = OwnerNamespace::local("install-1").unwrap();
    let track = track("install-1", "mic-1");
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
