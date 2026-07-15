use std::sync::Mutex;

use tauri::Manager;
use tauri_plugin_global_shortcut::Shortcut;

use crate::{authorization, live, runtime, runtime_policy, stt};

const INJECTION_COPIED_ERROR: &str = "Couldn't insert text here. Transcript copied; press Ctrl+V.";
const INJECTION_FAILED_ERROR: &str = "Couldn't insert or copy this transcript.";

pub(crate) struct QuitCoordinator {
    state: Mutex<QuitState>,
}

enum QuitState {
    Ready,
    Finalizing,
    Failed(String),
    ExitAuthorized,
}

#[derive(Debug, PartialEq, Eq)]
enum QuitClaim {
    Finalize,
    Coalesced,
    Blocked(String),
    ExitAuthorized,
}

impl QuitCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(QuitState::Ready),
        }
    }

    fn claim(&self) -> QuitClaim {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &*state {
            QuitState::Ready => {
                *state = QuitState::Finalizing;
                QuitClaim::Finalize
            }
            QuitState::Finalizing => QuitClaim::Coalesced,
            QuitState::Failed(error) => QuitClaim::Blocked(error.clone()),
            QuitState::ExitAuthorized => QuitClaim::ExitAuthorized,
        }
    }

    fn finish(&self, result: Result<(), String>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *state = match result {
            Ok(()) => QuitState::ExitAuthorized,
            Err(error) => QuitState::Failed(error),
        };
    }

    fn worker_start_failed(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(*state, QuitState::Finalizing) {
            *state = QuitState::Ready;
        }
    }

    pub(crate) fn exit_authorized(&self) -> bool {
        matches!(
            *self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            QuitState::ExitAuthorized
        )
    }
}

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

pub(crate) fn quit_from_app(app: &tauri::AppHandle) {
    let quit = app.state::<QuitCoordinator>();
    match quit.claim() {
        QuitClaim::Finalize => {}
        QuitClaim::Coalesced => return,
        QuitClaim::Blocked(error) => {
            stt::log_yap(&format!(
                "quit remains blocked by an unacknowledged save failure: {error}"
            ));
            present_quit_failure(app);
            return;
        }
        QuitClaim::ExitAuthorized => {
            app.exit(0);
            return;
        }
    }

    let worker_app = app.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("live-semantic-quit".into())
        .spawn(move || {
            let result = run_quit_with(
                || finalize_live_before_quit(&worker_app),
                || {
                    worker_app.state::<QuitCoordinator>().finish(Ok(()));
                    worker_app.exit(0);
                },
            );
            if let Err(error) = result {
                worker_app
                    .state::<QuitCoordinator>()
                    .finish(Err(error.clone()));
                stt::log_yap(&format!(
                    "quit deferred after live finalization failed: {error}"
                ));
                present_quit_failure(&worker_app);
            }
        })
    {
        app.state::<QuitCoordinator>().worker_start_failed();
        stt::log_yap(&format!("quit worker failed to start: {error}"));
        present_quit_failure(app);
    }
}

fn run_quit_with(
    finalize: impl FnOnce() -> Result<(), String>,
    exit: impl FnOnce(),
) -> Result<(), String> {
    finalize()?;
    exit();
    Ok(())
}

fn finalize_live_before_quit(app: &tauri::AppHandle) -> Result<(), String> {
    let live = app.state::<live::LiveSessionState>();
    let live_runtime = app.state::<live::runtime::LiveRuntime>();
    live_runtime.cancel_pending_start();
    let outcome = live_runtime.run_stop_lifecycle(|| {
        finalize_live_runtime_with_mode(
            app.clone(),
            &live,
            &live_runtime,
            None,
            None,
            CompletionMode::Quit,
        )
    });
    outcome.save_error.map_or(Ok(()), Err)
}

fn present_quit_failure(app: &tauri::AppHandle) {
    let live = app.state::<live::LiveSessionState>();
    let view = live.update(|view| {
        view.error = Some(append_error(
            view.error.take(),
            "Yap stayed open because the current recording could not be saved.",
        ));
    });
    show_main_window(app);
    if let Err(error) = live::overlay_window::ensure_active(app) {
        stt::log_yap(&format!("quit failure overlay show failed: {error}"));
    }
    live::events::emit_session(app, &view);
}

