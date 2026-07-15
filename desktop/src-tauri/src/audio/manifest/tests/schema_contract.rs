use super::*;

#[test]
fn schema_one_manifest_round_trips_unchanged() {
    let value = schema_one_manifest_json();
    let manifest = serde_json::from_value::<AudioSessionEnvelope>(value.clone()).unwrap();

    assert_eq!(serde_json::to_value(manifest).unwrap(), value);
}

#[test]
fn missing_manifest_schema_version_is_rejected() {
    let mut value = schema_one_manifest_json();
    value.as_object_mut().unwrap().remove("schemaVersion");

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn manifest_schema_zero_is_rejected() {
    let mut value = schema_one_manifest_json();
    value["schemaVersion"] = serde_json::json!(0);

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn unknown_manifest_schema_version_is_rejected() {
    let mut value = schema_one_manifest_json();
    value["schemaVersion"] = serde_json::json!(2);

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn numeric_manifest_session_ids_are_rejected() {
    let mut value = schema_one_manifest_json();
    value["sessionId"] = serde_json::json!(7);

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn old_manifest_source_field_is_rejected() {
    let mut value = schema_one_manifest_json();
    value["source"] = serde_json::json!("live");

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn null_old_manifest_source_field_is_rejected() {
    let mut value = schema_one_manifest_json();
    value["source"] = serde_json::Value::Null;

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn unrelated_unknown_manifest_fields_are_tolerated() {
    let mut value = schema_one_manifest_json();
    value["futureField"] = serde_json::Value::Null;

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_ok());
}

#[test]
fn schema_zero_compatibility_shape_is_rejected() {
    let value = serde_json::json!({
        "schemaVersion": 0,
        "sessionId": 7,
        "source": "live",
        "startedAtMs": 0,
        "sampleRateHz": 16_000,
        "chunks": [],
        "degraded": false
    });

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn unversioned_manifest_with_incomplete_numeric_chunk_is_rejected() {
    let value = serde_json::json!({
        "sessionId": 7,
        "source": "live",
        "startedAtMs": 0,
        "sampleRateHz": 16_000,
        "chunks": [{
            "sessionId": 7,
            "chunkId": "7-1-20",
            "sequenceStart": 1,
            "startMs": 0,
            "durationMs": 20,
            "sampleRateHz": 16_000,
            "codec": "pcm_s16_le",
            "vadSegments": [],
            "purpose": "captureEnvelope",
            "retry": {
                "idempotencyKey": "7-1-7-1-20",
                "attempt": 1,
                "maxAttempts": 1
            }
        }],
        "degraded": false
    });

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn manifest_deserialization_rejects_origin_and_track_source_mismatch() {
    let error = serde_json::from_value::<AudioSessionEnvelope>(serde_json::json!({
        "schemaVersion": MANIFEST_SCHEMA_VERSION,
        "sessionId": "s-imported",
        "sessionMode": "dictation",
        "sessionOrigin": "imported_file",
        "tracks": [{
            "trackId": "mic-1",
            "source": { "kind": "captured", "source": "microphone" },
            "deviceId": "dev-opaque"
        }],
        "trackConfigurationRevisions": [],
        "startedAtMs": 0,
        "sampleRateHz": 16_000,
        "chunks": [],
        "degraded": false
    }))
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("ImportedFile sessions must contain only imported tracks"));
}

#[test]
fn manifest_deserialization_rejects_chunk_replay_key_mismatch() {
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
    value["chunks"][0]["replayKey"]["sequenceEnd"] = serde_json::json!(2);

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn manifest_device_reference_is_opaque_and_does_not_contain_the_os_label() {
    let descriptor = crate::audio::session::CaptureTrackDescriptor::from_selector(
        crate::audio::session::TrackId::new("mic-1").unwrap(),
        crate::audio::session::TrackSource::Captured {
            source: crate::audio::session::CaptureSource::Microphone,
        },
        "install-id",
        "0:Built-in Microphone",
    );

    assert!(descriptor.device_id.starts_with("dev-"));
    assert!(!descriptor.device_id.contains("Built-in Microphone"));
}

#[test]
fn manifest_chunk_missing_identity_fields_is_rejected() {
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
    value["chunks"][0]
        .as_object_mut()
        .unwrap()
        .remove("contentIdentity");

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn manifest_chunk_missing_modern_fields_is_rejected() {
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
    let chunk = value["chunks"][0].as_object_mut().unwrap();
    for field in [
        "replayKey",
        "contentIdentity",
        "sessionMode",
        "sessionOrigin",
        "trackSource",
        "route",
        "audioArtifactId",
        "gaps",
    ] {
        chunk.remove(field);
    }

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn current_manifest_requires_persisted_configuration_revision_field() {
    let session = AudioSessionEnvelope {
        schema_version: MANIFEST_SCHEMA_VERSION,
        session_id: session_id(55),
        session_mode: SessionMode::Dictation,
        session_origin: SessionOrigin::LiveCapture,
        tracks: vec![track_descriptor()],
        track_configuration_revisions: Vec::new(),
        started_at_ms: 0,
        sample_rate_hz: 16_000,
        chunks: Vec::new(),
        degraded: false,
    };
    let mut value = serde_json::to_value(session).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("trackConfigurationRevisions");

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}

#[test]
fn current_chunks_reject_unknown_replay_key_schema_versions() {
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
    value["chunks"][0]["replayKey"]["schemaVersion"] = serde_json::json!(2);

    assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
}
