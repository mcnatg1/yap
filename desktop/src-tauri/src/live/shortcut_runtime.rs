use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{live, stt};

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
struct LiveShortcutDispatcher {
    events: mpsc::Sender<ShortcutDispatchEvent>,
    dictation_normalizer: Arc<ShortcutInputNormalizer>,
    paste_normalizer: Arc<ShortcutInputNormalizer>,
}

enum ShortcutDispatchEvent {
    Input {
        at: Instant,
        projected_mode: Option<live::state::LiveCaptureMode>,
        state: ShortcutState,
    },
    Reset,
    StartFinished(Option<live::state::LiveCaptureMode>),
}

impl LiveShortcutDispatcher {
    fn input(&self, state: ShortcutState, projected_mode: Option<live::state::LiveCaptureMode>) {
        if !self.dictation_normalizer.accept(state) {
            return;
        }
        let _ = self.events.send(ShortcutDispatchEvent::Input {
            at: Instant::now(),
            projected_mode,
            state,
        });
    }

    fn accept_paste(&self, state: ShortcutState) -> bool {
        self.paste_normalizer.accept(state)
    }

    fn reset(&self) {
        self.dictation_normalizer.reset();
        self.paste_normalizer.reset();
        let _ = self.events.send(ShortcutDispatchEvent::Reset);
    }
}

#[derive(Debug)]
struct LiveShortcutRegistration {
    hotkey: String,
    is_paste: bool,
    shortcut: Result<Shortcut, String>,
}

pub(crate) struct StartupShortcutPlan {
    registrations: Vec<LiveShortcutRegistration>,
}

pub(crate) fn prepare(settings: &live::settings::LiveSettings) -> StartupShortcutPlan {
    let mut registrations: Vec<LiveShortcutRegistration> = Vec::new();
    for (configured, is_paste) in [
        (settings.hotkey.as_deref(), false),
        (settings.paste_hotkey.as_deref(), true),
    ] {
        let Some(hotkey) = configured
            .map(str::trim)
            .filter(|hotkey| !hotkey.is_empty())
        else {
            continue;
        };
        if registrations
            .iter()
            .any(|existing| live::hotkeys::configured_hotkeys_match(&existing.hotkey, hotkey))
        {
            continue;
        }
        registrations.push(LiveShortcutRegistration {
            hotkey: hotkey.to_string(),
            is_paste,
            shortcut: live::hotkeys::parse_hotkey_for(
                hotkey,
                if is_paste {
                    live::hotkeys::HotkeyPurpose::PasteLast
                } else {
                    live::hotkeys::HotkeyPurpose::Dictation
                },
            ),
        });
    }
    StartupShortcutPlan { registrations }
}

pub(crate) fn install(app: &mut tauri::App, plan: StartupShortcutPlan) -> tauri::Result<()> {
    let shortcut_dispatcher = spawn_shortcut_dispatcher(app.handle().clone());
    let handler_dispatcher = shortcut_dispatcher.clone();
    app.manage(shortcut_dispatcher);
    app.handle().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, shortcut, event| {
                if app
                    .state::<live::hotkey_commands::HotkeyEnrollmentGate>()
                    .is_active()
                {
                    handler_dispatcher.reset();
                    return;
                }
                let snapshot = {
                    let live = app.state::<live::LiveSessionState>();
                    live.snapshot()
                };
                if live::actions::configured_hotkey_matches_shortcut(
                    &snapshot.paste_hotkey,
                    shortcut,
                ) {
                    if live::state::is_live_session_started(snapshot.status) {
                        handler_dispatcher.paste_normalizer.reset();
                        return;
                    }
                    if !handler_dispatcher.accept_paste(event.state()) {
                        return;
                    }
                    if event.state() == ShortcutState::Released {
                        let target = live::injection::capture_target();
                        let app = app.clone();
                        std::thread::spawn(move || {
                            live::actions::inject_last_live_transcript(&app, target);
                        });
                    }
                    return;
                }
                if !live::actions::configured_hotkey_matches_shortcut(&snapshot.hotkey, shortcut) {
                    return;
                }
                if snapshot.status == live::state::LiveSessionStatus::Saving {
                    handler_dispatcher.reset();
                    return;
                }
                handler_dispatcher.input(event.state(), snapshot.active_capture_mode);
            })
            .build(),
    )?;

    for registration in &plan.registrations {
        match registration.shortcut.as_ref() {
            Ok(shortcut) => {
                if let Err(error) = app.handle().global_shortcut().register(*shortcut) {
                    record_startup_shortcut_failure(app.handle(), registration, &error.to_string());
                }
            }
            Err(error) => {
                record_startup_shortcut_failure(app.handle(), registration, error);
            }
        }
    }
    Ok(())
}

