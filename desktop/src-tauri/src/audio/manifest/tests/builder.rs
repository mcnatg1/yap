use super::*;

#[test]
fn builder_rejects_cross_session_cross_track_and_sequence_regression() {
    let owner_namespace = crate::audio::session::OwnerNamespace::local("install-1").unwrap();
    let track = track_descriptor();
    let encoded_audio = [1_u8, 2, 3];

    let mut cross_session = AudioChunkEnvelopeBuilder::new(
        session_id(7),
        chunk_context(&owner_namespace, &track, &encoded_audio),
        AudioPurpose::CaptureEnvelope,
        AudioCodec::PcmS16Le,
    );
    assert!(cross_session.push(frame(8, 1, 0, 20, 16_000)).is_err());

    let mut cross_track = AudioChunkEnvelopeBuilder::new(
        session_id(7),
        chunk_context(&owner_namespace, &track, &encoded_audio),
        AudioPurpose::CaptureEnvelope,
        AudioCodec::PcmS16Le,
    );
    let mut wrong_track = frame(7, 1, 0, 20, 16_000);
    wrong_track.track_id = TrackId::new("mic-2").unwrap();
    assert!(cross_track.push(wrong_track).is_err());

    let mut regression = AudioChunkEnvelopeBuilder::new(
        session_id(7),
        chunk_context(&owner_namespace, &track, &encoded_audio),
        AudioPurpose::CaptureEnvelope,
        AudioCodec::PcmS16Le,
    );
    regression.push(frame(7, 2, 20, 20, 16_000)).unwrap();
    assert!(regression.push(frame(7, 1, 0, 20, 16_000)).is_err());
}

#[test]
fn builder_rejects_impossible_or_overlapping_frame_timing() {
    let owner_namespace = crate::audio::session::OwnerNamespace::local("install-1").unwrap();
    let track = track_descriptor();
    let encoded_audio = [1_u8, 2, 3];

    let mut impossible = AudioChunkEnvelopeBuilder::new(
        session_id(7),
        chunk_context(&owner_namespace, &track, &encoded_audio),
        AudioPurpose::CaptureEnvelope,
        AudioCodec::PcmS16Le,
    );
    assert!(impossible.push(frame(7, 1, 0, 0, 16_000)).is_err());

    let mut overlapping = AudioChunkEnvelopeBuilder::new(
        session_id(7),
        chunk_context(&owner_namespace, &track, &encoded_audio),
        AudioPurpose::CaptureEnvelope,
        AudioCodec::PcmS16Le,
    );
    overlapping.push(frame(7, 1, 0, 20, 16_000)).unwrap();
    assert!(overlapping.push(frame(7, 2, 19, 20, 16_000)).is_err());
}

#[test]
fn session_envelope_serializes_with_expected_field_names() {
    let mut builder = chunk_builder(55, AudioPurpose::CaptureEnvelope);
    builder.push(frame(55, 2, 40, 20, 16_000)).unwrap();
    builder.push(frame(55, 3, 60, 20, 16_000)).unwrap();
    let descriptor = builder
        .finish(vec![VadSegment {
            start_ms: 40,
            end_ms: 80,
            kind: VadKind::Speech,
            rms: 0.33,
        }])
        .unwrap()
        .capture_descriptor();
    let session = AudioSessionEnvelope {
        schema_version: MANIFEST_SCHEMA_VERSION,
        session_id: session_id(55),
        session_mode: SessionMode::Dictation,
        session_origin: SessionOrigin::LiveCapture,
        tracks: vec![track_descriptor()],
        track_configuration_revisions: Vec::new(),
        started_at_ms: 1_000,
        sample_rate_hz: 16_000,
        chunks: vec![descriptor],
        degraded: true,
    };

    let value = serde_json::to_value(&session).expect("session envelope should serialize");

    assert_eq!(value["sessionId"], "s-55");
    assert_eq!(value["sessionMode"], "dictation");
    assert_eq!(value["sessionOrigin"], "live_capture");
    assert!(value.get("source").is_none());
    assert_eq!(value["startedAtMs"], 1_000);
    assert_eq!(value["sampleRateHz"], 16_000);
    assert_eq!(value["degraded"], true);
    assert!(value["chunks"][0]["chunkId"]
        .as_str()
        .unwrap()
        .starts_with("chunk-"));
    assert!(value["chunks"][0].get("retry").is_none());
    assert_eq!(value["chunks"][0]["contentIdentity"]["byteLength"], 3);
}

