use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

use time::OffsetDateTime;

use crate::audio::recording;

#[cfg(test)]
use super::super::mutation_ownership::session_mutation_ownership;
use super::super::retention::committed_meeting_is_expired;
use super::super::transcripts::{
    has_valid_transcript_revision, highest_transcript_revision, transcript_artifact_names,
};
use super::evidence::{physical_entry_exists, reconcile_intent_evidence_quarantines_while_owned};

pub(in crate::live::recordings) const DELETION_INTENT_SCHEMA_VERSION: u16 = 1;
const MAX_DELETION_ARTIFACTS: usize = 128;

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

#[cfg(test)]
pub(in crate::live::recordings) fn write_deletion_intent(
    path: &Path,
    intent: &DeletionIntent,
) -> Result<(), String> {
    write_deletion_intent_with_publication_barrier(path, intent, |_| {})
}

#[cfg(test)]
pub(in crate::live::recordings) fn write_deletion_intent_with_publication_barrier<F>(
    path: &Path,
    intent: &DeletionIntent,
    publication_barrier: F,
) -> Result<(), String>
where
    F: FnMut(bool),
{
    let _ownership = session_mutation_ownership();
    write_deletion_intent_with_publication_barrier_while_owned(path, intent, publication_barrier)
}

pub(in crate::live::recordings) fn write_deletion_intent_with_publication_barrier_while_owned<F>(
    path: &Path,
    intent: &DeletionIntent,
    mut publication_barrier: F,
) -> Result<(), String>
where
    F: FnMut(bool),
{
    validate_deletion_intent(intent)?;
    let dir = path
        .parent()
        .ok_or_else(|| "recording deletion intent has no parent directory".to_string())?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "recording deletion intent has no valid file name".to_string())?;
    if name != deletion_intent_name(&intent.session_id) {
        return Err("recording deletion intent name does not match its session".into());
    }
    reconcile_intent_evidence_quarantines_while_owned(dir, name)?;
    let replace_corrupt_final = if physical_entry_exists(dir, name)? {
        let existing = recording::read_regular_artifact(dir, name)
            .ok()
            .and_then(|text| {
                serde_json::from_str::<DeletionIntent>(&text)
                    .ok()
                    .filter(|existing| validate_deletion_intent(existing).is_ok())
            });
        if existing.as_ref() == Some(intent) {
            return Ok(());
        }
        if !intent_originals_are_intact(dir, intent)? {
            return Err("recording deletion intent is corrupt and deletion may have started; evidence was retained".into());
        }
        true
    } else {
        false
    };

    let (staging, mut file) = create_unique_deletion_intent_staging(dir, &intent.session_id)?;
    let mut quarantined = None;
    let result = (|| {
        serde_json::to_writer(&mut file, intent)
            .map_err(|error| format!("Failed to serialize recording deletion intent: {error}"))?;
        file.write_all(b"\n")
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("Failed to persist recording deletion intent: {error}"))?;
        if replace_corrupt_final {
            quarantined = Some(recording::quarantine_regular_artifact(dir, name)?);
        }
        publication_barrier(false);
        let published = recording::publish_no_replace(
            &staging,
            path,
            &file,
            "publish recording deletion intent",
        )?;
        drop(published);
        publication_barrier(true);
        let published = recording::read_regular_artifact(dir, name)?;
        let published: DeletionIntent = serde_json::from_str(&published).map_err(|error| {
            format!("Failed to re-read published recording deletion intent: {error}")
        })?;
        if published != *intent {
            return Err("published recording deletion intent changed before verification".into());
        }
        if let Some(quarantined) = quarantined.as_ref() {
            recording::remove_verified_quarantined_artifact(quarantined)?;
        }
        let _ = recording::sync_recordings_parent(dir);
        Ok(())
    })();
    if let Err(error) = &result {
        if let Some(quarantined) = quarantined.as_ref() {
            if let Err(restore_error) =
                recording::restore_verified_quarantined_artifact(quarantined, path)
            {
                crate::stt::log_yap(&format!(
                    "Retained quarantined recording deletion intent after publication failure: {restore_error}"
                ));
            }
        }
        recording::remove_owned_staging(&staging, &file, "publish recording deletion intent");
        crate::stt::log_yap(&format!(
            "Failed to publish recording deletion intent: {error}"
        ));
    }
    drop(file);
    result
}

fn create_unique_deletion_intent_staging(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<(std::path::PathBuf, std::fs::File), String> {
    for nonce in 0..128_u64 {
        let path = dir.join(format!(
            ".live-{session_id}.deletion.v1.{}-{nonce}.part",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Failed to create recording deletion intent staging file: {error}"
                ))
            }
        }
    }
    Err("Failed to allocate a private recording deletion intent staging file".into())
}

pub(super) fn revalidate_intent_artifact(
    dir: &Path,
    name: &str,
    sha256: &str,
    identity: Option<&recording::FileIdentity>,
) -> Result<(), String> {
    let identity = identity.ok_or_else(|| {
        "recording deletion intent lacks durable identity evidence; manual reconciliation is required"
            .to_string()
    })?;
    recording::revalidate_regular_artifact_file_identity_and_hash(dir, name, identity, sha256)
}

