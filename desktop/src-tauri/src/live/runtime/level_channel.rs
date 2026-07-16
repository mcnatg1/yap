use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc, Condvar, Mutex,
};
use std::thread::JoinHandle;
use std::time::Duration;

use tauri::Manager;

use super::super::state::{LiveLevelView, LiveSessionState};
use super::session_identity::active_session_matches;
use super::worker::join_worker;

const LEVEL_TICK: Duration = Duration::from_millis(50);

pub(super) struct LatestLevelSender {
    shared: Arc<LatestLevelShared>,
}

pub(super) struct LatestLevelReceiver {
    shared: Arc<LatestLevelShared>,
}

pub(super) struct LevelWorker {
    handle: Option<JoinHandle<()>>,
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

impl LevelWorker {
    pub(super) fn new() -> Self {
        Self { handle: None }
    }

    pub(super) fn start(
        &mut self,
        app: tauri::AppHandle,
        level: LatestLevelReceiver,
        session: u64,
        active_session: Arc<AtomicU64>,
    ) {
        if let Some(handle) = self.handle.take() {
            if let Err(error) = join_worker(handle) {
                crate::stt::log_yap(&format!("live level worker shutdown failed: {error}"));
            }
        }
        self.handle = Some(std::thread::spawn(move || {
            let state = app.state::<LiveSessionState>();
            while let Ok(value) = level.recv() {
                if !active_session_matches(active_session.load(Ordering::SeqCst), session) {
                    break;
                }
                let view = state.update_level(value);
                let level = LiveLevelView::from(&view);
                super::super::events::emit_level(&app, &level);
                std::thread::sleep(LEVEL_TICK);
            }
        }));
    }

    pub(super) fn shutdown(&mut self) -> Result<(), String> {
        match self.handle.take() {
            Some(handle) => join_worker(handle),
            None => Ok(()),
        }
    }
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
