mod evidence;
mod execution;
mod intent;
mod maintenance;

#[cfg(test)]
pub(super) use evidence::reconcile_intent_evidence_quarantines;
pub(super) use execution::{
    admit_expected_private_artifact_identity,
    delete_committed_session_in_dir_with_publication_barrier_while_owned,
    delete_saved_live_session_in_dir, resume_deletion_intent_while_owned,
};
#[cfg(test)]
pub(super) use execution::{
    delete_committed_session_in_dir_with_publication_barrier, resume_deletion_intent,
};
pub(super) use intent::{
    admit_deletion_artifact, deletion_intent_name, validate_deletion_intent,
    write_deletion_intent_with_publication_barrier_while_owned, DeletionIntent,
    DELETION_INTENT_SCHEMA_VERSION,
};
#[cfg(test)]
pub(super) use intent::{
    build_deletion_intent, write_deletion_intent, write_deletion_intent_with_publication_barrier,
    DeletionArtifact,
};
pub(super) use maintenance::{
    push_maintenance_warning, reconcile_pending_deletion_intents_while_owned,
};
#[cfg(test)]
pub(super) use maintenance::{
    reconcile_pending_deletion_intents, RotatingDeletionCandidates, MAX_MAINTENANCE_WARNINGS,
    MAX_PRIVATE_DELETION_LEFTOVERS,
};
