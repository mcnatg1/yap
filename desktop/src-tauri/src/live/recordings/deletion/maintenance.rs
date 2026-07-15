use crate::audio::recording;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[cfg(test)]
use super::super::mutation_ownership::session_mutation_ownership;
use super::evidence::{
    generic_delete_quarantine, physical_entry_exists, private_deletion_leftover_is_old,
    private_process_id, reconcile_intent_evidence_quarantines_while_owned,
};
use super::execution::resume_deletion_intent_while_owned;

pub(in crate::live::recordings) const MAX_MAINTENANCE_WARNINGS: usize = 8;
pub(in crate::live::recordings) const MAX_PRIVATE_DELETION_LEFTOVERS: usize = 128;

static DELETION_CLEANUP_CURSORS: OnceLock<Mutex<HashMap<PathBuf, DeletionCleanupCursors>>> =
    OnceLock::new();

const MAX_DELETION_CLEANUP_CURSOR_DIRS: usize = 64;

#[derive(Default)]
struct DeletionCleanupCursors {
    private_leftovers: Option<String>,
    pending_intents: Option<String>,
}

#[derive(Clone, Copy)]
enum DeletionCleanupCursor {
    PrivateLeftovers,
    PendingIntents,
}

fn deletion_cleanup_cursor(dir: &Path, kind: DeletionCleanupCursor) -> Option<String> {
    let cursors = DELETION_CLEANUP_CURSORS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cursors = cursors.get(dir)?;
    match kind {
        DeletionCleanupCursor::PrivateLeftovers => cursors.private_leftovers.clone(),
        DeletionCleanupCursor::PendingIntents => cursors.pending_intents.clone(),
    }
}

fn update_deletion_cleanup_cursor(dir: &Path, kind: DeletionCleanupCursor, cursor: Option<String>) {
    let Some(cursor) = cursor else {
        return;
    };
    let mut cursors = DELETION_CLEANUP_CURSORS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cursors.len() >= MAX_DELETION_CLEANUP_CURSOR_DIRS && !cursors.contains_key(dir) {
        cursors.clear();
    }
    let cursors = cursors.entry(dir.to_path_buf()).or_default();
    match kind {
        DeletionCleanupCursor::PrivateLeftovers => cursors.private_leftovers = Some(cursor),
        DeletionCleanupCursor::PendingIntents => cursors.pending_intents = Some(cursor),
    }
}

pub(in crate::live::recordings) struct RotatingDeletionCandidates {
    cursor: Option<String>,
    after: BTreeSet<String>,
    before: BTreeSet<String>,
    overflow: bool,
    limit: usize,
}

impl RotatingDeletionCandidates {
    pub(in crate::live::recordings) fn new(cursor: Option<String>, limit: usize) -> Self {
        Self {
            cursor,
            after: BTreeSet::new(),
            before: BTreeSet::new(),
            overflow: false,
            limit,
        }
    }

    pub(in crate::live::recordings) fn push(&mut self, name: String) {
        let before_cursor = self.cursor.as_ref().is_some_and(|cursor| name <= *cursor);
        let target = if before_cursor {
            &mut self.before
        } else {
            &mut self.after
        };
        self.overflow |= push_bounded_candidate(target, name, self.limit);
    }

    pub(in crate::live::recordings) fn finish(self) -> (BTreeSet<String>, bool, Option<String>) {
        let mut selected = self.after;
        let mut wrapped_last = None;
        let remaining = self.limit.saturating_sub(selected.len());
        for name in self.before.into_iter().take(remaining) {
            wrapped_last = Some(name.clone());
            selected.insert(name);
        }
        let next_cursor = wrapped_last.or_else(|| selected.last().cloned());
        (selected, self.overflow, next_cursor)
    }
}

pub(in crate::live::recordings) struct ReconciliationWarnings {
    pub(in crate::live::recordings) session_warnings: HashMap<String, String>,
    pub(in crate::live::recordings) maintenance_warnings: Vec<String>,
}

#[cfg(test)]
pub(in crate::live::recordings) fn reconcile_pending_deletion_intents(
    dir: &Path,
) -> ReconciliationWarnings {
    let _ownership = session_mutation_ownership();
    reconcile_pending_deletion_intents_while_owned(dir)
}