pub(crate) fn start_live_from_app(app: &tauri::AppHandle) {
    let live = app.state::<live::LiveSessionState>();
    let live_runtime = app.state::<live::runtime::LiveRuntime>();
    let stt = app.state::<stt::dispatch::SttState>();
    let capture_mode = live.snapshot().capture_mode;
    let _ = start_live_runtime(app.clone(), &live, &live_runtime, &stt, capture_mode);
}

pub(crate) fn stop_live_from_app(app: &tauri::AppHandle) {
    let live = app.state::<live::LiveSessionState>();
    let live_runtime = app.state::<live::runtime::LiveRuntime>();
    let _ = stop_live_runtime(app.clone(), &live, &live_runtime);
}

struct CompletionEffects<I, S> {
    injection: Result<Option<I>, String>,
    save: Result<S, String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CompletionMode {
    Normal,
    Quit,
}

struct FinalizationOutcome {
    view: live::state::LiveSessionView,
    save_error: Option<String>,
}

enum StartLifecycleResult {
    Complete(live::state::LiveSessionView),
    CaptureInstalled(live::runtime::LocalCaptureStart),
}

#[cfg(test)]
fn run_completion_effects_with<I, S>(
    view: &live::state::LiveSessionView,
    remember: impl FnOnce(&str),
    inject: impl FnOnce(&str) -> Result<I, String>,
    save: impl FnOnce() -> Result<S, String>,
) -> CompletionEffects<I, S> {
    run_completion_effects_with_mode(view, CompletionMode::Normal, remember, inject, save)
}

fn run_completion_effects_with_mode<I, S>(
    view: &live::state::LiveSessionView,
    mode: CompletionMode,
    remember: impl FnOnce(&str),
    inject: impl FnOnce(&str) -> Result<I, String>,
    save: impl FnOnce() -> Result<S, String>,
) -> CompletionEffects<I, S> {
    let text = live::recordings::completed_transcript_text(view);
    if let Some(text) = text.as_deref() {
        remember(text);
    }
    let injection = match (mode, text) {
        (CompletionMode::Normal, Some(text)) => inject(&text).map(Some),
        (CompletionMode::Normal | CompletionMode::Quit, None) | (CompletionMode::Quit, Some(_)) => {
            Ok(None)
        }
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
    if let Err(error) = live_runtime.request_warm(app.clone()) {
        stt::log_yap(&format!("live warmup skipped: {error}"));
    }
}

pub(crate) fn handle_live_shortcut_action(
    app: tauri::AppHandle,
    action: live::hotkeys::LiveShortcutAction,
) -> Option<live::state::LiveCaptureMode> {
    match action {
        live::hotkeys::LiveShortcutAction::None
        | live::hotkeys::LiveShortcutAction::ScheduleHold(_) => None,
        live::hotkeys::LiveShortcutAction::Start(capture_mode) => {
            let live = app.state::<live::LiveSessionState>();
            let live_runtime = app.state::<live::runtime::LiveRuntime>();
            let stt = app.state::<stt::dispatch::SttState>();
            let view = start_live_runtime(app.clone(), &live, &live_runtime, &stt, capture_mode);
            view.active_capture_mode
        }
        live::hotkeys::LiveShortcutAction::Stop => {
            stop_live_from_app(&app);
            None
        }
    }
}

pub(crate) fn start_live_runtime(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    stt: &stt::dispatch::SttState,
    active_capture_mode: live::state::LiveCaptureMode,
) -> live::state::LiveSessionView {
    let intent = live_runtime.capture_start_intent();
    let start_app = app.clone();
    let result = live_runtime.run_start_lifecycle(intent, || {
        start_live_runtime_serialized(
            start_app,
            live,
            live_runtime,
            stt,
            active_capture_mode,
            intent,
        )
    });
    match result {
        None => live.snapshot(),
        Some(StartLifecycleResult::Complete(view)) => view,
        Some(StartLifecycleResult::CaptureInstalled(start)) => {
            match live_runtime.complete_local_start(app.clone(), start, intent) {
                Ok(_) => live.snapshot(),
                Err(failure) => {
                    let Some(message) = live_runtime.claim_start_failure(failure) else {
                        return live.snapshot();
                    };
                    let _ = stop_live_runtime(app.clone(), live, live_runtime);
                    let view = live.update(|view| {
                        view.error = Some(append_error(view.error.take(), &message));
                        view.route = live::state::LiveRoute::Blocked;
                        view.status = live::state::LiveSessionStatus::Blocked;
                        view.active_capture_mode = None;
                    });
                    if let Err(error) = live::overlay_window::ensure_active(&app) {
                        stt::log_yap(&format!("live overlay model failure show failed: {error}"));
                    }
                    live::events::emit_session(&app, &view);
                    view
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn start_live_runtime_serialized(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    stt: &stt::dispatch::SttState,
    active_capture_mode: live::state::LiveCaptureMode,
    intent: live::runtime::StartIntent,
) -> StartLifecycleResult {
    if live::state::is_live_session_started(live.snapshot().status) || live_runtime.is_active() {
        return StartLifecycleResult::Complete(live.snapshot());
    }

    if stt.is_transcribing() {
        let view = live.block_with_error(stt::error::SttError::Busy.user_message());
        if view.visibility == live::state::LiveOverlayVisibility::Enabled {
            if let Err(error) = live::overlay_window::ensure_active(&app) {
                stt::log_yap(&format!("live overlay busy show failed: {error}"));
            }
        }
        live::events::emit_session(&app, &view);
        return StartLifecycleResult::Complete(view);
    }

    let setup = runtime_policy::current_setup_status().runtime_setup_state();
    if live::state::live_route_for(setup, false) == live::state::LiveRoute::Blocked {
        let view = block_for_setup(live, setup);
        if view.visibility == live::state::LiveOverlayVisibility::Enabled {
            if let Err(error) = live::overlay_window::ensure_active(&app) {
                stt::log_yap(&format!("live overlay blocked show failed: {error}"));
            }
        }
        live::events::emit_session(&app, &view);
        return StartLifecycleResult::Complete(view);
    }

    let requested_device_id = live.snapshot().input_device_id;
    let resolved = live::devices::resolve_input_device(requested_device_id.as_deref());
    let Some(armed) = live.try_begin_local_start(
        active_capture_mode,
        requested_device_id.clone(),
        resolved.label.clone(),
    ) else {
        return StartLifecycleResult::Complete(live.snapshot());
    };
    if let Err(error) = live::overlay_window::ensure_active(&app) {
        stt::log_yap(&format!("live overlay initializing show failed: {error}"));
    }
    live::events::emit_session(&app, &armed);

    match live_runtime.start_local_capture(
        app.clone(),
        requested_device_id,
        active_capture_mode,
        intent,
    ) {
        Ok(Some(start)) => {
            let view = if resolved.recovered
                && live::state::is_live_capture_active(live.snapshot().status)
            {
                live.update(|view| {
                    view.error = Some("Selected microphone unavailable. Using default.".into());
                })
            } else {
                live.snapshot()
            };
            if live::state::is_live_capture_active(view.status) {
                if let Err(error) = live::overlay_window::ensure_active(&app) {
                    stt::log_yap(&format!("live overlay start show failed: {error}"));
                }
                live::events::emit_session(&app, &view);
            }
            StartLifecycleResult::CaptureInstalled(start)
        }
        Ok(None) => StartLifecycleResult::Complete(live.snapshot()),
        Err(failure) => {
            let Some(message) = live_runtime.claim_start_failure(failure) else {
                return StartLifecycleResult::Complete(live.snapshot());
            };
            let view = live.block_with_error(&message);
            if let Err(error) = live::overlay_window::ensure_active(&app) {
                stt::log_yap(&format!("live overlay start failure show failed: {error}"));
            }
            live::events::emit_session(&app, &view);
            StartLifecycleResult::Complete(view)
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
) -> live::state::LiveSessionView {
    live_runtime.cancel_pending_start();
    live_runtime.run_stop_lifecycle(|| finalize_live_runtime(app, live, live_runtime, None, None))
}

pub(crate) fn stop_live_runtime_after_crash(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    session: u64,
    message: &str,
) -> live::state::LiveSessionView {
    live_runtime.run_stop_lifecycle(|| {
        finalize_live_runtime(app, live, live_runtime, Some(session), Some(message))
    })
}

fn finalize_live_runtime(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    expected_session: Option<u64>,
    completion_error: Option<&str>,
) -> live::state::LiveSessionView {
    finalize_live_runtime_with_mode(
        app,
        live,
        live_runtime,
        expected_session,
        completion_error,
        CompletionMode::Normal,
    )
    .view
}

#[allow(clippy::too_many_arguments)]
fn finalize_live_runtime_with_mode(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    expected_session: Option<u64>,
    completion_error: Option<&str>,
    mode: CompletionMode,
) -> FinalizationOutcome {
    if expected_session.is_some_and(|session| !live_runtime.is_session_current(session)) {
        return FinalizationOutcome {
            view: live.snapshot(),
            save_error: None,
        };
    }
    let Some(saving) = live.try_begin_saving(live_runtime.is_active()) else {
        if let Some(message) = completion_error {
            if let Some(view) = live.update_if_saving(|view| {
                view.transcription_degraded = true;
                view.error = Some(append_error(view.error.take(), message));
            }) {
                live::events::emit_session(&app, &view);
                return FinalizationOutcome {
                    view,
                    save_error: None,
                };
            }
        }
        return FinalizationOutcome {
            view: live.snapshot(),
            save_error: None,
        };
    };
    let injection_target = (mode == CompletionMode::Normal)
        .then(live::injection::capture_target)
        .flatten();

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
    let effects = run_completion_effects_with_mode(
        &before_stop,
        mode,
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
            return finish_live_finalization(app, live, Some(error));
        }
    }
    finish_live_finalization(app, live, None)
}

fn finish_live_finalization(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    save_error: Option<String>,
) -> FinalizationOutcome {
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
    FinalizationOutcome { view, save_error }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::{
        apply_injection_result, block_for_setup, run_completion_effects_with,
        run_completion_effects_with_mode, run_quit_with, CompletionMode, QuitClaim,
        QuitCoordinator, INJECTION_COPIED_ERROR,
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
    fn quit_completion_remembers_and_saves_without_injecting() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.final_text = Some("Finished words".into());
        let events = RefCell::new(Vec::<String>::new());

        let effects = run_completion_effects_with_mode(
            &view,
            CompletionMode::Quit,
            |text| events.borrow_mut().push(format!("remember:{text}")),
            |_| -> Result<(), String> {
                events.borrow_mut().push("inject".into());
                Ok(())
            },
            || {
                events.borrow_mut().push("save".into());
                Ok(())
            },
        );

        assert_eq!(effects.injection, Ok(None));
        assert_eq!(effects.save, Ok(()));
        assert_eq!(events.into_inner(), vec!["remember:Finished words", "save"]);
    }

    #[test]
    fn quit_does_not_exit_when_finalization_fails() {
        let events = RefCell::new(Vec::new());

        let result = run_quit_with(
            || {
                events.borrow_mut().push("finalize");
                Err("save failed".to_string())
            },
            || events.borrow_mut().push("exit"),
        );

        assert_eq!(result, Err("save failed".into()));
        assert_eq!(events.into_inner(), vec!["finalize"]);
    }

    #[test]
    fn quit_exits_only_after_successful_finalization() {
        let events = RefCell::new(Vec::new());

        let result = run_quit_with(
            || {
                events.borrow_mut().push("finalize");
                Ok(())
            },
            || events.borrow_mut().push("exit"),
        );

        assert_eq!(result, Ok(()));
        assert_eq!(events.into_inner(), vec!["finalize", "exit"]);
    }

    #[test]
    fn repeated_quit_coalesces_and_cannot_bypass_an_unacknowledged_save_failure() {
        let quit = QuitCoordinator::new();

        assert_eq!(quit.claim(), QuitClaim::Finalize);
        assert_eq!(quit.claim(), QuitClaim::Coalesced);
        quit.finish(Err("save failed".into()));

        assert_eq!(quit.claim(), QuitClaim::Blocked("save failed".to_string()));
        assert!(!quit.exit_authorized());
    }

    #[test]
    fn successful_quit_authorizes_only_the_semantic_exit_it_started() {
        let quit = QuitCoordinator::new();

        assert_eq!(quit.claim(), QuitClaim::Finalize);
        quit.finish(Ok(()));

        assert!(quit.exit_authorized());
        assert_eq!(quit.claim(), QuitClaim::ExitAuthorized);
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
