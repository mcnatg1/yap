use std::sync::{Arc, Condvar, Mutex};

pub(super) struct LifecycleGate {
    state: Mutex<LifecycleQueue>,
    changed: Condvar,
}

struct LifecycleQueue {
    active: LifecycleState,
    next_ticket: u64,
    serving_ticket: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LifecycleState {
    Idle,
    Starting,
    Stopping,
}

pub(super) struct LifecycleOperation<'a> {
    gate: &'a LifecycleGate,
}

pub(super) struct OwnedLifecycleOperation {
    gate: Arc<LifecycleGate>,
}

impl LifecycleGate {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(LifecycleQueue {
                active: LifecycleState::Idle,
                next_ticket: 0,
                serving_ticket: 0,
            }),
            changed: Condvar::new(),
        }
    }

    pub(super) fn begin_start(&self) -> LifecycleOperation<'_> {
        self.begin(LifecycleState::Starting)
    }

    #[cfg(test)]
    pub(super) fn begin_start_with_wait_hook<F>(&self, on_wait: F) -> LifecycleOperation<'_>
    where
        F: FnOnce(),
    {
        self.begin_with_wait_hook(LifecycleState::Starting, Some(on_wait))
    }

    pub(super) fn begin_stop(&self) -> LifecycleOperation<'_> {
        self.begin(LifecycleState::Stopping)
    }

    pub(super) fn begin_stop_owned(self: &Arc<Self>) -> OwnedLifecycleOperation {
        self.acquire(LifecycleState::Stopping, None::<fn()>);
        OwnedLifecycleOperation {
            gate: Arc::clone(self),
        }
    }

    #[cfg(test)]
    pub(super) fn begin_stop_with_wait_hook<F>(&self, on_wait: F) -> LifecycleOperation<'_>
    where
        F: FnOnce(),
    {
        self.begin_with_wait_hook(LifecycleState::Stopping, Some(on_wait))
    }

    fn begin(&self, next: LifecycleState) -> LifecycleOperation<'_> {
        self.begin_with_wait_hook(next, None::<fn()>)
    }

    fn begin_with_wait_hook<F>(
        &self,
        next: LifecycleState,
        on_wait: Option<F>,
    ) -> LifecycleOperation<'_>
    where
        F: FnOnce(),
    {
        self.acquire(next, on_wait);
        LifecycleOperation { gate: self }
    }

    fn acquire<F>(&self, next: LifecycleState, mut on_wait: Option<F>)
    where
        F: FnOnce(),
    {
        let mut state = self.state.lock().expect("live transition gate poisoned");
        let ticket = state.next_ticket;
        state.next_ticket = state.next_ticket.wrapping_add(1);
        while state.active != LifecycleState::Idle || state.serving_ticket != ticket {
            if let Some(on_wait) = on_wait.take() {
                on_wait();
            }
            state = self
                .changed
                .wait(state)
                .expect("live transition gate poisoned");
        }
        state.active = next;
    }

    fn complete(&self) {
        let mut state = self.state.lock().expect("live transition gate poisoned");
        state.active = LifecycleState::Idle;
        state.serving_ticket = state.serving_ticket.wrapping_add(1);
        self.changed.notify_all();
    }
}

impl Drop for LifecycleOperation<'_> {
    fn drop(&mut self) {
        self.gate.complete();
    }
}

impl Drop for OwnedLifecycleOperation {
    fn drop(&mut self) {
        self.gate.complete();
    }
}