pub(in crate::live::recordings) fn reconcile_pending_deletion_intents_while_owned(
    dir: &Path,
) -> ReconciliationWarnings {
    let mut warnings = ReconciliationWarnings {
        session_warnings: HashMap::new(),
        maintenance_warnings: Vec::new(),
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return warnings;
    };
    let mut private_candidates = RotatingDeletionCandidates::new(
        deletion_cleanup_cursor(dir, DeletionCleanupCursor::PrivateLeftovers),
        MAX_PRIVATE_DELETION_LEFTOVERS,
    );
    let mut intent_candidates = RotatingDeletionCandidates::new(
        deletion_cleanup_cursor(dir, DeletionCleanupCursor::PendingIntents),
        MAX_PRIVATE_DELETION_LEFTOVERS,
    );
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if let Some(leftover) = private_deletion_leftover(&name) {
            if let Some((artifact_name, process_id)) = generic_delete_quarantine(&name) {
                if process_id == std::process::id()
                    && deletion_intent_session(artifact_name).is_some()
                {
                    intent_candidates.push(artifact_name.to_string());
                }
            }
            match private_deletion_leftover_is_reconcilable(dir, &name, leftover) {
                Ok(true) => {
                    if let Some((artifact_name, _)) = generic_delete_quarantine(&name) {
                        if deletion_intent_session(artifact_name).is_some() {
                            intent_candidates.push(artifact_name.to_string());
                        }
                    }
                    private_candidates.push(name.clone());
                }
                Ok(false) => {}
                Err(error) => push_maintenance_warning(
                    &mut warnings.maintenance_warnings,
                    format!("Private recording deletion cleanup was retained: {name}: {error}"),
                ),
            }
        } else if looks_like_private_deletion_artifact(&name) {
            push_maintenance_warning(
                &mut warnings.maintenance_warnings,
                format!("Unknown private deletion artifact was retained: {name}"),
            );
        }
        if deletion_intent_session(&name).is_some() {
            intent_candidates.push(name);
        }
    }
    let (private_candidates, private_candidate_overflow, next_private_cursor) =
        private_candidates.finish();
    let (intent_names, intent_candidate_overflow, next_intent_cursor) = intent_candidates.finish();
    update_deletion_cleanup_cursor(
        dir,
        DeletionCleanupCursor::PrivateLeftovers,
        next_private_cursor,
    );
    update_deletion_cleanup_cursor(
        dir,
        DeletionCleanupCursor::PendingIntents,
        next_intent_cursor,
    );
    if private_candidate_overflow {
        push_maintenance_warning(
            &mut warnings.maintenance_warnings,
            "Private recording deletion cleanup scan reached its fixed budget.".into(),
        );
    }
    if intent_candidate_overflow {
        push_maintenance_warning(
            &mut warnings.maintenance_warnings,
            "Recording deletion intent scan reached its fixed budget.".into(),
        );
    }
    for name in &intent_names {
        if let Err(error) = reconcile_intent_evidence_quarantines_while_owned(dir, name) {
            push_maintenance_warning(
                &mut warnings.maintenance_warnings,
                format!("Recording cleanup evidence was retained: {name}: {error}"),
            );
        }
    }
    reconcile_private_deletion_leftovers(
        dir,
        private_candidates.iter().map(String::as_str),
        &mut warnings.maintenance_warnings,
    );
    for name in intent_names {
        if let Err(error) = resume_deletion_intent_while_owned(dir, &name) {
            if let Some(session) = deletion_intent_session(&name) {
                let warning = format!("Recording deletion is pending: {error}");
                warnings
                    .session_warnings
                    .insert(session.to_string(), warning.clone());
                push_maintenance_warning(&mut warnings.maintenance_warnings, warning);
            } else {
                push_maintenance_warning(
                    &mut warnings.maintenance_warnings,
                    format!("Recording cleanup evidence was retained: {name}: {error}"),
                );
            }
        }
    }
    warnings
}

#[derive(Clone, Copy)]
enum PrivateDeletionLeftover {
    Staging { process_id: u32 },
    Quarantine { process_id: u32 },
}

fn reconcile_private_deletion_leftovers<'a>(
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

fn private_deletion_leftover_is_reconcilable(
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

fn private_deletion_leftover(name: &str) -> Option<PrivateDeletionLeftover> {
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

fn looks_like_private_deletion_artifact(name: &str) -> bool {
    name.starts_with(".live-") && (name.contains(".deletion.v1.") || name.contains(".delete-"))
}

fn deletion_intent_session(name: &str) -> Option<crate::audio::session::SessionId> {
    name.strip_prefix("live-")
        .and_then(|session| session.strip_suffix(".deletion.v1.json"))
        .and_then(|session| crate::audio::session::SessionId::new(session.to_string()).ok())
}

fn push_bounded_candidate(candidates: &mut BTreeSet<String>, name: String, limit: usize) -> bool {
    if candidates.len() < limit {
        candidates.insert(name);
        return false;
    }
    let Some(last) = candidates.last().cloned() else {
        return false;
    };
    if name < last {
        candidates.remove(&last);
        candidates.insert(name);
    }
    true
}

pub(in crate::live::recordings) fn push_maintenance_warning(
    warnings: &mut Vec<String>,
    warning: String,
) {
    if warnings.len() < MAX_MAINTENANCE_WARNINGS && !warnings.contains(&warning) {
        warnings.push(warning);
    }
}
