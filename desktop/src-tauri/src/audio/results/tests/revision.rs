use crate::audio::{
    evidence::{EvidenceQuality, ModelRevision},
    session::SessionId,
};

use super::super::{
    ResultAuthority, ResultStatus, SpeakerResultRevision, TranscriptResultRevision,
};
use super::{hash, turn};

#[test]
fn result_revisions_require_capture_hash_and_monotonic_revision() {
    let session_id = SessionId::new("s-result").unwrap();
    let model = ModelRevision::new("speaker-model", "r1", "calibration-r1").unwrap();

    assert!(SpeakerResultRevision::new(
        session_id.clone(),
        1,
        ResultAuthority::LocalProvisional,
        "",
        None,
        ResultStatus::Complete,
        vec![turn()],
        Vec::new(),
        vec![model.clone()],
    )
    .is_err());

    let first = SpeakerResultRevision::new(
        session_id,
        1,
        ResultAuthority::LocalProvisional,
        hash('a'),
        None,
        ResultStatus::Complete,
        vec![turn()],
        Vec::new(),
        vec![model],
    )
    .unwrap();

    assert!(first
        .next_revision(
            1,
            ResultAuthority::LocalReconciled,
            hash('a'),
            hash('b'),
            ResultStatus::Complete,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .is_err());
    assert!(first
        .next_revision(
            2,
            ResultAuthority::LocalReconciled,
            hash('a'),
            "",
            ResultStatus::Complete,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .is_err());
    assert!(first
        .next_revision(
            2,
            ResultAuthority::LocalReconciled,
            hash('a'),
            hash('b'),
            ResultStatus::Complete,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .is_ok());
}

#[test]
fn evidence_and_result_json_contains_no_embedding_or_exemplar_values() {
    let evidence = crate::audio::evidence::SpeakerEvidence::new(
        crate::audio::session::TrackId::new("mic-1").unwrap(),
        0,
        20,
        Some("slot-1".into()),
        ModelRevision::new("speaker-model", "r1", "calibration-r1").unwrap(),
        EvidenceQuality::Clean,
        Some(0.8),
    )
    .unwrap();
    let result = SpeakerResultRevision::new(
        SessionId::new("s-result").unwrap(),
        1,
        ResultAuthority::LocalProvisional,
        hash('a'),
        None,
        ResultStatus::Partial,
        vec![turn()],
        Vec::new(),
        vec![ModelRevision::new("speaker-model", "r1", "calibration-r1").unwrap()],
    )
    .unwrap();

    let serialized = format!(
        "{}{}",
        serde_json::to_string(&evidence).unwrap(),
        serde_json::to_string(&result).unwrap()
    );
    assert!(!serialized.contains("embedding"));
    assert!(!serialized.contains("exemplar"));
}

#[test]
fn transcript_revisions_require_a_valid_predecessor_hash_and_next_revision() {
    let first = TranscriptResultRevision::new(
        SessionId::new("s-result").unwrap(),
        1,
        ResultAuthority::LocalProvisional,
        hash('a'),
        None,
        ResultStatus::Complete,
        "hello",
        Vec::new(),
        Vec::new(),
    )
    .unwrap();

    assert!(first
        .next_revision(
            1,
            ResultAuthority::LocalReconciled,
            hash('a'),
            hash('b'),
            ResultStatus::Complete,
            "hello again",
            Vec::new(),
            Vec::new(),
        )
        .is_err());
}

#[test]
fn result_revision_chains_keep_the_capture_sidecar_immutable() {
    let first = TranscriptResultRevision::new(
        SessionId::new("s-result").unwrap(),
        1,
        ResultAuthority::LocalProvisional,
        hash('a'),
        None,
        ResultStatus::Complete,
        "hello",
        Vec::new(),
        Vec::new(),
    )
    .unwrap();

    assert!(first
        .next_revision(
            2,
            ResultAuthority::LocalReconciled,
            hash('c'),
            hash('b'),
            ResultStatus::Complete,
            "hello again",
            Vec::new(),
            Vec::new(),
        )
        .is_err());
}
