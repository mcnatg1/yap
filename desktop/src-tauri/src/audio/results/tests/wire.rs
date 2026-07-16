use super::super::{SpeakerResultRevision, TranscriptResultRevision};
use super::hash;

#[test]
fn result_json_rejects_bad_hashes_and_zero_revision_for_both_result_types() {
    let invalid = serde_json::json!({
        "sessionId": "s-result",
        "revision": 0,
        "authority": "local_provisional",
        "captureSidecarSha256": "not-a-sha256",
        "previousResultSha256": null,
        "status": "complete",
        "speakerTurns": [],
        "alignedWords": [],
        "modelProvenance": []
    });
    assert!(serde_json::from_value::<SpeakerResultRevision>(invalid.clone()).is_err());

    let mut transcript = invalid;
    let object = transcript.as_object_mut().unwrap();
    object.remove("speakerTurns");
    object.insert("transcript".into(), serde_json::json!("hello"));
    assert!(serde_json::from_value::<TranscriptResultRevision>(transcript).is_err());
}

#[test]
fn local_result_json_cannot_claim_named_server_attribution() {
    let local_named = serde_json::json!({
        "sessionId": "s-result",
        "revision": 1,
        "authority": "local_provisional",
        "captureSidecarSha256": hash('a'),
        "previousResultSha256": null,
        "status": "complete",
        "speakerTurns": [{
            "turnId": "turn-1",
            "startMs": 0,
            "endMs": 20,
            "attribution": {
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
            },
            "confidence": 0.9
        }],
        "alignedWords": [],
        "modelProvenance": []
    });

    assert!(serde_json::from_value::<SpeakerResultRevision>(local_named).is_err());
}

#[test]
fn server_result_json_accepts_named_attribution_with_complete_provenance() {
    let server_named = serde_json::json!({
        "sessionId": "s-result",
        "revision": 1,
        "authority": "server_authoritative",
        "captureSidecarSha256": hash('a'),
        "previousResultSha256": null,
        "status": "complete",
        "speakerTurns": [{
            "turnId": "turn-1",
            "startMs": 0,
            "endMs": 20,
            "attribution": {
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
            },
            "confidence": 0.9
        }],
        "alignedWords": [],
        "modelProvenance": []
    });

    assert!(serde_json::from_value::<SpeakerResultRevision>(server_named).is_ok());
}
