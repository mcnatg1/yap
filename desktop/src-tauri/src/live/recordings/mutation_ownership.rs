use std::sync::{Condvar, Mutex, OnceLock};

static SESSION_MUTATION_OWNERSHIP: OnceLock<SessionMutationOwnership> = OnceLock::new();

#[derive(Default)]
struct SessionMutationOwnershipState {
    next_ticket: u64,
    serving_ticket: u64,
}

#[derive(Default)]
struct SessionMutationOwnership {
    state: Mutex<SessionMutationOwnershipState>,
    changed: Condvar,
}

pub(super) struct SessionMutationOwnershipGuard {
    owner: &'static SessionMutationOwnership,
}

impl Drop for SessionMutationOwnershipGuard {
    fn drop(&mut self) {
        let mut state = self
            .owner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.serving_ticket = state.serving_ticket.wrapping_add(1);
        self.owner.changed.notify_all();
    }
}

pub(super) fn session_mutation_ownership() -> SessionMutationOwnershipGuard {
    session_mutation_ownership_with_queue_observer(|| {})
}

pub(super) fn session_mutation_ownership_with_queue_observer<F>(
    queued: F,
) -> SessionMutationOwnershipGuard
where
    F: FnOnce(),
{
    let owner = SESSION_MUTATION_OWNERSHIP.get_or_init(SessionMutationOwnership::default);
    let mut state = owner
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let ticket = state.next_ticket;
    state.next_ticket = state.next_ticket.wrapping_add(1);
    queued();
    while state.serving_ticket != ticket {
        state = owner
            .changed
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    drop(state);
    SessionMutationOwnershipGuard { owner }
}
