use std::path::Path;

use time::OffsetDateTime;

use crate::audio::recording;

use super::super::artifacts::committed_session_output_path;
use super::super::mutation_ownership::session_mutation_ownership;
use super::evidence::physical_entry_exists;
use super::intent::{
    build_deletion_intent, deletion_intent_name, prove_intent_against_current_commit,
    revalidate_intent_artifact, validate_deletion_intent,
    write_deletion_intent_with_publication_barrier_while_owned, DeletionArtifact, DeletionIntent,
};

fn admit_expected_artifact_identity(
    actual_path: &Path,
    expected_path: &str,
) -> Result<recording::RegularArtifactIdentity, String> {
    recording::admit_expected_regular_artifact(actual_path, Path::new(expected_path))
}

pub(in crate::live::recordings) fn admit_expected_private_artifact_identity(
    actual_path: &Path,
    expected_path: &str,
) -> Result<recording::RegularArtifactIdentity, String> {
    recording::admit_expected_private_regular_artifact(actual_path, Path::new(expected_path))
}

pub(in crate::live::recordings) struct ExpectedDeletionArtifacts<'a> {
    output: &'a recording::RegularArtifactIdentity,
    commit: &'a recording::RegularArtifactIdentity,
}

pub(in crate::live::recordings) fn delete_saved_live_session_in_dir(
    dir: &Path,
    session_id: String,
    expected_output_path: String,
    expected_capture_commit_path: String,
) -> Result<(), String> {
    let _ownership = session_mutation_ownership();
    let session_id = crate::audio::session::SessionId::new(session_id)?;
    let scan = recording::scan_recordings(dir)?;
    let capture = scan
        .complete
        .iter()
        .find(|capture| capture.manifest.session_id == session_id)
        .ok_or_else(|| "Live recording is not a hash-valid committed Yap session.".to_string())?;
    let expected_output = admit_expected_artifact_identity(
        &committed_session_output_path(dir, capture),
        &expected_output_path,
    )?;
    let expected_commit = admit_expected_artifact_identity(
        &dir.join(format!("live-{session_id}.commit.json")),
        &expected_capture_commit_path,
    )?;
    let expected = ExpectedDeletionArtifacts {
        output: &expected_output,
        commit: &expected_commit,
    };
    delete_committed_session_in_dir_with_publication_barrier_while_owned(
        dir,
        capture,
        "manual",
        |_| {},
        Some(&expected),
    )
}

#[cfg(test)]
pub(in crate::live::recordings) fn delete_committed_session_in_dir_with_publication_barrier<F>(
    dir: &Path,
    capture: &recording::CommittedCapture,
    reason: &str,
    publication_barrier: F,
) -> Result<(), String>
where
    F: FnMut(bool),
{
    let _ownership = session_mutation_ownership();
    delete_committed_session_in_dir_with_publication_barrier_while_owned(
        dir,
        capture,
        reason,
        publication_barrier,
        None,
    )
}

pub(in crate::live::recordings) fn delete_committed_session_in_dir_with_publication_barrier_while_owned<
    F,
>(
    dir: &Path,
    capture: &recording::CommittedCapture,
    reason: &str,
    publication_barrier: F,
    expected: Option<&ExpectedDeletionArtifacts<'_>>,
) -> Result<(), String>
where
    F: FnMut(bool),
{
    let intent = build_deletion_intent(dir, capture, reason)?;
    let intent_name = deletion_intent_name(&intent.session_id);
    let intent_path = dir.join(&intent_name);
    write_deletion_intent_with_publication_barrier_while_owned(
        &intent_path,
        &intent,
        publication_barrier,
    )?;
    resume_deletion_intent_while_owned_with_expected(dir, &intent_name, expected)
}

#[cfg(test)]
pub(in crate::live::recordings) fn resume_deletion_intent(
    dir: &Path,
    intent_name: &str,
) -> Result<(), String> {
    let _ownership = session_mutation_ownership();
    resume_deletion_intent_while_owned(dir, intent_name)
}

