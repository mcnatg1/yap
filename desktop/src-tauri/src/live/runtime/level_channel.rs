use std::sync::{mpsc, Arc, Condvar, Mutex};

pub(super) struct LatestLevelSender {
    shared: Arc<LatestLevelShared>,
}

pub(super) struct LatestLevelReceiver {
    shared: Arc<LatestLevelShared>,
}

struct LatestLevelShared {
    state: Mutex<LatestLevelState>,
    changed: Condvar,
}

struct LatestLevelState {
    latest: Option<f32>,
    producers: usize,
    receiver_open: bool,
}

impl LatestLevelReceiver {
    pub(super) fn recv(&self) -> Result<f32, mpsc::RecvError> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if let Some(level) = state.latest.take() {
                return Ok(level);
            }
            if state.producers == 0 {
                return Err(mpsc::RecvError);
            }
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    #[cfg(test)]
    pub(super) fn recv_with_ready_hook(
        &self,
        ready: impl FnOnce(),
    ) -> Result<f32, mpsc::RecvError> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while state.latest.is_none() {
            if state.producers == 0 {
                return Err(mpsc::RecvError);
            }
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        drop(state);
        ready();
        self.recv()
    }
}

pub(super) fn level_channel() -> (LatestLevelSender, LatestLevelReceiver) {
    let shared = Arc::new(LatestLevelShared {
        state: Mutex::new(LatestLevelState {
            latest: None,
            producers: 1,
            receiver_open: true,
        }),
        changed: Condvar::new(),
    });
    (
        LatestLevelSender {
            shared: Arc::clone(&shared),
        },
        LatestLevelReceiver { shared },
    )
}

pub(super) fn publish_level(levels: &LatestLevelSender, level: f32) -> bool {
    let mut state = levels
        .shared
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !state.receiver_open {
        return false;
    }
    state.latest = Some(level);
    drop(state);
    levels.shared.changed.notify_one();
    true
}

impl Clone for LatestLevelSender {
    fn clone(&self) -> Self {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.producers = state.producers.saturating_add(1);
        drop(state);
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Drop for LatestLevelSender {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.producers = state.producers.saturating_sub(1);
        let closed = state.producers == 0;
        drop(state);
        if closed {
            self.shared.changed.notify_all();
        }
    }
}

impl Drop for LatestLevelReceiver {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.receiver_open = false;
        state.latest = None;
        drop(state);
        self.shared.changed.notify_all();
    }
}
