use super::{ClientSpeakerAttribution, SpeakerAttribution};

#[test]
fn client_evidence_builder_can_emit_only_unknown_or_session_speaker() {
    assert_eq!(
        ClientSpeakerAttribution::unknown(),
        SpeakerAttribution::Unknown
    );
    let session_speaker = ClientSpeakerAttribution::session_speaker("speaker-1").unwrap();
    assert!(matches!(
        session_speaker,
        SpeakerAttribution::SessionSpeaker(_)
    ));
    assert!(ClientSpeakerAttribution::session_speaker("").is_err());
}

#[test]
fn evidence_json_cannot_bypass_interval_confidence_or_named_provenance_validation() {
    let invalid_evidence = serde_json::json!({
        "trackId": "mic-1",
        "startMs": 20,
        "endMs": 20,
        "localSlotId": "slot-1",
        "model": {
            "modelId": "speaker-model",
            "revision": "r1",
            "calibrationRevision": "calibration-r1"
        },
        "quality": "clean",
        "confidence": 1.2
    });
    assert!(serde_json::from_value::<super::SpeakerEvidence>(invalid_evidence).is_err());

    let invalid_named = serde_json::json!({
        "named": {
            "identityId": "",
            "profileRevision": "profile-r1",
            "model": {
                "modelId": "speaker-model",
                "revision": "r1",
                "calibrationRevision": "calibration-r1"
            },
            "confidenceMicros": 1_100_000,
            "purposeGrantId": "grant-1",
            "revocationEpoch": 1
        }
    });
    assert!(serde_json::from_value::<super::SpeakerAttribution>(invalid_named).is_err());
}

#[test]
fn client_attribution_json_cannot_mint_a_named_speaker() {
    let named = serde_json::json!({
        "named": {
            "identityId": "identity-1",
            "profileRevision": "profile-r1",
            "model": {
                "modelId": "speaker-model",
                "revision": "r1",
                "calibrationRevision": "calibration-r1"
            },
            "confidenceMicros": 900_000,
            "purposeGrantId": "grant-1",
            "revocationEpoch": 1
        }
    });

    assert!(serde_json::from_value::<super::SpeakerAttribution>(named).is_err());
}
