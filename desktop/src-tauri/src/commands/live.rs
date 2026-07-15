use tauri::{Emitter, Manager};

use crate::{authorization, file_actions, live, runtime, stt};

#[tauri::command]
pub(super) fn live_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    Ok(state.update(|view| {
        let requested_id = view.input_device_id.clone();
        let resolved = live::devices::resolve_input_device(requested_id.as_deref());
        if requested_id.is_some() {
            view.input_device_id = resolved.id;
        }
        view.input_device_label = resolved.label;
        if resolved.recovered {
            view.error = Some("Selected microphone unavailable. Using default.".into());
        }
    }))
}

#[tauri::command]
pub(super) fn live_overlay_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveOverlayView, String> {
    authorization::ensure_live_overlay(&window)?;
    Ok(live::state::LiveOverlayView::from(&state.snapshot()))
}

#[tauri::command]
pub(super) async fn show_live_overlay(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    live::actions::warm_on_intent(&app, &live_runtime);
    let view = state.update(|view| view.visibility = live::state::LiveOverlayVisibility::Enabled);
    live::settings::save_view(&view)?;
    if view.status == live::state::LiveSessionStatus::Idle {
        live::overlay_window::ensure_idle(&app)?;
    } else {
        live::overlay_window::ensure_active(&app)?;
    }
    live::events::emit_session(&app, &view);
    Ok(view)
}

#[tauri::command]
pub(super) fn hide_live_overlay(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    if live::state::is_live_session_started(state.snapshot().status) {
        return Err("Stop live before hiding the pill.".into());
    }
    let view = state.update(|view| view.visibility = live::state::LiveOverlayVisibility::Hidden);
    live::settings::save_view(&view)?;
    if let Some(window) = app.get_webview_window(live::overlay_window::WINDOW_LABEL) {
        window
            .hide()
            .map_err(|err| format!("Failed to hide live overlay: {err}"))?;
    }
    live::events::emit_session(&app, &view);
    Ok(view)
}

#[tauri::command]
pub(super) fn set_live_overlay_surface(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    surface: String,
) -> Result<(), String> {
    authorization::ensure_live_overlay(&window)?;
    let snapshot = state.snapshot();
    if snapshot.visibility == live::state::LiveOverlayVisibility::Hidden
        && !live::state::is_live_session_started(snapshot.status)
    {
        return Ok(());
    }
    if !live_overlay_surface_matches_state(&snapshot, &surface) {
        return Err("Live overlay surface does not match native session state.".into());
    }
    live::overlay_window::ensure_surface(&app, &surface)
}

fn live_overlay_surface_matches_state(view: &live::state::LiveSessionView, surface: &str) -> bool {
    use live::state::LiveSessionStatus;

    match view.status {
        LiveSessionStatus::Idle => match surface {
            "collapsed" | "expanded" => view.error.is_none(),
            "success" => {
                view.error.is_none()
                    && view
                        .final_text
                        .as_deref()
                        .is_some_and(|text| !text.trim().is_empty())
            }
            "feedback" => view.error.is_some(),
            _ => false,
        },
        LiveSessionStatus::Armed => surface == "initializing",
        LiveSessionStatus::Listening | LiveSessionStatus::Speaking => surface == "recording",
        LiveSessionStatus::Settling | LiveSessionStatus::Saving => surface == "processing",
        LiveSessionStatus::Blocked => surface == "feedback",
    }
}

#[tauri::command]
pub(super) async fn set_live_overlay_enabled(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
    enabled: bool,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    if enabled {
        show_live_overlay(window, app, state, live_runtime).await
    } else {
        hide_live_overlay(window, app, state)
    }
}

#[tauri::command]
pub(super) fn set_live_capture_mode(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    capture_mode: live::state::LiveCaptureMode,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    if live::state::is_live_session_started(state.snapshot().status) {
        return Err("Stop live before changing live mode.".into());
    }
    let view = state.update(|view| view.capture_mode = capture_mode);
    live::settings::save_view(&view)?;
    live::events::emit_session(&app, &view);
    Ok(view)
}

