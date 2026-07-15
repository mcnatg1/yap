use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};
use std::thread;
use std::time::Duration;

pub(super) struct SharedWarmup<T> {
    state: Mutex<SharedWarmupState<T>>,
    changed: Condvar,
}

enum SharedWarmupState<T> {
    Empty,
    Loading { cancelled: Arc<AtomicBool> },
    Ready(T),
    InUse,
    Failed(String),
}

pub(super) struct SharedWarmupLease<T: Send + 'static> {
    value: Option<T>,
    warmup: Arc<SharedWarmup<T>>,
}

impl<T> SharedWarmup<T>
where
    T: Send + 'static,
{
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(SharedWarmupState::Empty),
            changed: Condvar::new(),
        }
    }

    pub(super) fn request<F>(self: &Arc<Self>, worker_name: &str, load: F) -> Result<bool, String>
    where
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        let cancelled = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match &*state {
                SharedWarmupState::Loading { cancelled } => {
                    cancelled.store(false, Ordering::Release);
                    return Ok(false);
                }
                SharedWarmupState::Ready(_) | SharedWarmupState::InUse => return Ok(false),
                SharedWarmupState::Empty | SharedWarmupState::Failed(_) => {}
            }
            let cancelled = Arc::new(AtomicBool::new(false));
            *state = SharedWarmupState::Loading {
                cancelled: Arc::clone(&cancelled),
            };
            cancelled
        };

        let warmup = Arc::clone(self);
        let worker_cancelled = Arc::clone(&cancelled);
        if let Err(error) = thread::Builder::new()
            .name(worker_name.to_string())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(load))
                    .unwrap_or_else(|_| Err("Live model warmup panicked.".to_string()));
                warmup.complete_loading(&worker_cancelled, result);
            })
        {
            self.reset_failed_spawn(&cancelled);
            return Err(format!("Live model warmup worker could not start: {error}"));
        }
        Ok(true)
    }

    pub(super) fn wait_cancellable<F>(
        self: &Arc<Self>,
        cancelled: F,
    ) -> Result<Option<SharedWarmupLease<T>>, String>
    where
        F: Fn() -> bool,
    {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if cancelled() {
                return Ok(None);
            }
            match &*state {
                SharedWarmupState::Ready(_) => {
                    let SharedWarmupState::Ready(value) =
                        std::mem::replace(&mut *state, SharedWarmupState::InUse)
                    else {
                        unreachable!("ready warmup state was just matched")
                    };
                    return Ok(Some(SharedWarmupLease {
                        value: Some(value),
                        warmup: Arc::clone(self),
                    }));
                }
                SharedWarmupState::Failed(error) => return Err(error.clone()),
                SharedWarmupState::Empty => {
                    return Err("Live model warmup was not requested.".to_string())
                }
                SharedWarmupState::InUse => {
                    return Err("Live model is already owned by a stream.".to_string())
                }
                SharedWarmupState::Loading { .. } => {
                    let (next, _) = self
                        .changed
                        .wait_timeout(state, Duration::from_millis(25))
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    state = next;
                }
            }
        }
    }

    pub(super) fn cancel_loading(&self) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let SharedWarmupState::Loading { cancelled } = &*state {
            cancelled.store(true, Ordering::Release);
        }
        self.changed.notify_all();
    }

    fn complete_loading(&self, cancelled: &Arc<AtomicBool>, result: Result<T, String>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let owns_load = matches!(
            &*state,
            SharedWarmupState::Loading { cancelled: current }
                if Arc::ptr_eq(current, cancelled)
        );
        if !owns_load {
            return;
        }
        *state = if cancelled.load(Ordering::Acquire) {
            SharedWarmupState::Empty
        } else {
            match result {
                Ok(value) => SharedWarmupState::Ready(value),
                Err(error) => SharedWarmupState::Failed(error),
            }
        };
        self.changed.notify_all();
    }

    fn reset_failed_spawn(&self, cancelled: &Arc<AtomicBool>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(
            &*state,
            SharedWarmupState::Loading { cancelled: current }
                if Arc::ptr_eq(current, cancelled)
        ) {
            *state = SharedWarmupState::Empty;
        }
        self.changed.notify_all();
    }

    fn restore_ready(&self, value: T) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(*state, SharedWarmupState::InUse) {
            *state = SharedWarmupState::Ready(value);
        }
        self.changed.notify_all();
    }

    pub(super) fn release_in_use(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(*state, SharedWarmupState::InUse) {
            *state = SharedWarmupState::Empty;
        }
        self.changed.notify_all();
    }

    pub(super) fn clear_idle(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            match &*state {
                SharedWarmupState::Empty => return Ok(()),
                SharedWarmupState::InUse => {
                    return Err("Live model is still owned by a stream.".to_string())
                }
                SharedWarmupState::Loading { cancelled } => {
                    cancelled.store(true, Ordering::Release);
                    self.changed.notify_all();
                    state = self
                        .changed
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                SharedWarmupState::Ready(_) | SharedWarmupState::Failed(_) => {
                    let retired = std::mem::replace(&mut *state, SharedWarmupState::Empty);
                    self.changed.notify_all();
                    drop(state);
                    drop(retired);
                    return Ok(());
                }
            }
        }
    }

    #[cfg(test)]
    pub(super) fn is_loading_for_test(&self) -> bool {
        matches!(
            *self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            SharedWarmupState::Loading { .. }
        )
    }

    #[cfg(test)]
    pub(super) fn seed_ready_for_test(&self, value: T) {
        *self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = SharedWarmupState::Ready(value);
    }

    #[cfg(test)]
    pub(super) fn is_empty_for_test(&self) -> bool {
        matches!(
            *self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            SharedWarmupState::Empty
        )
    }
}

impl<T> SharedWarmupLease<T>
where
    T: Send + 'static,
{
    pub(super) fn commit(mut self) -> T {
        self.value
            .take()
            .expect("warmup lease commits exactly one model")
    }
}

impl<T: Send + 'static> Drop for SharedWarmupLease<T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.warmup.restore_ready(value);
        }
    }
}
