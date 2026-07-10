use crate::audio::{
    evidence::{AlignedWord, ModelRevision, ServerAlignedWord, ServerSpeakerTurn, SpeakerTurn},
    session::SessionId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultAuthority {
    LocalProvisional,
    LocalReconciled,
    ServerAuthoritative,
    UserCorrected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    Complete,
    Partial,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptResultRevision {
    session_id: SessionId,
    revision: u64,
    authority: ResultAuthority,
    capture_sidecar_sha256: String,
    previous_result_sha256: Option<String>,
    status: ResultStatus,
    transcript: String,
    aligned_words: Vec<AlignedWord>,
    model_provenance: Vec<ModelRevision>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerResultRevision {
    session_id: SessionId,
    revision: u64,
    authority: ResultAuthority,
    capture_sidecar_sha256: String,
    previous_result_sha256: Option<String>,
    status: ResultStatus,
    speaker_turns: Vec<SpeakerTurn>,
    aligned_words: Vec<AlignedWord>,
    model_provenance: Vec<ModelRevision>,
}

impl SpeakerResultRevision {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: SessionId,
        revision: u64,
        authority: ResultAuthority,
        capture_sidecar_sha256: impl Into<String>,
        previous_result_sha256: Option<String>,
        status: ResultStatus,
        speaker_turns: Vec<SpeakerTurn>,
        aligned_words: Vec<AlignedWord>,
        model_provenance: Vec<ModelRevision>,
    ) -> Result<Self, ResultRevisionError> {
        let capture_sidecar_sha256 = capture_sidecar_sha256.into();
        validate_root_revision(
            revision,
            &capture_sidecar_sha256,
            previous_result_sha256.as_deref(),
        )?;
        validate_named_attribution_authority(authority, &speaker_turns, &aligned_words)?;
        Ok(Self {
            session_id,
            revision,
            authority,
            capture_sidecar_sha256,
            previous_result_sha256,
            status,
            speaker_turns,
            aligned_words,
            model_provenance,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn next_revision(
        &self,
        revision: u64,
        authority: ResultAuthority,
        capture_sidecar_sha256: impl Into<String>,
        previous_result_sha256: impl Into<String>,
        status: ResultStatus,
        speaker_turns: Vec<SpeakerTurn>,
        aligned_words: Vec<AlignedWord>,
        model_provenance: Vec<ModelRevision>,
    ) -> Result<Self, ResultRevisionError> {
        let capture_sidecar_sha256 = capture_sidecar_sha256.into();
        let previous_result_sha256 = previous_result_sha256.into();
        validate_next_revision(
            self.revision,
            revision,
            &self.capture_sidecar_sha256,
            &capture_sidecar_sha256,
            &previous_result_sha256,
        )?;
        validate_named_attribution_authority(authority, &speaker_turns, &aligned_words)?;
        Ok(Self {
            session_id: self.session_id.clone(),
            revision,
            authority,
            capture_sidecar_sha256,
            previous_result_sha256: Some(previous_result_sha256),
            status,
            speaker_turns,
            aligned_words,
            model_provenance,
        })
    }
}

impl TranscriptResultRevision {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: SessionId,
        revision: u64,
        authority: ResultAuthority,
        capture_sidecar_sha256: impl Into<String>,
        previous_result_sha256: Option<String>,
        status: ResultStatus,
        transcript: impl Into<String>,
        aligned_words: Vec<AlignedWord>,
        model_provenance: Vec<ModelRevision>,
    ) -> Result<Self, ResultRevisionError> {
        let capture_sidecar_sha256 = capture_sidecar_sha256.into();
        validate_root_revision(
            revision,
            &capture_sidecar_sha256,
            previous_result_sha256.as_deref(),
        )?;
        validate_named_attribution_authority(authority, &[], &aligned_words)?;
        Ok(Self {
            session_id,
            revision,
            authority,
            capture_sidecar_sha256,
            previous_result_sha256,
            status,
            transcript: transcript.into(),
            aligned_words,
            model_provenance,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn next_revision(
        &self,
        revision: u64,
        authority: ResultAuthority,
        capture_sidecar_sha256: impl Into<String>,
        previous_result_sha256: impl Into<String>,
        status: ResultStatus,
        transcript: impl Into<String>,
        aligned_words: Vec<AlignedWord>,
        model_provenance: Vec<ModelRevision>,
    ) -> Result<Self, ResultRevisionError> {
        let capture_sidecar_sha256 = capture_sidecar_sha256.into();
        let previous_result_sha256 = previous_result_sha256.into();
        validate_next_revision(
            self.revision,
            revision,
            &self.capture_sidecar_sha256,
            &capture_sidecar_sha256,
            &previous_result_sha256,
        )?;
        validate_named_attribution_authority(authority, &[], &aligned_words)?;
        Ok(Self {
            session_id: self.session_id.clone(),
            revision,
            authority,
            capture_sidecar_sha256,
            previous_result_sha256: Some(previous_result_sha256),
            status,
            transcript: transcript.into(),
            aligned_words,
            model_provenance,
        })
    }
}

fn validate_root_revision(
    revision: u64,
    capture_sidecar_sha256: &str,
    previous_result_sha256: Option<&str>,
) -> Result<(), ResultRevisionError> {
    validate_sha256(
        capture_sidecar_sha256,
        ResultRevisionError::InvalidCaptureHash,
    )?;
    if revision != 1 {
        return Err(ResultRevisionError::NonMonotonicRevision);
    }
    if previous_result_sha256.is_some() {
        return Err(ResultRevisionError::UnexpectedPreviousResultHash);
    }
    Ok(())
}

fn validate_next_revision(
    previous_revision: u64,
    revision: u64,
    previous_capture_sidecar_sha256: &str,
    capture_sidecar_sha256: &str,
    previous_result_sha256: &str,
) -> Result<(), ResultRevisionError> {
    if revision
        != previous_revision
            .checked_add(1)
            .ok_or(ResultRevisionError::RevisionOverflow)?
    {
        return Err(ResultRevisionError::NonMonotonicRevision);
    }
    if capture_sidecar_sha256 != previous_capture_sidecar_sha256 {
        return Err(ResultRevisionError::CaptureHashChanged);
    }
    validate_sha256(
        capture_sidecar_sha256,
        ResultRevisionError::InvalidCaptureHash,
    )?;
    validate_sha256(
        previous_result_sha256,
        ResultRevisionError::InvalidPreviousResultHash,
    )
}

fn validate_named_attribution_authority(
    authority: ResultAuthority,
    speaker_turns: &[SpeakerTurn],
    aligned_words: &[AlignedWord],
) -> Result<(), ResultRevisionError> {
    let contains_named = speaker_turns.iter().any(SpeakerTurn::has_named_attribution)
        || aligned_words.iter().any(AlignedWord::has_named_attribution);
    if contains_named && authority != ResultAuthority::ServerAuthoritative {
        return Err(ResultRevisionError::NamedAttributionRequiresServerAuthority);
    }
    Ok(())
}

fn validate_wire_revision(
    revision: u64,
    capture_sidecar_sha256: &str,
    previous_result_sha256: Option<&str>,
) -> Result<(), ResultRevisionError> {
    validate_sha256(
        capture_sidecar_sha256,
        ResultRevisionError::InvalidCaptureHash,
    )?;
    match (revision, previous_result_sha256) {
        (1, None) => Ok(()),
        (1, Some(_)) => Err(ResultRevisionError::UnexpectedPreviousResultHash),
        (0, _) => Err(ResultRevisionError::NonMonotonicRevision),
        (_, Some(hash)) => validate_sha256(hash, ResultRevisionError::InvalidPreviousResultHash),
        (_, None) => Err(ResultRevisionError::MissingPreviousResultHash),
    }
}

fn validate_sha256(value: &str, error: ResultRevisionError) -> Result<(), ResultRevisionError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultRevisionError {
    InvalidCaptureHash,
    MissingPreviousResultHash,
    InvalidPreviousResultHash,
    UnexpectedPreviousResultHash,
    NonMonotonicRevision,
    RevisionOverflow,
    CaptureHashChanged,
    NamedAttributionRequiresServerAuthority,
}

impl std::fmt::Display for ResultRevisionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ResultRevisionError {}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptResultRevisionWire {
    session_id: SessionId,
    revision: u64,
    authority: ResultAuthority,
    capture_sidecar_sha256: String,
    previous_result_sha256: Option<String>,
    status: ResultStatus,
    transcript: String,
    aligned_words: Vec<ServerAlignedWord>,
    model_provenance: Vec<ModelRevision>,
}

impl<'de> serde::Deserialize<'de> for TranscriptResultRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = TranscriptResultRevisionWire::deserialize(deserializer)?;
        validate_wire_revision(
            wire.revision,
            &wire.capture_sidecar_sha256,
            wire.previous_result_sha256.as_deref(),
        )
        .map_err(serde::de::Error::custom)?;
        let aligned_words = wire
            .aligned_words
            .into_iter()
            .map(ServerAlignedWord::into_inner)
            .collect::<Vec<_>>();
        validate_named_attribution_authority(wire.authority, &[], &aligned_words)
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            session_id: wire.session_id,
            revision: wire.revision,
            authority: wire.authority,
            capture_sidecar_sha256: wire.capture_sidecar_sha256,
            previous_result_sha256: wire.previous_result_sha256,
            status: wire.status,
            transcript: wire.transcript,
            aligned_words,
            model_provenance: wire.model_provenance,
        })
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeakerResultRevisionWire {
    session_id: SessionId,
    revision: u64,
    authority: ResultAuthority,
    capture_sidecar_sha256: String,
    previous_result_sha256: Option<String>,
    status: ResultStatus,
    speaker_turns: Vec<ServerSpeakerTurn>,
    aligned_words: Vec<ServerAlignedWord>,
    model_provenance: Vec<ModelRevision>,
}

impl<'de> serde::Deserialize<'de> for SpeakerResultRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SpeakerResultRevisionWire::deserialize(deserializer)?;
        validate_wire_revision(
            wire.revision,
            &wire.capture_sidecar_sha256,
            wire.previous_result_sha256.as_deref(),
        )
        .map_err(serde::de::Error::custom)?;
        let speaker_turns = wire
            .speaker_turns
            .into_iter()
            .map(ServerSpeakerTurn::into_inner)
            .collect::<Vec<_>>();
        let aligned_words = wire
            .aligned_words
            .into_iter()
            .map(ServerAlignedWord::into_inner)
            .collect::<Vec<_>>();
        validate_named_attribution_authority(wire.authority, &speaker_turns, &aligned_words)
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            session_id: wire.session_id,
            revision: wire.revision,
            authority: wire.authority,
            capture_sidecar_sha256: wire.capture_sidecar_sha256,
            previous_result_sha256: wire.previous_result_sha256,
            status: wire.status,
            speaker_turns,
            aligned_words,
            model_provenance: wire.model_provenance,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::audio::{
        evidence::{ClientSpeakerAttribution, EvidenceQuality, ModelRevision, SpeakerTurn},
        session::SessionId,
    };

    use super::{ResultAuthority, ResultStatus, SpeakerResultRevision};

    fn hash(value: char) -> String {
        value.to_string().repeat(64)
    }

    fn turn() -> SpeakerTurn {
        SpeakerTurn::new(
            "turn-1",
            0,
            20,
            ClientSpeakerAttribution::unknown(),
            Some(0.8),
        )
        .unwrap()
    }

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
        assert!(serde_json::from_value::<super::TranscriptResultRevision>(transcript).is_err());
    }

    #[test]
    fn transcript_revisions_require_a_valid_predecessor_hash_and_next_revision() {
        let first = super::TranscriptResultRevision::new(
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
        let first = super::TranscriptResultRevision::new(
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
}
