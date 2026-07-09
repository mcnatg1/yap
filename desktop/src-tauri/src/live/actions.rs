use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tauri::Manager;
use tauri_plugin_global_shortcut::Shortcut;

use crate::{live, runtime, stt};

pub(crate) fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(crate::MAIN_WINDOW_LABEL) {
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

pub(crate) fn paste_last_live_transcript(app: &tauri::AppHandle) {
    let live = app.state::<live::LiveSessionState>();
    if let Some(text) = live::recordings::transcript_text(&live.snapshot()) {
        paste_live_text(&text);
    }
}

fn paste_live_text(text: &str) {
    if let Err(error) = live::paste::paste_text(text) {
        crate::log_line(&format!("live paste failed: {error}"));
    }
}

pub(crate) fn configured_hotkey_matches_shortcut(configured: &str, shortcut: &Shortcut) -> bool {
    !configured.trim().is_empty()
        && live::hotkeys::parse_hotkey(configured)
            .map(|configured| configured == *shortcut)
            .unwrap_or(false)
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
                crate::log_line(&format!("live overlay busy show failed: {error}"));
            }
        }
        crate::emit_live(&app, &view);
        return view;
    }

    let setup = crate::current_setup_status().runtime_setup_state();
    orchestrator.with(|orchestrator| orchestrator.set_setup(setup));
    if live::state::live_route_for(setup, false) == live::state::LiveRoute::Blocked {
        let view = crate::block_live_for_setup(live, setup);
        if view.visibility == live::state::LiveOverlayVisibility::Enabled {
            if let Err(error) = live::overlay_window::ensure_active(&app) {
                crate::log_line(&format!("live overlay blocked show failed: {error}"));
            }
        }
        crate::emit_live(&app, &view);
        return view;
    }

    if let Err(error) = orchestrator.with(|orchestrator| orchestrator.start_fallback()) {
        let view = live.block_with_error(&crate::runtime_error_to_stt(error).message);
        if view.visibility == live::state::LiveOverlayVisibility::Enabled {
            if let Err(error) = live::overlay_window::ensure_active(&app) {
                crate::log_line(&format!("live overlay route error show failed: {error}"));
            }
        }
        crate::emit_live(&app, &view);
        return view;
    }

    let requested_device_id = live.snapshot().input_device_id;
    let resolved = live::devices::resolve_input_device(requested_device_id.as_deref());

    let view = live.update(|view| {
        view.error = resolved
            .recovered
            .then(|| "Selected microphone unavailable. Using default.".into());
        view.input_device_id = requested_device_id.clone();
        view.input_device_label = resolved.label.clone();
        view.level = Some(0.0);
        view.route = live::state::LiveRoute::LocalFallback;
        view.status = live::state::LiveSessionStatus::Armed;
        view.active_capture_mode = Some(active_capture_mode);
    });
    if let Err(error) = live::overlay_window::ensure_active(&app) {
        crate::log_line(&format!("live overlay start show failed: {error}"));
    }
    crate::emit_live(&app, &view);

    match live_runtime.start_local(app.clone(), requested_device_id) {
        Ok(()) => live.snapshot(),
        Err(message) => {
            orchestrator.with(|orchestrator| orchestrator.finish_active_work());
            let view = live.block_with_error(&message);
            crate::emit_live(&app, &view);
            view
        }
    }
}

pub(crate) fn stop_live_runtime(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    orchestrator: &runtime::RuntimeOrchestratorState,
) -> live::state::LiveSessionView {
    let snapshot = live.snapshot();
    if snapshot.status == live::state::LiveSessionStatus::Saving
        || (!live::state::is_live_session_started(snapshot.status) && !live_runtime.is_active())
    {
        return snapshot;
    }

    let saving = live.begin_saving();
    crate::emit_live(&app, &saving);
    let finish_status = live_runtime.stop();
    if finish_status.should_report() {
        crate::log_line(&format!(
            "live stream stop completed with {finish_status:?}"
        ));
    }
    let before_stop = if finish_status.should_report() {
        live.mark_transcription_degraded()
    } else {
        live.snapshot()
    };
    orchestrator.with(|orchestrator| orchestrator.finish_active_work());
    let view = live.stop();
    let transcript = live::recordings::transcript_text(&before_stop);
    match live::recordings::save_session_files(live_runtime, &before_stop) {
        Ok(Some(saved)) => crate::emit_live_saved(&app, &saved),
        Ok(None) => {}
        Err(error) => crate::log_line(&format!("live save failed: {error}")),
    }
    if let Some(text) = transcript {
        paste_live_text(&text);
    }
    if view.visibility == live::state::LiveOverlayVisibility::Enabled {
        if let Err(error) = live::overlay_window::ensure_idle(&app) {
            crate::log_line(&format!("live overlay idle show failed: {error}"));
        }
    } else if let Some(window) = app.get_webview_window(live::overlay_window::WINDOW_LABEL) {
        let _ = window.hide();
    }
    crate::emit_live(&app, &view);
    view
}
