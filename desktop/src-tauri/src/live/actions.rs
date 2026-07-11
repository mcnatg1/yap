use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tauri::Manager;
use tauri_plugin_global_shortcut::Shortcut;

use crate::{authorization, live, runtime, runtime_policy, stt};

const INJECTION_COPIED_ERROR: &str = "Couldn't insert text here. Transcript copied; press Ctrl+V.";
const INJECTION_FAILED_ERROR: &str = "Couldn't insert or copy this transcript.";

fn append_error(existing: Option<String>, message: &str) -> String {
    match existing {
        Some(existing) if existing.contains(message) => existing,
        Some(existing) => format!("{existing} {message}"),
        None => message.into(),
    }
}

fn without_injection_feedback(error: Option<&str>) -> Option<String> {
    let error = error?;
    if !error.contains(INJECTION_COPIED_ERROR) && !error.contains(INJECTION_FAILED_ERROR) {
        return Some(error.to_string());
    }
    let cleaned = error
        .replace(INJECTION_COPIED_ERROR, "")
        .replace(INJECTION_FAILED_ERROR, "");
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    (!cleaned.is_empty()).then_some(cleaned)
}

pub(crate) fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(authorization::MAIN_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub(crate) fn start_live_from_app(app: &tauri::AppHandle) {
    let live = app.state::<live::LiveSessionState>();
    let live_runtime = app.state::<live::runtime::LiveRuntime>();
    let stt = app.state::<stt::dispatch::SttState>();
    let orchestrator = app.state::<runtime::RuntimeOrchestratorState>();
    let capture_mode = live.snapshot().capture_mode;
    let _ = start_live_runtime(
        app.clone(),
        &live,
        &live_runtime,
        &stt,
        &orchestrator,
        capture_mode,
    );
}

pub(crate) fn stop_live_from_app(app: &tauri::AppHandle) {
    let live = app.state::<live::LiveSessionState>();
    let live_runtime = app.state::<live::runtime::LiveRuntime>();
    let orchestrator = app.state::<runtime::RuntimeOrchestratorState>();
    let _ = stop_live_runtime(app.clone(), &live, &live_runtime, &orchestrator);
}

struct CompletionEffects<I, S> {
    injection: Result<Option<I>, String>,
    save: Result<S, String>,
}

fn run_completion_effects_with<I, S>(
    view: &live::state::LiveSessionView,
    remember: impl FnOnce(&str),
    inject: impl FnOnce(&str) -> Result<I, String>,
    save: impl FnOnce() -> Result<S, String>,
) -> CompletionEffects<I, S> {
    let injection = match live::recordings::completed_transcript_text(view) {
        Some(text) => {
            remember(&text);
            inject(&text).map(Some)
        }
        None => Ok(None),
    };
    CompletionEffects {
        injection,
        save: save(),
    }
}

pub(crate) fn inject_last_live_transcript(
    app: &tauri::AppHandle,
    target: Option<live::injection::InjectionTarget>,
) {
    let live = app.state::<live::LiveSessionState>();
    let result = match live.last_completed_transcript() {
        Some(text) => live::injection::inject_text(app, target, &text).map(Some),
        None => Ok(None),
    };
    let view = apply_injection_result(&live, result);
    if view.error.is_some() {
        if let Err(error) = live::overlay_window::ensure_active(app) {
            stt::log_yap(&format!("live paste feedback show failed: {error}"));
        }
    } else if view.visibility == live::state::LiveOverlayVisibility::Enabled {
        if let Err(error) = live::overlay_window::ensure_idle(app) {
            stt::log_yap(&format!("live paste idle show failed: {error}"));
        }
    } else if let Some(window) = app.get_webview_window(live::overlay_window::WINDOW_LABEL) {
        let _ = window.hide();
    }
    live::events::emit_session(app, &view);
}

fn apply_injection_result(
    live: &live::LiveSessionState,
    result: Result<Option<live::injection::InjectionOutcome>, String>,
) -> live::state::LiveSessionView {
    match result {
        Ok(Some(live::injection::InjectionOutcome::Injected)) => live.update(|view| {
            view.error = without_injection_feedback(view.error.as_deref());
        }),
        Ok(Some(live::injection::InjectionOutcome::CopiedOnly(reason))) => {
            stt::log_yap(&format!("live injection copied fallback: {reason}"));
            live.update(|view| {
                let existing = without_injection_feedback(view.error.as_deref());
                view.error = Some(append_error(existing, INJECTION_COPIED_ERROR));
            })
        }
        Ok(Some(live::injection::InjectionOutcome::Ignored)) | Ok(None) => live.snapshot(),
        Err(error) => {
            stt::log_yap(&format!("live transcript injection failed: {error}"));
            live.update(|view| {
                let existing = without_injection_feedback(view.error.as_deref());
                view.error = Some(append_error(existing, INJECTION_FAILED_ERROR));
            })
        }
    }
}

pub(crate) fn configured_hotkey_matches_shortcut(configured: &str, shortcut: &Shortcut) -> bool {
    !configured.trim().is_empty()
        && live::hotkeys::parse_hotkey(configured)
            .map(|configured| configured == *shortcut)
            .unwrap_or(false)
}

pub(crate) fn warm_on_intent(app: &tauri::AppHandle, live_runtime: &live::runtime::LiveRuntime) {
    let app = app.clone();
    let live_runtime = live_runtime.clone();
    std::thread::spawn(move || {
        if let Err(error) = live_runtime.warm(app) {
            stt::log_yap(&format!("live warmup skipped: {error}"));
        }
    });
}

pub(crate) fn handle_live_shortcut_action(
    app: tauri::AppHandle,
    interaction: Arc<Mutex<live::hotkeys::LiveShortcutInteraction>>,
    action: live::hotkeys::LiveShortcutAction,
) {
    match action {
        live::hotkeys::LiveShortcutAction::None => {}
        live::hotkeys::LiveShortcutAction::ScheduleHold(press_id) => {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(live::hotkeys::SHORTCUT_HOLD_MS));
                let active_mode = {
                    let live = app.state::<live::LiveSessionState>();
                    live.snapshot().active_capture_mode
                };
                let action = interaction
                    .lock()
                    .expect("live shortcut state poisoned")
                    .hold_elapsed(press_id, Instant::now(), active_mode);
                handle_live_shortcut_action(app, interaction, action);
            });
        }
        live::hotkeys::LiveShortcutAction::Start(capture_mode) => {
            let live = app.state::<live::LiveSessionState>();
            let live_runtime = app.state::<live::runtime::LiveRuntime>();
            let stt = app.state::<stt::dispatch::SttState>();
            let orchestrator = app.state::<runtime::RuntimeOrchestratorState>();
            let view = start_live_runtime(
                app.clone(),
                &live,
                &live_runtime,
                &stt,
                &orchestrator,
                capture_mode,
            );
            if capture_mode == live::state::LiveCaptureMode::PushToTalk {
                let should_stop = interaction
                    .lock()
                    .expect("live shortcut state poisoned")
                    .finish_push_to_talk_start();
                if should_stop
                    && view.active_capture_mode == Some(live::state::LiveCaptureMode::PushToTalk)
                {
                    stop_live_from_app(&app);
                }
            }
        }
        live::hotkeys::LiveShortcutAction::Stop => {
            std::thread::spawn(move || {
                stop_live_from_app(&app);
            });
        }
    }
}