pub(super) fn prove_intent_against_current_commit(
    dir: &Path,
    intent: &DeletionIntent,
    now: OffsetDateTime,
) -> Result<(), String> {
    let (commit_text, commit_sha256) =
        recording::read_and_hash_regular_artifact(dir, &intent.commit_file)?;
    if commit_sha256 != intent.commit_sha256 {
        return Err("recording deletion commit no longer matches the published intent".into());
    }
    let manifest: recording::CaptureCommitManifest = serde_json::from_str(&commit_text)
        .map_err(|error| format!("Failed to parse committed recording manifest: {error}"))?;
    manifest.validate()?;
    if manifest.session_id != intent.session_id
        || !intent_has_artifact(intent, &manifest.audio_file, &manifest.audio_sha256)
        || !intent_has_artifact(
            intent,
            &manifest.capture_sidecar_file,
            &manifest.capture_sidecar_sha256,
        )
    {
        return Err(
            "recording deletion intent does not match the current committed session".into(),
        );
    }
    if intent.reason == "expired-meeting-retention" && !committed_meeting_is_expired(&manifest, now)
    {
        return Err(
            "expired-meeting deletion is no longer authorized by the committed metadata".into(),
        );
    }
    Ok(())
}

fn intent_has_artifact(intent: &DeletionIntent, name: &str, sha256: &str) -> bool {
    intent
        .artifacts
        .iter()
        .any(|artifact| artifact.name == name && artifact.sha256 == sha256)
}

pub(super) fn intent_originals_are_intact(
    dir: &Path,
    intent: &DeletionIntent,
) -> Result<bool, String> {
    if !physical_entry_exists(dir, &intent.commit_file)? {
        return Ok(false);
    }
    if intent.reason != "recoverable"
        && prove_intent_against_current_commit(dir, intent, OffsetDateTime::now_utc()).is_err()
    {
        return Ok(false);
    }
    if revalidate_intent_artifact(
        dir,
        &intent.commit_file,
        &intent.commit_sha256,
        intent.commit_file_identity.as_ref(),
    )
    .is_err()
    {
        return Ok(false);
    }
    for artifact in &intent.artifacts {
        if !physical_entry_exists(dir, &artifact.name)?
            || revalidate_intent_artifact(
                dir,
                &artifact.name,
                &artifact.sha256,
                artifact.file_identity.as_ref(),
            )
            .is_err()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(in crate::live::recordings) fn validate_deletion_intent(
    intent: &DeletionIntent,
) -> Result<(), String> {
    let recoverable = intent.reason == "recoverable";
    if intent.schema_version != DELETION_INTENT_SCHEMA_VERSION
        || (!recoverable
            && intent.reason != "manual"
            && intent.reason != "expired-meeting-retention")
        || (!recoverable && intent.commit_file != format!("live-{}.commit.json", intent.session_id))
        || (recoverable
            && !is_deletion_artifact_for_session(&intent.commit_file, &intent.session_id))
        || (!recoverable && intent.artifacts.is_empty())
        || intent.artifacts.len() > MAX_DELETION_ARTIFACTS
        || !is_sha256(&intent.commit_sha256)
    {
        return Err("recording deletion intent has an unsupported shape".into());
    }
    let mut names = HashSet::new();
    for artifact in &intent.artifacts {
        recording::validate_artifact_name(&artifact.name)?;
        if !names.insert(artifact.name.clone())
            || artifact.name == intent.commit_file
            || !is_deletion_artifact_for_session(&artifact.name, &intent.session_id)
        {
            return Err("recording deletion intent names an unowned artifact".into());
        }
        if !is_sha256(&artifact.sha256) {
            return Err("recording deletion intent has an invalid artifact hash".into());
        }
    }
    if intent.commit_file_identity.is_none()
        || intent
            .artifacts
            .iter()
            .any(|artifact| artifact.file_identity.is_none())
    {
        return Err(
            "recording deletion intent lacks complete durable identity evidence; manual reconciliation is required"
                .into(),
        );
    }
    if !recoverable
        && (!names.contains(&format!("live-{}.wav", intent.session_id))
            || !names.contains(&format!("live-{}.capture.json", intent.session_id)))
    {
        return Err("recording deletion intent is missing required capture artifacts".into());
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_deletion_artifact_for_session(
    name: &str,
    session_id: &crate::audio::session::SessionId,
) -> bool {
    let stem = format!("live-{session_id}");
    name == format!("{stem}.wav")
        || name == format!("{stem}.wav.part")
        || name == format!("{stem}.capture.json")
        || name == format!("{stem}.capture.json.part")
        || name == format!("{stem}.capture.journal.part")
        || name == format!("{stem}.capture.partial.json")
        || name == format!("{stem}.capture.partial.json.part")
        || name == format!("{stem}.commit.json")
        || name == format!("{stem}.commit.json.part")
        || name == format!("{stem}.txt")
        || name == format!("{stem}.polished.txt")
        || name
            .strip_prefix(&format!("{stem}.transcript.r"))
            .and_then(|value| value.strip_suffix(".json"))
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|revision| revision > 0)
}
