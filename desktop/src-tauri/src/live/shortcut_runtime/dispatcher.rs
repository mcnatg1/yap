use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

use tauri::Manager;
use tauri_plugin_global_shortcut::ShortcutState;

use crate::live;

const SHORTCUT_EVENT_CAPACITY: usize = 16;
const SHORTCUT_ACTION_CAPACITY: usize = 4;

#[derive(Default)]
struct ShortcutInputNormalizer {
    key_down: AtomicBool,
}

impl ShortcutInputNormalizer {
    fn accept(&self, state: ShortcutState) -> bool {
        match state {
            ShortcutState::Pressed => self
                .key_down
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            ShortcutState::Released => self.key_down.swap(false, Ordering::AcqRel),
        }
    }

    fn reset(&self) {
        self.key_down.store(false, Ordering::Release);
    }
}

#[derive(Clone)]
pub(super) struct LiveShortcutDispatcher {
    events: mpsc::SyncSender<ShortcutDispatchEvent>,
    dictation_normalizer: Arc<ShortcutInputNormalizer>,
    paste_normalizer: Arc<ShortcutInputNormalizer>,
}

enum ShortcutDispatchEvent {
    Input {
        at: Instant,
        projected_mode: Option<live::state::LiveCaptureMode>,
        state: ShortcutState,
    },
    Paste(Option<live::injection::InjectionTarget>),
    Reset,
}

enum ShortcutWorkerAction {
    Live(live::hotkeys::LiveShortcutAction),
    Paste(Option<live::injection::InjectionTarget>),
}

impl LiveShortcutDispatcher {
    pub(super) fn input(
        &self,
        state: ShortcutState,
        projected_mode: Option<live::state::LiveCaptureMode>,
    ) {
        if !self.dictation_normalizer.accept(state) {
            return;
        }
        self.send(ShortcutDispatchEvent::Input {
            at: Instant::now(),
            projected_mode,
            state,
        });
    }

    pub(super) fn accept_paste(&self, state: ShortcutState) -> bool {
        self.paste_normalizer.accept(state)
    }

    pub(super) fn paste(&self, target: Option<live::injection::InjectionTarget>) {
        self.send(ShortcutDispatchEvent::Paste(target));
    }

    pub(super) fn reset_paste(&self) {
        self.paste_normalizer.reset();
    }

    pub(super) fn reset(&self) {
        self.dictation_normalizer.reset();
        self.paste_normalizer.reset();
        self.send(ShortcutDispatchEvent::Reset);
    }

    fn send(&self, event: ShortcutDispatchEvent) {
        // Human shortcut input applies bounded backpressure instead of dropping a
        // release event that could leave push-to-talk capture running.
        if self.events.send(event).is_err() {
            self.dictation_normalizer.reset();
            self.paste_normalizer.reset();
        }
    }
}

pub(super) fn spawn(app: tauri::AppHandle) -> LiveShortcutDispatcher {
    let (events, event_rx, actions, action_rx) = shortcut_channels();
    let dictation_normalizer = Arc::new(ShortcutInputNormalizer::default());
    let paste_normalizer = Arc::new(ShortcutInputNormalizer::default());
    let interaction = Arc::new(Mutex::new(live::hotkeys::LiveShortcutInteraction::default()));
    let action_interaction = Arc::clone(&interaction);
    let action_app = app.clone();
    std::thread::Builder::new()
        .name("live-shortcut-actions".into())
        .spawn(move || run_shortcut_actions(action_app, action_rx, action_interaction))
        .expect("live shortcut action worker must start");
    std::thread::Builder::new()
        .name("live-shortcut-input".into())
        .spawn(move || run_shortcut_input(app, event_rx, actions, interaction))
        .expect("live shortcut input worker must start");
    LiveShortcutDispatcher {
        events,
        dictation_normalizer,
        paste_normalizer,
    }
}

fn shortcut_channels() -> (
    mpsc::SyncSender<ShortcutDispatchEvent>,
    mpsc::Receiver<ShortcutDispatchEvent>,
    mpsc::SyncSender<ShortcutWorkerAction>,
    mpsc::Receiver<ShortcutWorkerAction>,
) {
    let (events, event_rx) = mpsc::sync_channel(SHORTCUT_EVENT_CAPACITY);
    let (actions, action_rx) = mpsc::sync_channel(SHORTCUT_ACTION_CAPACITY);
    (events, event_rx, actions, action_rx)
}