pub(crate) fn start_live_runtime(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    stt: &stt::dispatch::SttState,
    orchestrator: &runtime::RuntimeOrchestratorState,
    active_capture_mode: live::state::LiveCaptureMode,
) -> live::state::LiveSessionView {
    if live::state::is_live_session_started(live.snapshot().status) || live_runtime.is_active() {
        return live.snapshot();
    }

    if stt.is_transcribing() {
        let view = live.block_with_error(stt::error::SttError::Busy.user_message());
        if view.visibility == live::state::LiveOverlayVisibility::Enabled {
            if let Err(error) = live::overlay_window::ensure_active(&app) {
                stt::log_yap(&format!("live overlay busy show failed: {error}"));
            }
        }
        live::events::emit_session(&app, &view);
        return view;
    }

    let setup = runtime_policy::current_setup_status().runtime_setup_state();
    orchestrator.with(|orchestrator| orchestrator.set_setup(setup));
    if live::state::live_route_for(setup, false) == live::state::LiveRoute::Blocked {
        let view = block_for_setup(live, setup);
        if view.visibility == live::state::LiveOverlayVisibility::Enabled {
            if let Err(error) = live::overlay_window::ensure_active(&app) {
                stt::log_yap(&format!("live overlay blocked show failed: {error}"));
            }
        }
        live::events::emit_session(&app, &view);
        return view;
    }

    let requested_device_id = live.snapshot().input_device_id;
    let resolved = live::devices::resolve_input_device(requested_device_id.as_deref());
    let Some(view) = live.try_begin_local_start(
        active_capture_mode,
        requested_device_id.clone(),
        resolved.label.clone(),
    ) else {
        return live.snapshot();
    };

    if let Err(error) = orchestrator.with(|orchestrator| orchestrator.start_fallback()) {
        let view = live.block_with_error(&runtime_policy::runtime_error_to_stt(error).message);
        if view.visibility == live::state::LiveOverlayVisibility::Enabled {
            if let Err(error) = live::overlay_window::ensure_active(&app) {
                stt::log_yap(&format!("live overlay route error show failed: {error}"));
            }
        }
        live::events::emit_session(&app, &view);
        return view;
    }

    let view = if resolved.recovered {
        live.update(|view| {
            view.error = Some("Selected microphone unavailable. Using default.".into());
        })
    } else {
        view
    };
    if let Err(error) = live::overlay_window::ensure_active(&app) {
        stt::log_yap(&format!("live overlay start show failed: {error}"));
    }
    live::events::emit_session(&app, &view);

    match live_runtime.start_local(app.clone(), requested_device_id, active_capture_mode) {
        Ok(()) => live.snapshot(),
        Err(failure) => {
            let Some(message) = live_runtime.claim_start_failure(failure) else {
                return live.snapshot();
            };
            orchestrator.with(|orchestrator| orchestrator.finish_active_work());
            let view = live.block_with_error(&message);
            live::events::emit_session(&app, &view);
            view
        }
    }
}

