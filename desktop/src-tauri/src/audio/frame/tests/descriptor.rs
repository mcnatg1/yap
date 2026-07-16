use super::{current_descriptor_json, incomplete_chunk_json};
use crate::audio::frame::CaptureChunkDescriptor;

#[test]
fn schema_one_descriptor_round_trips_unchanged() {
    let value = current_descriptor_json();
    let descriptor = serde_json::from_value::<CaptureChunkDescriptor>(value.clone()).unwrap();

    assert_eq!(serde_json::to_value(descriptor).unwrap(), value);
}

#[test]
fn descriptor_missing_replay_schema_version_is_rejected() {
    let mut value = current_descriptor_json();
    value["replayKey"]
        .as_object_mut()
        .unwrap()
        .remove("schemaVersion");

    assert!(serde_json::from_value::<CaptureChunkDescriptor>(value).is_err());
}

#[test]
fn descriptor_replay_schema_zero_is_rejected() {
    let mut value = current_descriptor_json();
    value["replayKey"]["schemaVersion"] = serde_json::json!(0);

    assert!(serde_json::from_value::<CaptureChunkDescriptor>(value).is_err());
}

#[test]
fn descriptor_unknown_replay_schema_version_is_rejected() {
    let mut value = current_descriptor_json();
    value["replayKey"]["schemaVersion"] = serde_json::json!(2);

    assert!(serde_json::from_value::<CaptureChunkDescriptor>(value).is_err());
}

#[test]
fn numeric_chunk_session_ids_are_rejected() {
    let mut value = current_descriptor_json();
    value["sessionId"] = serde_json::json!(7);

    assert!(serde_json::from_value::<CaptureChunkDescriptor>(value).is_err());
}

#[test]
fn numeric_replay_key_session_ids_are_rejected() {
    let mut value = current_descriptor_json();
    value["replayKey"]["sessionId"] = serde_json::json!(7);

    assert!(serde_json::from_value::<CaptureChunkDescriptor>(value).is_err());
}

#[test]
fn incomplete_chunk_payloads_are_rejected() {
    assert!(serde_json::from_value::<CaptureChunkDescriptor>(incomplete_chunk_json()).is_err());
}

#[test]
fn current_descriptor_json_rejects_local_contract_violations() {
    let value = current_descriptor_json();

    let mut bad_chunk_id = value.clone();
    bad_chunk_id["chunkId"] = serde_json::json!("chunk-tampered");
    assert!(serde_json::from_value::<CaptureChunkDescriptor>(bad_chunk_id).is_err());

    let mut bad_rate = value.clone();
    bad_rate["sampleRateHz"] = serde_json::json!(0);
    assert!(serde_json::from_value::<CaptureChunkDescriptor>(bad_rate).is_err());

    let mut bad_vad = value.clone();
    bad_vad["vadSegments"] = serde_json::json!([{
        "startMs": 0,
        "endMs": 21,
        "kind": "speech",
        "rms": 0.3
    }]);
    assert!(serde_json::from_value::<CaptureChunkDescriptor>(bad_vad).is_err());

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
    assert!(serde_json::from_value::<CaptureChunkDescriptor>(full_gap).is_err());
}
