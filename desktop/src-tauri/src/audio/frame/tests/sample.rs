use super::frame;
use crate::audio::frame::{AudioFrame, PreparedFrame, TrackConfigurationRevision};

#[test]
fn duration_ms_from_samples_uses_session_relative_sample_math() {
    assert_eq!(AudioFrame::duration_ms_from_samples(320, 16_000), 20);
    assert_eq!(AudioFrame::duration_ms_from_samples(16_000, 16_000), 1_000);
    assert_eq!(AudioFrame::duration_ms_from_samples(0, 16_000), 0);
}

#[test]
fn end_ms_uses_saturating_frame_coverage() {
    assert_eq!(frame(11, u64::MAX - 5, 10, 320).end_ms(), u64::MAX);
}

#[test]
fn prepared_frames_keep_samples_out_of_serializable_metadata() {
    let metadata = frame(1, 0, 20, 320);
    let prepared = PreparedFrame {
        metadata: metadata.clone(),
        samples: std::sync::Arc::from([0.0_f32, 0.25_f32]),
    };
    let value = serde_json::to_value(metadata).unwrap();
    assert!(value.get("samples").is_none());
    assert_eq!(prepared.samples.len(), 2);
    assert_eq!(prepared.metadata.track_id.as_str(), "mic-1");
}

#[test]
fn configuration_revision_json_cannot_bypass_field_validation() {
    let invalid = serde_json::json!({
        "trackId": "mic-1",
        "revision": 0,
        "effectiveAtMs": 20,
        "sampleRateHz": 0
    });

    assert!(serde_json::from_value::<TrackConfigurationRevision>(invalid).is_err());
}