fn block_for_setup(
    live: &live::LiveSessionState,
    setup: runtime::state::SetupState,
) -> live::state::LiveSessionView {
    live.start(setup, false)
}

pub(crate) fn stop_live_runtime(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    orchestrator: &runtime::RuntimeOrchestratorState,
) -> live::state::LiveSessionView {
    finalize_live_runtime(app, live, live_runtime, orchestrator, None, None)
}

pub(crate) fn stop_live_runtime_after_crash(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    orchestrator: &runtime::RuntimeOrchestratorState,
    session: u64,
    message: &str,
) -> live::state::LiveSessionView {
    finalize_live_runtime(
        app,
        live,
        live_runtime,
        orchestrator,
        Some(session),
        Some(message),
    )
}

fn finalize_live_runtime(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    orchestrator: &runtime::RuntimeOrchestratorState,
    expected_session: Option<u64>,
    completion_error: Option<&str>,
) -> live::state::LiveSessionView {
    if expected_session.is_some_and(|session| !live_runtime.is_session_current(session)) {
        return live.snapshot();
    }
    let Some(saving) = live.try_begin_saving(live_runtime.is_active()) else {
        if let Some(message) = completion_error {
            if let Some(view) = live.update_if_saving(|view| {
                view.transcription_degraded = true;
                view.error = Some(append_error(view.error.take(), message));
            }) {
                live::events::emit_session(&app, &view);
                return view;
            }
        }
        return live.snapshot();
    };
    let injection_target = live::injection::capture_target();

    live::events::emit_session(&app, &saving);
    let finish_status = live_runtime.stop_stream();
    if finish_status.should_report() {
        stt::log_yap(&format!(
            "live stream stop completed with {finish_status:?}"
        ));
    }
    let before_stop = if finish_status.should_report() {
        live.mark_transcription_degraded()
    } else {
        live.snapshot()
    };
    orchestrator.with(|orchestrator| orchestrator.finish_active_work());
    let effects = run_completion_effects_with(
        &before_stop,
        |text| live.remember_completed_transcript(text),
        |text| live::injection::inject_text(&app, injection_target, text),
        || {
            let stop = live_runtime.finish_stop(finish_status);
            live::recordings::save_stop_result(&stop, &before_stop)
        },
    );
    apply_injection_result(live, effects.injection);
    if let Some(message) = completion_error {
        live.update(|view| {
            view.transcription_degraded = true;
            view.error = Some(append_error(view.error.take(), message));
        });
    }
    match effects.save {
        Ok(Some(saved)) => live::events::emit_saved(&app, &saved),
        Ok(None) => {}
        Err(error) => {
            stt::log_yap(&format!("live save failed: {error}"));
            live.update(|view| {
                let save_error = "Couldn't save this recording to Home.";
                view.error = Some(append_error(view.error.take(), save_error));
            });
        }
    }
    let view = live.finish_saving();
    if view.error.is_some() {
        if let Err(error) = live::overlay_window::ensure_active(&app) {
            stt::log_yap(&format!("live overlay feedback show failed: {error}"));
        }
    } else if view.visibility == live::state::LiveOverlayVisibility::Enabled {
        if let Err(error) = live::overlay_window::ensure_idle(&app) {
            stt::log_yap(&format!("live overlay idle show failed: {error}"));
        }
    } else if let Some(window) = app.get_webview_window(live::overlay_window::WINDOW_LABEL) {
        let _ = window.hide();
    }
    live::events::emit_session(&app, &view);
    view
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::{
        apply_injection_result, block_for_setup, run_completion_effects_with,
        INJECTION_COPIED_ERROR,
    };
    use crate::live::{
        injection::InjectionOutcome,
        settings::LiveSettings,
        state::{LiveSessionState, LiveSessionView},
    };
    use crate::runtime::state::SetupState;

    #[test]
    fn start_live_setup_missing_blocks_without_claiming_server() {
        let live = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: None,
            capture_mode: crate::live::state::LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });

        let view = block_for_setup(&live, SetupState::FallbackMissing);

        assert_eq!(view.status, crate::live::state::LiveSessionStatus::Blocked);
        assert_eq!(view.route, crate::live::state::LiveRoute::Blocked);
        assert_eq!(view.error.as_deref(), Some("Local fallback is not ready."));
    }

    #[test]
    fn successful_retry_clears_only_injection_feedback() {
        let state = LiveSessionState::new(LiveSettings::default());
        state.update(|view| view.error = Some(INJECTION_COPIED_ERROR.into()));

        let recovered = apply_injection_result(&state, Ok(Some(InjectionOutcome::Injected)));
        assert_eq!(recovered.error, None);

        state.update(|view| view.error = Some("Live transcription stopped unexpectedly.".into()));
        let unrelated = apply_injection_result(&state, Ok(Some(InjectionOutcome::Injected)));
        assert_eq!(
            unrelated.error.as_deref(),
            Some("Live transcription stopped unexpectedly.")
        );

        state.update(|view| {
            view.error = Some(format!(
                "Live transcription stopped unexpectedly. {INJECTION_COPIED_ERROR}"
            ));
        });
        let combined = apply_injection_result(&state, Ok(Some(InjectionOutcome::Injected)));
        assert_eq!(
            combined.error.as_deref(),
            Some("Live transcription stopped unexpectedly.")
        );

        state.update(|view| view.error = Some("Couldn't save this recording to Home.".into()));
        let copied = apply_injection_result(
            &state,
            Ok(Some(InjectionOutcome::CopiedOnly("focus changed".into()))),
        );
        assert_eq!(
            copied.error.as_deref(),
            Some(
                "Couldn't save this recording to Home. Couldn't insert text here. Transcript copied; press Ctrl+V."
            )
        );
        let failed = apply_injection_result(&state, Err("clipboard busy".into()));
        assert_eq!(
            failed.error.as_deref(),
            Some("Couldn't save this recording to Home. Couldn't insert or copy this transcript.")
        );
    }

    #[test]
    fn completed_transcript_is_sent_to_the_injection_port() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.final_text = Some("  Thank you.  ".into());
        let injected = RefCell::new(Vec::new());

        let effects = run_completion_effects_with(
            &view,
            |_| {},
            |text| {
                injected.borrow_mut().push(text.to_string());
                Ok(())
            },
            || Ok(()),
        );

        assert_eq!(effects.injection, Ok(Some(())));
        assert_eq!(effects.save, Ok(()));
        assert_eq!(injected.into_inner(), vec!["Thank you.".to_string()]);
    }

    #[test]
    fn completion_effects_remember_and_inject_before_saving() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.final_text = Some("Finished words".into());
        let events = RefCell::new(Vec::<String>::new());

        let effects = run_completion_effects_with(
            &view,
            |text| events.borrow_mut().push(format!("remember:{text}")),
            |text| {
                events.borrow_mut().push(format!("inject:{text}"));
                Ok(())
            },
            || {
                events.borrow_mut().push("save".into());
                Ok(())
            },
        );

        assert_eq!(effects.injection, Ok(Some(())));
        assert_eq!(effects.save, Ok(()));
        assert_eq!(
            events.into_inner(),
            vec!["remember:Finished words", "inject:Finished words", "save"]
        );
    }

    #[test]
    fn only_the_saving_claim_holder_runs_completion_effects() {
        let state = LiveSessionState::new(LiveSettings::default());
        state
            .try_begin_local_start(crate::live::state::LiveCaptureMode::PushToTalk, None, None)
            .unwrap();
        state.try_begin_listening_from_armed().unwrap();
        state.update_final("Finished words");
        let first = state.try_begin_saving(true).unwrap();
        assert!(state.try_begin_saving(true).is_none());
        let effects = RefCell::new(Vec::new());

        let _ = run_completion_effects_with(
            &first,
            |_| effects.borrow_mut().push("remember"),
            |_| {
                effects.borrow_mut().push("inject");
                Ok(())
            },
            || {
                effects.borrow_mut().push("save");
                Ok(())
            },
        );

        assert_eq!(effects.into_inner(), vec!["remember", "inject", "save"]);
    }

    #[test]
    fn injection_failure_does_not_skip_save() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.final_text = Some("Finished words".into());
        let events = RefCell::new(Vec::<String>::new());

        let effects = run_completion_effects_with(
            &view,
            |_| {},
            |_| -> Result<(), String> {
                events.borrow_mut().push("inject".into());
                Err("blocked".into())
            },
            || {
                events.borrow_mut().push("save".into());
                Ok(())
            },
        );

        assert_eq!(effects.injection, Err("blocked".into()));
        assert_eq!(effects.save, Ok(()));
        assert_eq!(events.into_inner(), vec!["inject", "save"]);
    }

    #[test]
    fn empty_transcript_does_not_synthesize_input() {
        let view = LiveSessionView::from_settings(&LiveSettings::default());

        let effects = run_completion_effects_with(
            &view,
            |_| panic!("empty sessions must not update paste-last"),
            |_| -> Result<(), String> {
                panic!("empty sessions must not invoke the injection port")
            },
            || Ok(()),
        );

        assert_eq!(effects.injection, Ok(None));
        assert_eq!(effects.save, Ok(()));
    }

    #[test]
    fn partial_transcript_does_not_synthesize_input() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.partial_text = Some("not finalized".into());

        let effects = run_completion_effects_with(
            &view,
            |_| panic!("partial sessions must not update paste-last"),
            |_| -> Result<(), String> {
                panic!("partial sessions must not invoke the injection port")
            },
            || Ok(()),
        );

        assert_eq!(effects.injection, Ok(None));
        assert_eq!(effects.save, Ok(()));
    }
}
