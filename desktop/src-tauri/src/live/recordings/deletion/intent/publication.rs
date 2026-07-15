use std::{io::Write, path::Path};

use crate::audio::recording;

#[cfg(test)]
use super::super::super::mutation_ownership::session_mutation_ownership;
use super::super::evidence::{
    physical_entry_exists, reconcile_intent_evidence_quarantines_while_owned,
};
use super::{
    model::{deletion_intent_name, DeletionIntent},
    validation::{intent_originals_are_intact, validate_deletion_intent},
};

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
