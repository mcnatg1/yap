use std::path::Path;

use crate::audio::recording;

use super::super::evidence::{
    generic_delete_quarantine, physical_entry_exists, private_deletion_leftover_is_old,
    private_process_id,
};
use super::push_maintenance_warning;

#[derive(Clone, Copy)]
pub(super) enum PrivateDeletionLeftover {
    Staging { process_id: u32 },
    Quarantine { process_id: u32 },
}

pub(super) fn reconcile_private_deletion_leftovers<'a>(
    dir: &Path,
    names: impl IntoIterator<Item = &'a str>,
    warnings: &mut Vec<String>,
) {
    for name in names {
        match physical_entry_exists(dir, name) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(error) => {
                push_maintenance_warning(
                    warnings,
                    format!("Private recording deletion cleanup was retained: {name}: {error}"),
                );
                continue;
            }
        }
        match private_deletion_leftover(name) {
            Some(PrivateDeletionLeftover::Staging { process_id })
            | Some(PrivateDeletionLeftover::Quarantine { process_id }) => {
                let leftover = PrivateDeletionLeftover::Quarantine { process_id };
                match private_deletion_leftover_is_reconcilable(dir, name, leftover) {
                    Ok(true) => {
                        if let Err(error) = recording::remove_regular_artifact(dir, name) {
                            push_maintenance_warning(
                            warnings,
                            format!("Private recording deletion cleanup was retained: {name}: {error}"),
                        );
                        }
                    }
                    Ok(false) => {}
                    Err(error) => push_maintenance_warning(
                        warnings,
                        format!("Private recording deletion cleanup was retained: {name}: {error}"),
                    ),
                }
            }
            None if looks_like_private_deletion_artifact(name) => push_maintenance_warning(
                warnings,
                format!("Unknown private deletion artifact was retained: {name}"),
            ),
            None => {}
        }
    }
}

pub(super) fn private_deletion_leftover_is_reconcilable(
    dir: &Path,
    name: &str,
    leftover: PrivateDeletionLeftover,
) -> Result<bool, String> {
    let process_id = match leftover {
        PrivateDeletionLeftover::Staging { process_id }
        | PrivateDeletionLeftover::Quarantine { process_id } => process_id,
    };
    if process_id == std::process::id() {
        Ok(false)
    } else {
        private_deletion_leftover_is_old(dir, name)
    }
}

pub(super) fn private_deletion_leftover(name: &str) -> Option<PrivateDeletionLeftover> {
    if let Some((_, process_id)) = generic_delete_quarantine(name) {
        return Some(PrivateDeletionLeftover::Quarantine { process_id });
    }
    let stem = name.strip_prefix(".live-")?;
    if let Some((session, suffix)) = stem
        .strip_suffix(".part")
        .and_then(|value| value.split_once(".deletion.v1."))
    {
        crate::audio::session::SessionId::new(session.to_string()).ok()?;
        return Some(PrivateDeletionLeftover::Staging {
            process_id: private_process_id(suffix)?,
        });
    }
    None
}

pub(super) fn looks_like_private_deletion_artifact(name: &str) -> bool {
    name.starts_with(".live-") && (name.contains(".deletion.v1.") || name.contains(".delete-"))
}

pub(super) fn deletion_intent_session(name: &str) -> Option<crate::audio::session::SessionId> {
    name.strip_prefix("live-")
        .and_then(|session| session.strip_suffix(".deletion.v1.json"))
        .and_then(|session| crate::audio::session::SessionId::new(session.to_string()).ok())
}