#[test]
fn chunk_builder_accepts_contiguous_frames_in_sequence_order() {
    let mut builder = chunk_builder(7, AudioPurpose::CaptureEnvelope);
    builder.push(frame(7, 11, 100, 20, 16_000)).unwrap();
    builder.push(frame(7, 12, 120, 20, 16_000)).unwrap();

    let vad_segments = vec![VadSegment {
        start_ms: 100,
        end_ms: 140,
        kind: VadKind::Speech,
        rms: 0.42,
    }];

    let envelope = builder
        .finish(vad_segments.clone())
        .expect("frames should build an envelope");

    assert_eq!(envelope.session_id, session_id(7));
    assert_eq!(envelope.track_id, track_id());
    assert_eq!(envelope.sequence_start, 11);
    assert_eq!(envelope.start_ms, 100);
    assert_eq!(envelope.duration_ms, 40);
    assert_eq!(envelope.sample_rate_hz, 16_000);
    assert_eq!(envelope.codec, AudioCodec::PcmS16Le);
    assert_eq!(envelope.vad_segments, vad_segments);
    assert_eq!(envelope.purpose, AudioPurpose::CaptureEnvelope);
}

#[test]
fn chunk_builder_returns_none_for_empty_builders() {
    let builder = chunk_builder(7, AudioPurpose::LocalFallback);

    assert!(builder.finish(Vec::new()).is_err());
}

#[test]
fn chunk_builder_sets_retry_and_idempotency_fields() {
    let mut builder = chunk_builder(7, AudioPurpose::LocalFallback);
    builder.push(frame(7, 11, 100, 20, 16_000)).unwrap();
    builder.push(frame(7, 12, 120, 20, 16_000)).unwrap();

    let envelope = builder
        .finish(vec![VadSegment {
            start_ms: 100,
            end_ms: 140,
            kind: VadKind::Speech,
            rms: 0.42,
        }])
        .expect("frames should build an envelope");

    assert_eq!(envelope.retry.idempotency_key, envelope.chunk_id);
    assert_eq!(envelope.retry.attempt, 1);
    assert_eq!(envelope.retry.max_attempts, 1);
}

#[test]
fn session_builder_collects_chunks_and_marks_degraded() {
    let mut first_chunk_builder = chunk_builder(55, AudioPurpose::CaptureEnvelope);
    first_chunk_builder
        .push(frame(55, 4, 80, 20, 16_000))
        .unwrap();
    first_chunk_builder
        .push(frame(55, 5, 100, 20, 16_000))
        .unwrap();
    let first_chunk = first_chunk_builder
        .finish(vec![VadSegment {
            start_ms: 80,
            end_ms: 120,
            kind: VadKind::Silence,
            rms: 0.0,
        }])
        .expect("first chunk should build");

    let mut second_chunk_builder = chunk_builder(55, AudioPurpose::CaptureEnvelope);
    second_chunk_builder
        .push(frame(55, 2, 40, 20, 16_000))
        .unwrap();
    second_chunk_builder
        .push(frame(55, 3, 60, 20, 16_000))
        .unwrap();
    let second_chunk = second_chunk_builder
        .finish(vec![VadSegment {
            start_ms: 40,
            end_ms: 80,
            kind: VadKind::Speech,
            rms: 0.33,
        }])
        .expect("second chunk should build");

    let mut session_builder = AudioSessionEnvelopeBuilder::new(
        session_id(55),
        SessionMode::Dictation,
        SessionOrigin::LiveCapture,
        vec![track_descriptor()],
        1_000,
        16_000,
    );
    session_builder.push_chunk(first_chunk);
    session_builder.push_chunk(second_chunk);
    session_builder.mark_degraded();

    let session = session_builder.finish().unwrap();

    assert_eq!(session.session_id, session_id(55));
    assert_eq!(session.session_origin, SessionOrigin::LiveCapture);
    assert_eq!(session.started_at_ms, 1_000);
    assert_eq!(session.sample_rate_hz, 16_000);
    assert!(session.degraded);
    assert_eq!(session.chunks.len(), 2);
    assert_eq!(session.chunks[0].sequence_start, 2);
    assert_eq!(session.chunks[1].sequence_start, 4);
}

#[test]
fn session_builder_rejects_origin_and_track_source_mismatch() {
    let builder = AudioSessionEnvelopeBuilder::new(
        session_id(55),
        SessionMode::Dictation,
        SessionOrigin::ImportedFile,
        vec![track_descriptor()],
        1_000,
        16_000,
    );

    assert!(builder.finish().is_err());
}