pub(in crate::live::recordings) fn resume_deletion_intent_while_owned(
    dir: &Path,
    intent_name: &str,
) -> Result<(), String> {
    resume_deletion_intent_while_owned_with_expected(dir, intent_name, None)
}

fn resume_deletion_intent_while_owned_with_expected(
    dir: &Path,
    intent_name: &str,
    expected: Option<&ExpectedDeletionArtifacts<'_>>,
) -> Result<(), String> {
    let intent_admission = recording::admit_regular_artifact(&dir.join(intent_name))?;
    let (text, intent_sha256) = intent_admission.read_and_hash()?;
    let intent: DeletionIntent = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse recording deletion intent: {error}"))?;
    validate_deletion_intent(&intent)?;
    if intent_name != deletion_intent_name(&intent.session_id) {
        return Err("recording deletion intent name does not match its session".into());
    }
    preflight_deletion_intent_artifacts(dir, &intent)?;
    if physical_entry_exists(dir, &intent.commit_file)? {
        if intent.reason != "recoverable" {
            prove_intent_against_current_commit(dir, &intent, OffsetDateTime::now_utc())?;
            if let Some(expected) = expected {
                recording::revalidate_regular_artifact_identity(expected.commit)?;
                intent
                    .artifacts
                    .iter()
                    .find(|artifact| expected.output.matches_artifact_name(&artifact.name))
                    .ok_or_else(|| {
                        "admitted output artifact is absent from the deletion intent".to_string()
                    })?;
                recording::revalidate_regular_artifact_identity(expected.output)?;
            }
        }
        for artifact in &intent.artifacts {
            remove_intent_artifact_if_present(dir, artifact)?;
        }
        remove_intent_commit(dir, &intent)?;
    } else {
        ensure_intent_artifacts_are_absent(dir, &intent)?;
    }
    recording::remove_regular_artifact_if_identity_and_hash(
        dir,
        intent_name,
        &intent_admission,
        &intent_sha256,
    )
}

fn preflight_deletion_intent_artifacts(dir: &Path, intent: &DeletionIntent) -> Result<(), String> {
    if physical_entry_exists(dir, &intent.commit_file)? {
        revalidate_intent_artifact(
            dir,
            &intent.commit_file,
            &intent.commit_sha256,
            intent.commit_file_identity.as_ref(),
        )?;
    }
    for artifact in &intent.artifacts {
        if physical_entry_exists(dir, &artifact.name)? {
            revalidate_intent_artifact(
                dir,
                &artifact.name,
                &artifact.sha256,
                artifact.file_identity.as_ref(),
            )?;
        }
    }
    Ok(())
}

fn remove_intent_artifact_if_present(
    dir: &Path,
    artifact: &DeletionArtifact,
) -> Result<(), String> {
    if !physical_entry_exists(dir, &artifact.name)? {
        return Ok(());
    }
    let identity = artifact.file_identity.as_ref().ok_or_else(|| {
        "recording deletion intent lacks durable identity evidence; manual reconciliation is required"
            .to_string()
    })?;
    recording::remove_regular_artifact_if_file_identity_and_hash(
        dir,
        &artifact.name,
        identity,
        &artifact.sha256,
    )
}

fn remove_intent_commit(dir: &Path, intent: &DeletionIntent) -> Result<(), String> {
    let identity = intent.commit_file_identity.as_ref().ok_or_else(|| {
        "recording deletion intent lacks durable identity evidence; manual reconciliation is required"
            .to_string()
    })?;
    recording::remove_regular_artifact_if_file_identity_and_hash(
        dir,
        &intent.commit_file,
        identity,
        &intent.commit_sha256,
    )
}

fn ensure_intent_artifacts_are_absent(dir: &Path, intent: &DeletionIntent) -> Result<(), String> {
    for artifact in &intent.artifacts {
        if physical_entry_exists(dir, &artifact.name)? {
            return Err(
                "recording deletion commit is missing but an intended artifact remains".into(),
            );
        }
    }
    Ok(())
}
