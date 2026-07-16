use tauri::Manager;
use tauri_plugin_global_shortcut::Shortcut;

use crate::{live, runtime, runtime_policy, stt};

use super::{
    completion::append_error,
    stop::{stop_live_from_app, stop_live_runtime},
};

pub(crate) fn start_live_from_app(app: &tauri::AppHandle) {
    let live = app.state::<live::LiveSessionState>();
    let live_runtime = app.state::<live::runtime::LiveRuntime>();
    let stt = app.state::<stt::dispatch::SttState>();
    let capture_mode = live.snapshot().capture_mode;
    let _ = start_live_runtime(app.clone(), &live, &live_runtime, &stt, capture_mode);
}

enum StartLifecycleResult {
    Complete(live::state::LiveSessionView),
    CaptureInstalled(live::runtime::LocalCaptureStart),
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

pub(super) fn block_for_setup(
    live: &live::LiveSessionState,
    setup: runtime::state::SetupState,
) -> live::state::LiveSessionView {
    live.start(setup, false)
}