pub(crate) fn reset(app: &tauri::AppHandle) {
    app.state::<LiveShortcutDispatcher>().reset();
}

fn spawn_shortcut_dispatcher(app: tauri::AppHandle) -> LiveShortcutDispatcher {
    let (events, event_rx) = mpsc::channel();
    let (actions, action_rx) = mpsc::channel();
    let dictation_normalizer = Arc::new(ShortcutInputNormalizer::default());
    let paste_normalizer = Arc::new(ShortcutInputNormalizer::default());
    let action_events = events.clone();
    let action_app = app.clone();
    std::thread::Builder::new()
        .name("live-shortcut-actions".into())
        .spawn(move || run_shortcut_actions(action_app, action_rx, action_events))
        .expect("live shortcut action worker must start");
    std::thread::Builder::new()
        .name("live-shortcut-input".into())
        .spawn(move || run_shortcut_input(app, event_rx, actions))
        .expect("live shortcut input worker must start");
    LiveShortcutDispatcher {
        events,
        dictation_normalizer,
        paste_normalizer,
    }
}

fn run_shortcut_input(
    app: tauri::AppHandle,
    events: mpsc::Receiver<ShortcutDispatchEvent>,
    actions: mpsc::Sender<live::hotkeys::LiveShortcutAction>,
) {
    let mut interaction = live::hotkeys::LiveShortcutInteraction::default();
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
                    let action = interaction.hold_elapsed(press_id, Instant::now(), projected_mode);
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
                let action = interaction.pressed(at, projected_mode);
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
                let action = interaction.released(at, projected_mode);
                queue_shortcut_action(&app, &actions, action);
            }
            ShortcutDispatchEvent::Reset => {
                hold_deadline = None;
                interaction.reset();
            }
            ShortcutDispatchEvent::StartFinished(active_mode) => {
                interaction.finish_start(active_mode);
            }
        }
    }
}

fn queue_shortcut_action(
    app: &tauri::AppHandle,
    actions: &mpsc::Sender<live::hotkeys::LiveShortcutAction>,
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
        let _ = actions.send(action);
    }
}

fn run_shortcut_actions(
    app: tauri::AppHandle,
    actions: mpsc::Receiver<live::hotkeys::LiveShortcutAction>,
    events: mpsc::Sender<ShortcutDispatchEvent>,
) {
    while let Ok(action) = actions.recv() {
        let started = matches!(action, live::hotkeys::LiveShortcutAction::Start(_));
        let active_mode = live::actions::handle_live_shortcut_action(app.clone(), action);
        if started {
            let _ = events.send(ShortcutDispatchEvent::StartFinished(active_mode));
        }
    }
}

fn apply_startup_shortcut_failure(
    view: &mut live::state::LiveSessionView,
    is_paste_shortcut: bool,
) {
    if is_paste_shortcut {
        view.paste_hotkey.clear();
        if view.error.as_deref() != Some(live::hotkey_commands::DICTATION_UNAVAILABLE_ERROR) {
            view.error = Some(live::hotkey_commands::PASTE_UNAVAILABLE_ERROR.into());
        }
        return;
    }

    view.hotkey.clear();
    view.error = Some(live::hotkey_commands::DICTATION_UNAVAILABLE_ERROR.into());
    view.route = live::state::LiveRoute::Blocked;
    view.status = live::state::LiveSessionStatus::Blocked;
}