fn run_shortcut_input(
    app: tauri::AppHandle,
    events: mpsc::Receiver<ShortcutDispatchEvent>,
    actions: mpsc::SyncSender<ShortcutWorkerAction>,
    interaction: Arc<Mutex<live::hotkeys::LiveShortcutInteraction>>,
) {
    let mut hold_deadline: Option<(u64, Instant)> = None;
    loop {
        let event = if let Some((press_id, deadline)) = hold_deadline {
            match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(event) => event,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    hold_deadline = None;
                    let projected_mode = app
                        .state::<live::LiveSessionState>()
                        .snapshot()
                        .active_capture_mode;
                    let action = lock_shortcut_interaction(&interaction).hold_elapsed(
                        press_id,
                        Instant::now(),
                        projected_mode,
                    );
                    queue_shortcut_action(&app, &actions, action);
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match events.recv() {
                Ok(event) => event,
                Err(_) => break,
            }
        };

        match event {
            ShortcutDispatchEvent::Input {
                at,
                projected_mode,
                state: ShortcutState::Pressed,
            } => {
                let action = lock_shortcut_interaction(&interaction).pressed(at, projected_mode);
                if let live::hotkeys::LiveShortcutAction::ScheduleHold(press_id) = action {
                    hold_deadline = Some((
                        press_id,
                        at + Duration::from_millis(live::hotkeys::SHORTCUT_HOLD_MS),
                    ));
                } else {
                    queue_shortcut_action(&app, &actions, action);
                }
            }
            ShortcutDispatchEvent::Input {
                at,
                projected_mode,
                state: ShortcutState::Released,
            } => {
                hold_deadline = None;
                let action = lock_shortcut_interaction(&interaction).released(at, projected_mode);
                queue_shortcut_action(&app, &actions, action);
            }
            ShortcutDispatchEvent::Paste(target) => {
                let _ = actions.send(ShortcutWorkerAction::Paste(target));
            }
            ShortcutDispatchEvent::Reset => {
                hold_deadline = None;
                lock_shortcut_interaction(&interaction).reset();
            }
        }
    }
}

fn lock_shortcut_interaction(
    interaction: &Mutex<live::hotkeys::LiveShortcutInteraction>,
) -> std::sync::MutexGuard<'_, live::hotkeys::LiveShortcutInteraction> {
    interaction
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn queue_shortcut_action(
    app: &tauri::AppHandle,
    actions: &mpsc::SyncSender<ShortcutWorkerAction>,
    action: live::hotkeys::LiveShortcutAction,
) {
    if matches!(action, live::hotkeys::LiveShortcutAction::Stop) {
        app.state::<live::runtime::LiveRuntime>()
            .cancel_pending_start();
    }
    if !matches!(
        action,
        live::hotkeys::LiveShortcutAction::None
            | live::hotkeys::LiveShortcutAction::ScheduleHold(_)
    ) {
        let _ = actions.send(ShortcutWorkerAction::Live(action));
    }
}

fn run_shortcut_actions(
    app: tauri::AppHandle,
    actions: mpsc::Receiver<ShortcutWorkerAction>,
    interaction: Arc<Mutex<live::hotkeys::LiveShortcutInteraction>>,
) {
    while let Ok(action) = actions.recv() {
        match action {
            ShortcutWorkerAction::Live(action) => {
                let started = matches!(action, live::hotkeys::LiveShortcutAction::Start(_));
                let active_mode = live::actions::handle_live_shortcut_action(app.clone(), action);
                if started {
                    lock_shortcut_interaction(&interaction).finish_start(active_mode);
                }
            }
            ShortcutWorkerAction::Paste(target) => {
                let snapshot = app.state::<live::LiveSessionState>().snapshot();
                if !live::state::is_live_session_started(snapshot.status) {
                    live::actions::inject_last_live_transcript(&app, target);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_backend_normalizes_repeat_delayed_release_and_reset() {
        let normalizer = ShortcutInputNormalizer::default();

        assert!(normalizer.accept(ShortcutState::Pressed));
        assert!(!normalizer.accept(ShortcutState::Pressed));
        assert!(normalizer.accept(ShortcutState::Released));
        assert!(!normalizer.accept(ShortcutState::Released));

        assert!(normalizer.accept(ShortcutState::Pressed));
        normalizer.reset();
        assert!(!normalizer.accept(ShortcutState::Released));
        assert!(normalizer.accept(ShortcutState::Pressed));
    }

    #[test]
    fn shortcut_work_queues_have_fixed_capacity() {
        let (events, _event_rx, actions, _action_rx) = shortcut_channels();

        for _ in 0..SHORTCUT_EVENT_CAPACITY {
            events.try_send(ShortcutDispatchEvent::Reset).unwrap();
        }
        assert!(matches!(
            events.try_send(ShortcutDispatchEvent::Reset),
            Err(mpsc::TrySendError::Full(_))
        ));

        for _ in 0..SHORTCUT_ACTION_CAPACITY {
            actions
                .try_send(ShortcutWorkerAction::Live(
                    live::hotkeys::LiveShortcutAction::Stop,
                ))
                .unwrap();
        }
        assert!(matches!(
            actions.try_send(ShortcutWorkerAction::Live(
                live::hotkeys::LiveShortcutAction::Stop,
            )),
            Err(mpsc::TrySendError::Full(_))
        ));
    }
}
