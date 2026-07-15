use crate::audio::{
    frame::{
        AudioChunkEnvelope, AudioCodec, AudioFrame, AudioPurpose, AudioRoute, ChunkBuildContext,
        ChunkReplayKey, ContentIdentity,
    },
    manifest::{classify_replay, ReplayConflict, ReplayDecision},
    session::{
        CaptureSource, CaptureTrackDescriptor, OwnerNamespace, SessionId, SessionMode,
        SessionOrigin, TrackId, TrackSource,
    },
};

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
fn chunk_ids_are_collision_safe_for_hyphenated_replay_key_components() {
    fn envelope(install_id: &str, session: &str) -> AudioChunkEnvelope {
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
