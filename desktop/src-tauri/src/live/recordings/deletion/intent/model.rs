use std::{collections::HashSet, path::Path};

use crate::audio::recording;

use super::super::super::transcripts::{
    has_valid_transcript_revision, highest_transcript_revision, transcript_artifact_names,
};
use super::validation::validate_deletion_intent;

pub(in crate::live::recordings) const DELETION_INTENT_SCHEMA_VERSION: u16 = 1;
pub(super) const MAX_DELETION_ARTIFACTS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::live::recordings) struct DeletionArtifact {
    pub(in crate::live::recordings) name: String,
    pub(in crate::live::recordings) sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::live::recordings) file_identity: Option<recording::FileIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::live::recordings) struct DeletionIntent {
    pub(in crate::live::recordings) schema_version: u16,
    pub(in crate::live::recordings) session_id: crate::audio::session::SessionId,
    pub(in crate::live::recordings) reason: String,
    pub(in crate::live::recordings) commit_file: String,
    pub(in crate::live::recordings) commit_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::live::recordings) commit_file_identity: Option<recording::FileIdentity>,
    pub(in crate::live::recordings) artifacts: Vec<DeletionArtifact>,
}

pub(in crate::live::recordings) fn build_deletion_intent(
    dir: &Path,
    capture: &recording::CommittedCapture,
    reason: &str,
) -> Result<DeletionIntent, String> {
    if reason != "manual" && reason != "expired-meeting-retention" {
        return Err("unsupported recording deletion reason".into());
    }
    let manifest = &capture.manifest;
    let commit_file = format!("live-{}.commit.json", manifest.session_id);
    let commit_admission = recording::admit_regular_artifact(&dir.join(&commit_file))?;
    let (commit_text, commit_sha256) = commit_admission.read_and_hash()?;
    let current_manifest: recording::CaptureCommitManifest = serde_json::from_str(&commit_text)
        .map_err(|error| format!("Failed to parse committed recording manifest: {error}"))?;
    if current_manifest != *manifest {
        return Err("committed recording manifest changed before deletion".into());
    }
    let audio = admit_deletion_artifact(dir, &manifest.audio_file)?;
    let sidecar = admit_deletion_artifact(dir, &manifest.capture_sidecar_file)?;
    if audio.sha256 != manifest.audio_sha256 || sidecar.sha256 != manifest.capture_sidecar_sha256 {
        return Err("committed recording artifacts changed before deletion".into());
    }
    let mut artifacts = vec![audio, sidecar];
    let journal = format!("live-{}.capture.journal.part", manifest.session_id);
    if recording::is_regular_artifact(&dir.join(&journal)) {
        let journal_admission = recording::admit_regular_artifact(&dir.join(&journal))?;
        let (journal_text, journal_sha256) = journal_admission.read_and_hash()?;
        let parsed = recording::parse_journal_for_session(&journal_text, &manifest.session_id)?;
        if parsed {
            artifacts.push(DeletionArtifact {
                name: journal,
                sha256: journal_sha256,
                file_identity: Some(journal_admission.file_identity()),
            });
        }
    }
    let transcript_names = transcript_artifact_names(dir, &manifest.session_id)?;
    if !transcript_names.is_empty() {
        let highest = highest_transcript_revision(dir, &manifest.session_id).ok_or_else(|| {
            "transcript artifacts do not contain a numbered immutable revision".to_string()
        })?;
        let expected = std::iter::once(format!("live-{}.txt", manifest.session_id))
            .chain((1..=highest).map(|revision| {
                format!("live-{}.transcript.r{revision}.json", manifest.session_id)
            }))
            .collect::<HashSet<_>>();
        if transcript_names != expected
            || !has_valid_transcript_revision(
                dir,
                &manifest.session_id,
                &manifest.capture_sidecar_sha256,
            )
        {
            return Err("transcript artifacts are incomplete or do not form a valid immutable revision chain".into());
        }
        for name in transcript_names {
            artifacts.push(admit_deletion_artifact(dir, &name)?);
        }
        let polished = format!("live-{}.polished.txt", manifest.session_id);
        if recording::is_regular_artifact(&dir.join(&polished)) {
            artifacts.push(admit_deletion_artifact(dir, &polished)?);
        }
    }
    let intent = DeletionIntent {
        schema_version: DELETION_INTENT_SCHEMA_VERSION,
        session_id: manifest.session_id.clone(),
        reason: reason.to_string(),
        commit_file,
        commit_sha256,
        commit_file_identity: Some(commit_admission.file_identity()),
        artifacts,
    };
    validate_deletion_intent(&intent)?;
    Ok(intent)
}

pub(in crate::live::recordings) fn admit_deletion_artifact(
    dir: &Path,
    name: &str,
) -> Result<DeletionArtifact, String> {
    let admission = recording::admit_regular_artifact(&dir.join(name))?;
    Ok(DeletionArtifact {
        name: name.to_string(),
        sha256: admission.sha256()?,
        file_identity: Some(admission.file_identity()),
    })
}

pub(in crate::live::recordings) fn deletion_intent_name(
    session_id: &crate::audio::session::SessionId,
) -> String {
    format!("live-{session_id}.deletion.v1.json")
}