fn record_startup_shortcut_failure(
    app: &tauri::AppHandle,
    registration: &LiveShortcutRegistration,
    reason: &str,
) {
    stt::log_yap(&format!(
        "live {} hotkey unavailable: {reason}",
        if registration.is_paste {
            "paste"
        } else {
            "dictation"
        }
    ));
    let live = app.state::<live::LiveSessionState>();
    live.mark_startup_shortcut_failure(registration.is_paste);
    let view = live.update(|view| {
        apply_startup_shortcut_failure(view, registration.is_paste);
    });
    if let Err(persist_error) = live::settings::save_view(&view) {
        stt::log_yap(&format!(
            "failed to persist unavailable live shortcut cleanup: {persist_error}"
        ));
    }
    live::events::emit_session(app, &view);
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
    fn startup_shortcut_plan_keeps_dictation_and_paste_hotkeys() {
        let settings = live::settings::LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: Some(live::settings::DEFAULT_PASTE_HOTKEY.into()),
            capture_mode: live::state::LiveCaptureMode::PushToTalk,
            input_device_id: None,
        };

        assert_eq!(
            prepare(&settings)
                .registrations
                .iter()
                .map(|registration| (registration.hotkey.clone(), registration.is_paste))
                .collect::<Vec<_>>(),
            vec![
                ("Ctrl+Shift+Space".to_string(), false),
                (live::settings::DEFAULT_PASTE_HOTKEY.to_string(), true),
            ]
        );
    }

    #[test]
    fn startup_shortcut_plan_deduplicates_conflicting_hotkeys() {
        let settings = live::settings::LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: Some("Ctrl+Shift+Space".into()),
            capture_mode: live::state::LiveCaptureMode::PushToTalk,
            input_device_id: None,
        };

        assert_eq!(
            prepare(&settings)
                .registrations
                .iter()
                .map(|registration| registration.hotkey.as_str())
                .collect::<Vec<_>>(),
            vec!["Ctrl+Shift+Space"]
        );
    }

    #[test]
    fn startup_shortcut_plan_reports_invalid_dictation_and_paste_settings() {
        let settings = live::settings::LiveSettings {
            hotkey: Some("Ctrl".into()),
            paste_hotkey: Some("Shift".into()),
            ..Default::default()
        };

        let plan = prepare(&settings);

        assert_eq!(plan.registrations.len(), 2);
        assert!(!plan.registrations[0].is_paste);
        assert!(plan.registrations[0].shortcut.is_err());
        assert!(plan.registrations[1].is_paste);
        assert!(plan.registrations[1].shortcut.is_err());
    }

    #[test]
    fn failed_startup_shortcut_is_cleared_for_settings_recovery() {
        let mut dictation =
            live::state::LiveSessionView::from_settings(&live::settings::LiveSettings::default());
        apply_startup_shortcut_failure(&mut dictation, false);
        assert_eq!(dictation.hotkey, "");
        assert_eq!(dictation.status, live::state::LiveSessionStatus::Blocked);

        let mut paste =
            live::state::LiveSessionView::from_settings(&live::settings::LiveSettings {
                paste_hotkey: Some(live::settings::DEFAULT_PASTE_HOTKEY.into()),
                ..Default::default()
            });
        apply_startup_shortcut_failure(&mut paste, true);
        assert_eq!(paste.paste_hotkey, "");
        assert_eq!(paste.status, live::state::LiveSessionStatus::Idle);
    }

    #[test]
    fn paste_failure_does_not_overwrite_dictation_block_ownership() {
        let mut view =
            live::state::LiveSessionView::from_settings(&live::settings::LiveSettings::default());

        apply_startup_shortcut_failure(&mut view, false);
        apply_startup_shortcut_failure(&mut view, true);

        assert_eq!(
            view.error.as_deref(),
            Some(live::hotkey_commands::DICTATION_UNAVAILABLE_ERROR)
        );
        assert_eq!(view.status, live::state::LiveSessionStatus::Blocked);
        assert_eq!(view.hotkey, "");
        assert_eq!(view.paste_hotkey, "");
    }
}
