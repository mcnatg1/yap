use super::*;

#[test]
fn imported_sessions_reject_local_fallback_during_construction_and_deserialization() {
    let owner = crate::audio::session::OwnerNamespace::local("install-1").unwrap();
    let imported_track = CaptureTrackDescriptor::from_selector(
        track_id(),
        TrackSource::Imported {
            provenance: crate::audio::session::ImportedTrackProvenance::Unknown,
        },
        "install-1",
        "imported-file",
    );
    let encoded_audio = [1_u8, 2, 3];
    assert!(crate::audio::frame::AudioChunkEnvelope::from_frames(
        session_id(55),
        ChunkBuildContext {
            owner_namespace: &owner,
            session_mode: SessionMode::Dictation,
            session_origin: SessionOrigin::ImportedFile,
            track: &imported_track,
            route: AudioRoute::LocalFallback,
            audio_artifact_id: "imported-audio",
            encoded_audio: &encoded_audio,
        },
        &[frame(55, 1, 0, 20, 16_000)],
        AudioCodec::PcmS16Le,
        Vec::new(),
        AudioPurpose::LocalFallback,
    )
    .is_err());

    assert!(crate::audio::frame::AudioChunkEnvelope::from_frames(
        session_id(55),
        ChunkBuildContext {
            owner_namespace: &owner,
            session_mode: SessionMode::Dictation,
            session_origin: SessionOrigin::ImportedFile,
            track: &imported_track,
            route: AudioRoute::ServerBatch,
            audio_artifact_id: "imported-audio",
            encoded_audio: &encoded_audio,
        },
        &[frame(55, 1, 0, 20, 16_000)],
        AudioCodec::PcmS16Le,
        Vec::new(),
        AudioPurpose::LocalFallback,
    )
    .is_err());

    let mut builder = chunk_builder(55, AudioPurpose::CaptureEnvelope);
    builder.push(frame(55, 1, 0, 20, 16_000)).unwrap();
    let session = AudioSessionEnvelope {
        schema_version: MANIFEST_SCHEMA_VERSION,
        session_id: session_id(55),
        session_mode: SessionMode::Dictation,
        session_origin: SessionOrigin::LiveCapture,
        tracks: vec![track_descriptor()],
        track_configuration_revisions: Vec::new(),
        started_at_ms: 0,
        sample_rate_hz: 16_000,
        chunks: vec![builder.finish(Vec::new()).unwrap().capture_descriptor()],
        degraded: false,
    };
    let mut value = serde_json::to_value(session).unwrap();
    value["sessionOrigin"] = serde_json::json!("imported_file");
    value["tracks"][0]["source"] = serde_json::json!({
        "kind": "imported",
        "provenance": "unknown"
    });
    value["chunks"][0]["sessionOrigin"] = serde_json::json!("imported_file");
    value["chunks"][0]["trackSource"] = serde_json::json!({
        "kind": "imported",
        "provenance": "unknown"
    });
    value["chunks"][0]["route"] = serde_json::json!("local_fallback");

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn gaps_must_mark_manifests_degraded_without_consuming_entire_chunks() {
    let mut builder = chunk_builder(55, AudioPurpose::CaptureEnvelope);
    builder.push(frame(55, 1, 0, 20, 16_000)).unwrap();
    let session = AudioSessionEnvelope {
        schema_version: MANIFEST_SCHEMA_VERSION,
        session_id: session_id(55),
        session_mode: SessionMode::Dictation,
        session_origin: SessionOrigin::LiveCapture,
        tracks: vec![track_descriptor()],
        track_configuration_revisions: Vec::new(),
        started_at_ms: 0,
        sample_rate_hz: 16_000,
        chunks: vec![builder.finish(Vec::new()).unwrap().capture_descriptor()],
        degraded: false,
    };
    let mut value = serde_json::to_value(session).unwrap();
    value["chunks"][0]["gaps"] = serde_json::json!([{
        "sessionId": "s-55",
        "trackId": "mic-1",
        "startMs": 0,
        "durationMs": 20,
        "sourcePositionFrames": 0,
        "droppedFrames": 320,
        "cause": "sink_unavailable",
        "generation": 1
    }]);

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value.clone()).is_err());
    value["degraded"] = serde_json::json!(true);
    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());

    let owner = crate::audio::session::OwnerNamespace::local("install-1").unwrap();
    let track = track_descriptor();
    let retained_gap = AudioGap {
        session_id: session_id(55),
        track_id: track_id(),
        start_ms: 0,
        duration_ms: 20,
        source_position_frames: 0,
        dropped_frames: 320,
        cause: GapCause::SinkUnavailable,
        generation: 1,
    };
    assert!(
        crate::audio::frame::AudioChunkEnvelope::from_frames_with_continuity(
            session_id(55),
            chunk_context(&owner, &track, b"audio"),
            &[frame(55, 1, 0, 20, 16_000)],
            AudioCodec::PcmS16Le,
            Vec::new(),
            vec![retained_gap],
            AudioPurpose::CaptureEnvelope,
        )
        .is_err()
    );
}

#[test]
fn cross_chunk_sample_rate_changes_require_a_prior_track_configuration_revision() {
    let mut first_builder = chunk_builder(55, AudioPurpose::CaptureEnvelope);
    first_builder.push(frame(55, 1, 0, 20, 16_000)).unwrap();
    let first = first_builder.finish(Vec::new()).unwrap();
    let mut second_builder = chunk_builder(55, AudioPurpose::CaptureEnvelope);
    second_builder.push(frame(55, 2, 20, 20, 8_000)).unwrap();
    let second = second_builder.finish(Vec::new()).unwrap();

    let mut without_revision = AudioSessionEnvelopeBuilder::new(
        session_id(55),
        SessionMode::Dictation,
        SessionOrigin::LiveCapture,
        vec![track_descriptor()],
        0,
        16_000,
    );
    without_revision.push_chunk(first.clone());
    without_revision.push_chunk(second.clone());
    assert!(without_revision.finish().is_err());

    let mut with_revision = AudioSessionEnvelopeBuilder::new(
        session_id(55),
        SessionMode::Dictation,
        SessionOrigin::LiveCapture,
        vec![track_descriptor()],
        0,
        16_000,
    );
    with_revision.push_track_configuration_revision(
        crate::audio::frame::TrackConfigurationRevision::new(track_id(), 1, 20, 8_000).unwrap(),
    );
    with_revision.push_chunk(first);
    with_revision.push_chunk(second);
    assert!(with_revision.finish().is_ok());
}

#[test]
fn first_chunk_rate_must_match_the_persisted_track_configuration() {
    let mut first_builder = chunk_builder(55, AudioPurpose::CaptureEnvelope);
    first_builder.push(frame(55, 1, 0, 20, 8_000)).unwrap();

    let mut session = AudioSessionEnvelopeBuilder::new(
        session_id(55),
        SessionMode::Dictation,
        SessionOrigin::LiveCapture,
        vec![track_descriptor()],
        0,
        16_000,
    );
    session.push_chunk(first_builder.finish(Vec::new()).unwrap());

    assert!(session.finish().is_err());
}
