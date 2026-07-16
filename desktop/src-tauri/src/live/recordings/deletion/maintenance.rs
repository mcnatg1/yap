use std::{collections::HashMap, path::Path};

#[cfg(test)]
use super::super::mutation_ownership::session_mutation_ownership;
use super::{
    evidence::{generic_delete_quarantine, reconcile_intent_evidence_quarantines_while_owned},
    execution::resume_deletion_intent_while_owned,
};

mod candidates;
mod leftovers;

pub(in crate::live::recordings) use candidates::RotatingDeletionCandidates;
use candidates::{deletion_cleanup_cursor, update_deletion_cleanup_cursor, DeletionCleanupCursor};
use leftovers::{
    deletion_intent_session, looks_like_private_deletion_artifact, private_deletion_leftover,
    private_deletion_leftover_is_reconcilable, reconcile_private_deletion_leftovers,
};

pub(in crate::live::recordings) const MAX_MAINTENANCE_WARNINGS: usize = 8;
pub(in crate::live::recordings) const MAX_PRIVATE_DELETION_LEFTOVERS: usize = 128;

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

pub(in crate::live::recordings) fn push_maintenance_warning(
    warnings: &mut Vec<String>,
    warning: String,
) {
    if warnings.len() < MAX_MAINTENANCE_WARNINGS && !warnings.contains(&warning) {
        warnings.push(warning);
    }
}
