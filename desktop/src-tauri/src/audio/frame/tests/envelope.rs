use super::{context, frame, track};
use crate::audio::{
    frame::{AudioChunkEnvelope, AudioCodec, AudioPurpose, VadSegment},
    session::{OwnerNamespace, SessionId, TrackId},
    vad::VadKind,
};

#[test]
fn from_frames_rejects_empty_or_mixed_track_lists() {
    let owner = OwnerNamespace::local("install-1").unwrap();
    let track = track("install-1", "mic-1");
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
    let track = track("install-1", "mic-1");

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
    let track = track("install-1", "mic-1");
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
    let track = track("install-1", "mic-1");
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
    let track = track("install-1", "mic-1");
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