#[tauri::command]
pub(super) fn list_input_devices(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<Vec<live::state::LiveInputDeviceView>, String> {
    authorization::ensure_main(&window)?;
    let view = state.snapshot();
    live::devices::list_input_devices(view.input_device_id.as_deref())
}

#[tauri::command]
pub(super) fn set_input_device(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    device_id: Option<String>,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    if live::state::is_live_session_started(state.snapshot().status) {
        return Err("Stop live before changing microphones.".into());
    }
    let resolved = live::devices::resolve_input_device(device_id.as_deref());
    let recovered = resolved.recovered;
    let view = state.update(|view| {
        view.input_device_id = device_id.clone();
        view.input_device_label = resolved.label;
        view.error = recovered.then(|| "Selected microphone unavailable. Using default.".into());
    });
    live::settings::save_view(&view)?;
    live::events::emit_session(&app, &view);
    Ok(view)
}

#[tauri::command]
pub(super) fn preflight_input_device(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    let snapshot = state.snapshot();
    if live::state::is_live_session_started(snapshot.status) {
        return Ok(snapshot);
    }
    let selected = snapshot.input_device_id;
    state.clear_startup_shortcut_failure(false);
    let view = match live::devices::preflight_input_device(selected.as_deref()) {
        Ok(resolved) => state.update(|view| {
            view.input_device_id = selected.clone();
            view.input_device_label = resolved.label;
            view.level = Some(0.0);
            view.error = resolved
                .recovered
                .then(|| "Selected microphone unavailable. Using default.".into());
            view.route = live::state::LiveRoute::None;
            view.status = live::state::LiveSessionStatus::Idle;
        }),
        Err(message) => state.update(|view| {
            view.error = Some(message);
            view.level = Some(0.0);
            view.route = live::state::LiveRoute::Blocked;
            view.status = live::state::LiveSessionStatus::Blocked;
        }),
    };
    live::events::emit_session(&app, &view);
    Ok(view)
}

#[tauri::command]
pub(super) fn start_live_session(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
    stt_state: tauri::State<'_, stt::dispatch::SttState>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
    active_capture_mode: Option<live::state::LiveCaptureMode>,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    live::actions::warm_on_intent(&app, &live_runtime);
    let capture_mode = active_capture_mode.unwrap_or_else(|| state.snapshot().capture_mode);
    Ok(live::actions::start_live_runtime(
        app,
        &state,
        &live_runtime,
        &stt_state,
        &runtime_state,
        capture_mode,
    ))
}

#[tauri::command]
pub(super) fn start_live_overlay_session(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
    stt_state: tauri::State<'_, stt::dispatch::SttState>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
    active_capture_mode: Option<live::state::LiveCaptureMode>,
) -> Result<live::state::LiveOverlayView, String> {
    authorization::ensure_live_overlay(&window)?;
    live::actions::warm_on_intent(&app, &live_runtime);
    let capture_mode = active_capture_mode.unwrap_or_else(|| state.snapshot().capture_mode);
    let view = live::actions::start_live_runtime(
        app,
        &state,
        &live_runtime,
        &stt_state,
        &runtime_state,
        capture_mode,
    );
    Ok(live::state::LiveOverlayView::from(&view))
}

#[tauri::command]
pub(super) fn stop_live_session(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    Ok(live::actions::stop_live_runtime(
        app,
        &state,
        &live_runtime,
        &runtime_state,
    ))
}

#[tauri::command]
pub(super) fn stop_live_overlay_session(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
) -> Result<live::state::LiveOverlayView, String> {
    authorization::ensure_live_overlay(&window)?;
    let view = live::actions::stop_live_runtime(app, &state, &live_runtime, &runtime_state);
    Ok(live::state::LiveOverlayView::from(&view))
}

#[tauri::command]
pub(super) fn recover_live_session(
    window: tauri::WebviewWindow,
    session_id: String,
    expected_artifact_path: String,
) -> Result<live::recordings::SavedLiveSession, String> {
    file_actions::ensure_main_window(&window)?;
    live::recordings::recover_live_session(session_id, expected_artifact_path)
}

#[tauri::command]
pub(super) fn delete_recoverable_live_session(
    window: tauri::WebviewWindow,
    session_id: String,
    expected_artifact_path: String,
) -> Result<(), String> {
    file_actions::ensure_main_window(&window)?;
    live::recordings::delete_recoverable_live_session(session_id, expected_artifact_path)
}

#[tauri::command]
pub(super) fn delete_saved_live_session(
    window: tauri::WebviewWindow,
    session_id: String,
    expected_output_path: String,
    expected_capture_commit_path: String,
) -> Result<(), String> {
    file_actions::ensure_main_window(&window)?;
    live::recordings::delete_saved_live_session(
        session_id,
        expected_output_path,
        expected_capture_commit_path,
    )
}

#[tauri::command]
pub(super) fn show_main_workspace(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    workspace: String,
) -> Result<(), String> {
    authorization::ensure_main_or_overlay(&window)?;
    match workspace.as_str() {
        "home" | "transcribe" | "polish" => {
            live::actions::show_main_window(&app);
            let _ = app.emit_to(
                authorization::MAIN_WINDOW_LABEL,
                "open-workspace",
                workspace,
            );
            Ok(())
        }
        _ => Err("Unsupported workspace.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(status: live::state::LiveSessionStatus) -> live::state::LiveSessionView {
        let mut view = live::state::LiveSessionView::from_settings(&live::settings::LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: Some("Ctrl+Shift+Alt+V".into()),
            capture_mode: live::state::LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });
        view.status = status;
        view
    }

    #[test]
    fn active_native_sessions_reject_hidden_or_idle_surfaces() {
        use live::state::LiveSessionStatus;

        for (status, allowed) in [
            (LiveSessionStatus::Armed, "initializing"),
            (LiveSessionStatus::Listening, "recording"),
            (LiveSessionStatus::Speaking, "recording"),
            (LiveSessionStatus::Settling, "processing"),
            (LiveSessionStatus::Saving, "processing"),
        ] {
            let view = view(status);
            assert!(live_overlay_surface_matches_state(&view, allowed));
            for forbidden in ["collapsed", "expanded", "success", "feedback"] {
                assert!(
                    !live_overlay_surface_matches_state(&view, forbidden),
                    "{status:?} accepted {forbidden}"
                );
            }
        }
    }

    #[test]
    fn idle_and_blocked_surfaces_remain_bound_to_native_state() {
        use live::state::LiveSessionStatus;

        let idle = view(LiveSessionStatus::Idle);
        assert!(live_overlay_surface_matches_state(&idle, "collapsed"));
        assert!(live_overlay_surface_matches_state(&idle, "expanded"));
        assert!(!live_overlay_surface_matches_state(&idle, "success"));
        assert!(!live_overlay_surface_matches_state(&idle, "feedback"));

        let mut success = idle.clone();
        success.final_text = Some("saved".into());
        assert!(live_overlay_surface_matches_state(&success, "success"));

        let mut feedback = idle;
        feedback.error = Some("microphone unavailable".into());
        assert!(live_overlay_surface_matches_state(&feedback, "feedback"));
        assert!(!live_overlay_surface_matches_state(&feedback, "collapsed"));

        let blocked = view(LiveSessionStatus::Blocked);
        assert!(live_overlay_surface_matches_state(&blocked, "feedback"));
        assert!(!live_overlay_surface_matches_state(&blocked, "collapsed"));
    }
}
