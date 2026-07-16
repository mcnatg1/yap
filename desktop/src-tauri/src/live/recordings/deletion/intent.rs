mod model;
mod publication;
mod validation;

pub(in crate::live::recordings) use model::{
    admit_deletion_artifact, build_deletion_intent, deletion_intent_name, DeletionArtifact,
    DeletionIntent, DELETION_INTENT_SCHEMA_VERSION,
};
pub(in crate::live::recordings) use publication::write_deletion_intent_with_publication_barrier_while_owned;
#[cfg(test)]
pub(in crate::live::recordings) use publication::{
    write_deletion_intent, write_deletion_intent_with_publication_barrier,
};
pub(in crate::live::recordings) use validation::validate_deletion_intent;
pub(super) use validation::{prove_intent_against_current_commit, revalidate_intent_artifact};
