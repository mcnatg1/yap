use std::collections::BTreeSet;

use super::super::deletion::{
    admit_deletion_artifact, admit_expected_private_artifact_identity, deletion_intent_name,
    resume_deletion_intent_while_owned, validate_deletion_intent,
    write_deletion_intent_with_publication_barrier_while_owned, DeletionIntent,
    DELETION_INTENT_SCHEMA_VERSION,
};
use super::super::*;
use super::{
    catalog::{
        recoverable_session_artifact_path, recoverable_session_from_dir, regular_artifact_exists,
    },
    repair::{saved_recovered_session, valid_partial_sidecar},
};

pub(in crate::live::recordings) fn delete_recoverable_live_session_in_dir(
    dir: &Path,
    session_id: String,
    expected_artifact_path: String,
) -> Result<(), String> {
    delete_recoverable_live_session_in_dir_with_mutation_barrier(
        dir,
        session_id,
        expected_artifact_path,
        || {},
    )
}

pub(in crate::live::recordings) fn delete_recoverable_live_session_in_dir_with_mutation_barrier<F>(
    dir: &Path,
    session_id: String,
    expected_artifact_path: String,
    mutation_barrier: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    let _ownership = session_mutation_ownership();
    let session_id = crate::audio::session::SessionId::new(session_id)?;
    ensure_recoverable_session(dir, &session_id)?;
    let candidate = recoverable_session_from_dir(dir, &session_id)?;
    let artifact_path = recoverable_session_artifact_path(&candidate).ok_or_else(|| {
        "Recoverable live recording has no authoritative artifact identity.".to_string()
    })?;
    let expected = admit_expected_private_artifact_identity(
        Path::new(artifact_path),
        &expected_artifact_path,
    )?;
    delete_recoverable_session_artifacts_with_barrier_while_owned(
        dir,
        &session_id,
        Some(&expected),
        mutation_barrier,
    )
}

pub(in crate::live::recordings) fn delete_recoverable_session_artifacts_while_owned(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
    expected: Option<&recording::RegularArtifactIdentity>,
) -> Result<(), String> {
    delete_recoverable_session_artifacts_with_barrier_while_owned(dir, session_id, expected, || {})
}

pub(in crate::live::recordings) fn delete_recoverable_session_artifacts_with_barrier_while_owned<
    F,
>(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
    expected: Option<&recording::RegularArtifactIdentity>,
    mutation_barrier: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    ensure_recoverable_session(dir, session_id)?;
    let intent = build_recoverable_deletion_intent(dir, session_id, expected)?;
    let intent_name = deletion_intent_name(session_id);
    write_deletion_intent_with_publication_barrier_while_owned(
        &dir.join(&intent_name),
        &intent,
        |_| {},
    )?;
    mutation_barrier();
    resume_deletion_intent_while_owned(dir, &intent_name)
}

pub(in crate::live::recordings) fn build_recoverable_deletion_intent(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
    expected: Option<&recording::RegularArtifactIdentity>,
) -> Result<DeletionIntent, String> {
    let mut names = BTreeSet::new();
    for suffix in [
        ".wav.part",
        ".capture.journal.part",
        ".capture.json.part",
        ".capture.partial.json.part",
        ".commit.json.part",
        ".capture.partial.json",
        ".commit.json",
        ".wav",
        ".txt",
        ".polished.txt",
    ] {
        let name = format!("live-{session_id}{suffix}");
        if regular_artifact_exists(dir, &name) {
            names.insert(name);
        }
    }
    names.extend(transcript_artifact_names(dir, session_id)?);

    let sidecar_name = format!("live-{session_id}.capture.partial.json");
    if names.contains(&sidecar_name)
        && recording::read_regular_artifact(dir, &sidecar_name)
            .is_ok_and(|text| !valid_partial_sidecar(&text, session_id))
    {
        names.remove(&sidecar_name);
    }
    let commit_name = format!("live-{session_id}.commit.json");
    if names.contains(&commit_name) && saved_recovered_session(dir, session_id)?.is_none() {
        names.remove(&commit_name);
    }

    let mut artifacts = names
        .into_iter()
        .map(|name| admit_deletion_artifact(dir, &name))
        .collect::<Result<Vec<_>, _>>()?;
    let anchor_index = if let Some(expected) = expected {
        artifacts
            .iter()
            .position(|artifact| {
                expected.matches_artifact_name(&artifact.name)
                    && artifact.file_identity == Some(expected.file_identity())
            })
            .ok_or_else(|| {
                "admitted recovery artifact is absent from the deletion snapshot".to_string()
            })?
    } else {
        (!artifacts.is_empty())
            .then_some(0)
            .ok_or_else(|| "Recoverable live recording has no deletable artifacts.".to_string())?
    };
    let anchor = artifacts.remove(anchor_index);
    let intent = DeletionIntent {
        schema_version: DELETION_INTENT_SCHEMA_VERSION,
        session_id: session_id.clone(),
        reason: "recoverable".into(),
        commit_file: anchor.name,
        commit_sha256: anchor.sha256,
        commit_file_identity: anchor.file_identity,
        artifacts,
    };
    validate_deletion_intent(&intent)?;
    Ok(intent)
}

pub(in crate::live::recordings) fn ensure_recoverable_session(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<(), String> {
    let scan = recording::scan_recordings(dir)?;
    (scan
        .partial
        .into_iter()
        .any(|partial| partial.session_id.as_ref() == Some(session_id))
        || scan
            .recovered_partial
            .into_iter()
            .any(|recovered| recovered.session_id == *session_id))
    .then_some(())
    .ok_or_else(|| "Live recording is not a recoverable Yap session.".into())
}
