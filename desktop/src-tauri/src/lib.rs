use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

pub(crate) const MAIN_WINDOW_LABEL: &str = "main";

pub mod audio;
mod batch_recordings;
mod file_actions;
pub mod live;
mod paths;
pub mod runtime;
pub mod stt;
mod tray;

#[tauri::command]
fn polish_num_gpu(window: tauri::WebviewWindow) -> Result<u32, String> {
    ensure_main_command(&window)?;
    Ok(stt::settings::polish_num_gpu_layers())
}

#[tauri::command]
fn setup_status(
    window: tauri::WebviewWindow,
    _state: tauri::State<'_, stt::dispatch::SttState>,
) -> Result<SetupStatus, String> {
    ensure_main_command(&window)?;
    Ok(current_setup_status())
}

#[tauri::command]
fn server_connection_status(
    window: tauri::WebviewWindow,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
) -> Result<runtime::state::ServerConnectorState, String> {
    ensure_main_command(&window)?;
    Ok(runtime_state.with(|orchestrator| orchestrator.server()))
}

#[tauri::command]
fn fallback_model_status(
    window: tauri::WebviewWindow,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    ensure_main_stt_command(&window)?;
    Ok(stt::fallback_model::status(install_state.inner()))
}

#[tauri::command]
async fn fallback_model_install(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
    live_state: tauri::State<'_, live::LiveSessionState>,
    force: Option<bool>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    ensure_main_stt_command(&window)?;
    ensure_fallback_setup_idle(&live_state)?;
    stt::fallback_model::install(app, install_state.inner().clone(), force.unwrap_or(false)).await
}

#[tauri::command]
fn fallback_model_cancel_install(
    window: tauri::WebviewWindow,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    ensure_main_stt_command(&window)?;
    stt::fallback_model::cancel_install(install_state.inner())
}

#[tauri::command]
async fn fallback_model_verify(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    ensure_main_stt_command(&window)?;
    ensure_fallback_setup_idle(&live_state)?;
    stt::fallback_model::verify(app, install_state.inner().clone()).await
}

#[tauri::command]
fn fallback_model_remove(
    window: tauri::WebviewWindow,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    ensure_main_stt_command(&window)?;
    ensure_fallback_setup_idle(&live_state)?;
    stt::fallback_model::remove(install_state.inner())
}

#[tauri::command]
fn fallback_model_set_enabled(
    window: tauri::WebviewWindow,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
    live_state: tauri::State<'_, live::LiveSessionState>,
    enabled: bool,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    ensure_main_stt_command(&window)?;
    ensure_fallback_setup_idle(&live_state)?;
    stt::fallback_model::set_enabled(install_state.inner(), enabled)
}

#[tauri::command]
fn fallback_model_open_folder(
    window: tauri::WebviewWindow,
    _app: tauri::AppHandle,
) -> Result<(), stt::dispatch::SttCommandError> {
    ensure_main_stt_command(&window)?;
    stt::fallback_model::open_folder()
}

#[tauri::command]
fn list_local_compute_targets(
    window: tauri::WebviewWindow,
) -> Result<Vec<LocalComputeTargetView>, String> {
    ensure_main_command(&window)?;
    Ok(local_compute_targets())
}

#[tauri::command]
fn set_local_compute_target(
    window: tauri::WebviewWindow,
    live_state: tauri::State<'_, live::LiveSessionState>,
    target_id: String,
) -> Result<Vec<LocalComputeTargetView>, String> {
    ensure_main_command(&window)?;
    if live::state::is_live_session_started(live_state.snapshot().status) {
        return Err("Stop live before changing local compute.".into());
    }
    if !local_compute_targets()
        .iter()
        .any(|target| target.id == target_id)
    {
        return Err("Compute target unavailable.".into());
    }
    stt::settings::set_local_compute_target(&target_id)
        .map_err(|_| "Failed to save compute target.".to_string())?;
    Ok(local_compute_targets())
}

#[tauri::command]
fn live_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_or_overlay_command(&window)?;
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
async fn show_live_overlay(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_command(&window)?;
    warm_live_on_intent(&app, &live_runtime);
    let view = state.update(|view| view.visibility = live::state::LiveOverlayVisibility::Enabled);
    persist_live_view(&view)?;
    if view.status == live::state::LiveSessionStatus::Idle {
        live::overlay_window::ensure_idle(&app)?;
    } else {
        live::overlay_window::ensure_active(&app)?;
    }
    emit_live(&app, &view);
    Ok(view)
}

#[tauri::command]
fn hide_live_overlay(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_command(&window)?;
    if live::state::is_live_session_started(state.snapshot().status) {
        return Err("Stop live before hiding the pill.".into());
    }
    let view = state.update(|view| view.visibility = live::state::LiveOverlayVisibility::Hidden);
    persist_live_view(&view)?;
    if let Some(window) = app.get_webview_window(live::overlay_window::WINDOW_LABEL) {
        window
            .hide()
            .map_err(|err| format!("Failed to hide live overlay: {err}"))?;
    }
    emit_live(&app, &view);
    Ok(view)
}

#[tauri::command]
fn set_live_overlay_surface(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    surface: String,
    error_message: Option<String>,
) -> Result<(), String> {
    ensure_main_or_overlay_command(&window)?;
    let snapshot = state.snapshot();
    if snapshot.visibility == live::state::LiveOverlayVisibility::Hidden
        && !live::state::is_live_session_started(snapshot.status)
    {
        return Ok(());
    }
    let (width, height) = live::overlay_window::frame(&surface, error_message.as_deref());
    live::overlay_window::ensure_size(&app, width, height)
}

#[tauri::command]
async fn set_live_overlay_enabled(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
    enabled: bool,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_command(&window)?;
    if enabled {
        show_live_overlay(window, app, state, live_runtime).await
    } else {
        hide_live_overlay(window, app, state)
    }
}

#[tauri::command]
fn set_live_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    hotkey: String,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_command(&window)?;
    let next = live::hotkeys::parse_hotkey(&hotkey)?;
    let snapshot = state.snapshot();
    if live::actions::configured_hotkey_matches_shortcut(&snapshot.paste_hotkey, &next) {
        return Err("Dictation shortcut must differ from paste shortcut.".into());
    }
    let previous = snapshot.hotkey;
    if !previous.is_empty() {
        if let Ok(shortcut) = live::hotkeys::parse_hotkey(&previous) {
            let _ = app.global_shortcut().unregister(shortcut);
        }
    }
    if let Err(error) = app.global_shortcut().register(next) {
        if !previous.is_empty() {
            if let Ok(shortcut) = live::hotkeys::parse_hotkey(&previous) {
                let _ = app.global_shortcut().register(shortcut);
            }
        }
        return Err(format!("Shortcut is unavailable: {error}"));
    }
    let view = state.update(|view| view.hotkey = hotkey.trim().to_string());
    persist_live_view(&view)?;
    emit_live(&app, &view);
    Ok(view)
}

#[tauri::command]
fn clear_live_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_command(&window)?;
    let previous = state.snapshot().hotkey;
    if !previous.is_empty() {
        if let Ok(shortcut) = live::hotkeys::parse_hotkey(&previous) {
            let _ = app.global_shortcut().unregister(shortcut);
        }
    }
    let view = state.update(|view| view.hotkey.clear());
    persist_live_view(&view)?;
    emit_live(&app, &view);
    Ok(view)
}

#[tauri::command]
fn set_live_paste_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    hotkey: String,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_command(&window)?;
    let next = live::hotkeys::parse_hotkey(&hotkey)?;
    let snapshot = state.snapshot();
    if live::actions::configured_hotkey_matches_shortcut(&snapshot.hotkey, &next) {
        return Err("Paste shortcut must differ from dictation shortcut.".into());
    }
    let previous = snapshot.paste_hotkey;
    if !previous.is_empty() {
        if let Ok(shortcut) = live::hotkeys::parse_hotkey(&previous) {
            let _ = app.global_shortcut().unregister(shortcut);
        }
    }
    if let Err(error) = app.global_shortcut().register(next) {
        if !previous.is_empty() {
            if let Ok(shortcut) = live::hotkeys::parse_hotkey(&previous) {
                let _ = app.global_shortcut().register(shortcut);
            }
        }
        return Err(format!("Shortcut is unavailable: {error}"));
    }
    let view = state.update(|view| view.paste_hotkey = hotkey.trim().to_string());
    persist_live_view(&view)?;
    emit_live(&app, &view);
    Ok(view)
}

#[tauri::command]
fn clear_live_paste_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_command(&window)?;
    let previous = state.snapshot().paste_hotkey;
    if !previous.is_empty() {
        if let Ok(shortcut) = live::hotkeys::parse_hotkey(&previous) {
            let _ = app.global_shortcut().unregister(shortcut);
        }
    }
    let view = state.update(|view| view.paste_hotkey.clear());
    persist_live_view(&view)?;
    emit_live(&app, &view);
    Ok(view)
}

#[tauri::command]
fn set_live_capture_mode(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    capture_mode: live::state::LiveCaptureMode,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_command(&window)?;
    if live::state::is_live_session_started(state.snapshot().status) {
        return Err("Stop live before changing live mode.".into());
    }
    let view = state.update(|view| view.capture_mode = capture_mode);
    persist_live_view(&view)?;
    emit_live(&app, &view);
    Ok(view)
}

#[tauri::command]
fn list_input_devices(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<Vec<live::state::LiveInputDeviceView>, String> {
    ensure_main_command(&window)?;
    let view = state.snapshot();
    Ok(live::devices::list_input_devices(
        view.input_device_id.as_deref(),
    ))
}

#[tauri::command]
fn set_input_device(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    device_id: Option<String>,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_command(&window)?;
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
    persist_live_view(&view)?;
    emit_live(&app, &view);
    Ok(view)
}

#[tauri::command]
fn preflight_input_device(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_command(&window)?;
    let snapshot = state.snapshot();
    if live::state::is_live_session_started(snapshot.status) {
        return Ok(snapshot);
    }
    let selected = snapshot.input_device_id;
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
    emit_live(&app, &view);
    Ok(view)
}

#[tauri::command]
fn start_live_session(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
    stt_state: tauri::State<'_, stt::dispatch::SttState>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
    active_capture_mode: Option<live::state::LiveCaptureMode>,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_or_overlay_command(&window)?;
    warm_live_on_intent(&app, &live_runtime);
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
fn stop_live_session(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
) -> Result<live::state::LiveSessionView, String> {
    ensure_main_or_overlay_command(&window)?;
    Ok(live::actions::stop_live_runtime(
        app,
        &state,
        &live_runtime,
        &runtime_state,
    ))
}

#[tauri::command]
fn list_saved_live_sessions(
    window: tauri::WebviewWindow,
) -> Result<Vec<live::recordings::SavedLiveSession>, String> {
    file_actions::ensure_main_window(&window)?;
    live::recordings::list_session_files()
}

#[tauri::command]
fn show_main_workspace(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    workspace: String,
) -> Result<(), String> {
    ensure_main_or_overlay_command(&window)?;
    match workspace.as_str() {
        "home" | "transcribe" | "polish" => {
            live::actions::show_main_window(&app);
            let _ = app.emit("open-workspace", workspace);
            Ok(())
        }
        _ => Err("Unsupported workspace.".into()),
    }
}

fn current_setup_status() -> SetupStatus {
    let fallback_enabled = stt::settings::local_fallback_enabled();
    let model_installed = matches!(
        stt::nemotron::model_status(fallback_enabled).status,
        stt::nemotron::FallbackModelStatus::Ready | stt::nemotron::FallbackModelStatus::Disabled
    );
    let (setup_state, engine_ready, engine_status) =
        compose_engine_status(stt::nemotron::local_fallback_start_paths().map(|_| ()));
    log_line(&format!(
        "setup_status engine_ready={engine_ready} fallback_enabled={fallback_enabled} model=nemotron"
    ));

    SetupStatus {
        model: stt::nemotron::MODEL_LABEL.into(),
        root: stt::nemotron::root_dir().display().to_string(),
        engine_ready,
        engine_binary_status: "Built in".into(),
        model_installed,
        fallback_enabled,
        engine_status,
        setup_state,
    }
}

fn ensure_fallback_setup_idle(
    live_state: &live::LiveSessionState,
) -> Result<(), stt::dispatch::SttCommandError> {
    if live::state::is_live_session_started(live_state.snapshot().status) {
        return Err(live_setup_busy_error());
    }
    Ok(())
}

fn local_compute_targets() -> Vec<LocalComputeTargetView> {
    let selected_id = stt::settings::saved_compute_target().id();
    let mut targets = vec![
        LocalComputeTargetView {
            id: "auto".into(),
            label: "Auto (CPU)".into(),
            selected: selected_id == "auto",
        },
        LocalComputeTargetView {
            id: "cpu".into(),
            label: "CPU".into(),
            selected: selected_id == "cpu",
        },
    ];
    if !targets.iter().any(|target| target.selected) {
        if let Some(target) = targets.first_mut() {
            target.selected = true;
        }
    }
    targets
}

fn compose_engine_status(
    availability: Result<(), stt::error::SttError>,
) -> (runtime::state::SetupState, bool, String) {
    match availability {
        Ok(()) => (
            runtime::state::SetupState::FallbackReady,
            true,
            "Transcription engine ready".into(),
        ),
        Err(stt::error::SttError::FallbackDisabled) => (
            runtime::state::SetupState::FallbackDisabled,
            false,
            "Local fallback disabled".into(),
        ),
        Err(stt::error::SttError::ModelMissing) => (
            runtime::state::SetupState::FallbackMissing,
            false,
            "Local fallback model missing".into(),
        ),
        Err(stt::error::SttError::ModelCorrupt) => (
            runtime::state::SetupState::SetupError,
            false,
            stt::error::SttError::ModelCorrupt.user_message().into(),
        ),
        Err(_) => (
            runtime::state::SetupState::SetupError,
            false,
            "Local fallback needs attention.".into(),
        ),
    }
}

#[tauri::command]
fn start_transcribe(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, stt::dispatch::SttState>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
    paths: Vec<String>,
) -> Result<(), stt::dispatch::SttCommandError> {
    ensure_main_stt_command(&window)?;
    if paths.is_empty() {
        return Ok(());
    }
    let paths = batch_recordings::validate_recording_paths(&paths)?;
    if state.is_transcribing() {
        return Err(stt::dispatch::SttCommandError {
            code: stt::error::SttError::Busy.code().to_string(),
            message: stt::error::SttError::Busy.user_message().to_string(),
        });
    }

    let setup = current_setup_status();
    runtime_state
        .with(|orchestrator| {
            orchestrator.set_setup(setup.runtime_setup_state());
            orchestrator.route_recording(true)
        })
        .map_err(runtime_error_to_stt)?;
    log_line(&format!(
        "start_transcribe blocked count={} reason=server_batch_unwired",
        paths.len()
    ));
    Err(runtime_error_to_stt(
        runtime::RuntimeError::ServerUnavailable,
    ))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupStatus {
    model: String,
    root: String,
    engine_ready: bool,
    engine_binary_status: String,
    model_installed: bool,
    fallback_enabled: bool,
    engine_status: String,
    #[serde(skip_serializing)]
    setup_state: runtime::state::SetupState,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalComputeTargetView {
    id: String,
    label: String,
    selected: bool,
}

impl SetupStatus {
    fn runtime_setup_state(&self) -> runtime::state::SetupState {
        self.setup_state
    }
}

fn runtime_error_to_stt(error: runtime::RuntimeError) -> stt::dispatch::SttCommandError {
    let stt_error = match error {
        runtime::RuntimeError::FallbackDisabled => stt::error::SttError::FallbackDisabled,
        runtime::RuntimeError::RuntimeBusy => stt::error::SttError::Busy,
        runtime::RuntimeError::ServerUnavailable => stt::error::SttError::ServerUnavailable,
        runtime::RuntimeError::SetupUnavailable => stt::error::SttError::SidecarUnreachable,
        runtime::RuntimeError::SetupRequired => stt::error::SttError::ModelMissing,
    };
    stt_error.into()
}

fn live_setup_busy_error() -> stt::dispatch::SttCommandError {
    stt::dispatch::SttCommandError {
        code: stt::error::SttError::Busy.code().to_string(),
        message: "Stop live before changing local fallback.".into(),
    }
}

fn log_line(message: &str) {
    stt::log_yap(message);
}

fn is_main_command_window(label: &str) -> bool {
    label == MAIN_WINDOW_LABEL
}

fn is_main_or_overlay_command_window(label: &str) -> bool {
    is_main_command_window(label) || label == live::overlay_window::WINDOW_LABEL
}

fn forbidden_command_window_message() -> String {
    "Command is not available from this window.".into()
}

fn ensure_main_command(window: &tauri::WebviewWindow) -> Result<(), String> {
    is_main_command_window(window.label())
        .then_some(())
        .ok_or_else(forbidden_command_window_message)
}

fn ensure_main_or_overlay_command(window: &tauri::WebviewWindow) -> Result<(), String> {
    is_main_or_overlay_command_window(window.label())
        .then_some(())
        .ok_or_else(forbidden_command_window_message)
}

fn forbidden_stt_command_window() -> stt::dispatch::SttCommandError {
    stt::dispatch::SttCommandError {
        code: "UNAUTHORIZED_WINDOW".into(),
        message: forbidden_command_window_message(),
    }
}

fn ensure_main_stt_command(
    window: &tauri::WebviewWindow,
) -> Result<(), stt::dispatch::SttCommandError> {
    is_main_command_window(window.label())
        .then_some(())
        .ok_or_else(forbidden_stt_command_window)
}

fn persist_live_view(view: &live::state::LiveSessionView) -> Result<(), String> {
    live::settings::save(&live::settings::LiveSettings {
        overlay_enabled: view.visibility == live::state::LiveOverlayVisibility::Enabled,
        hotkey: (!view.hotkey.is_empty()).then(|| view.hotkey.clone()),
        paste_hotkey: (!view.paste_hotkey.is_empty()).then(|| view.paste_hotkey.clone()),
        capture_mode: view.capture_mode,
        input_device_id: view.input_device_id.clone(),
    })
}

fn emit_live(app: &tauri::AppHandle, view: &live::state::LiveSessionView) {
    let _ = app.emit("live-session", view);
}

fn emit_live_saved(app: &tauri::AppHandle, saved: &live::recordings::SavedLiveSession) {
    let _ = app.emit("live-session-saved", saved);
}

fn block_live_for_setup(
    live: &live::LiveSessionState,
    setup: runtime::state::SetupState,
) -> live::state::LiveSessionView {
    live.start(setup, false)
}

fn warm_live_on_intent(app: &tauri::AppHandle, live_runtime: &live::runtime::LiveRuntime) {
    let app = app.clone();
    let live_runtime = live_runtime.clone();
    std::thread::spawn(move || {
        if let Err(error) = live_runtime.warm(app) {
            log_line(&format!("live warmup skipped: {error}"));
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    std::panic::set_hook(Box::new(|panic| {
        log_line(&format!("panic: {panic}"));
    }));
    log_line("app start");

    let stt_state = stt::dispatch::SttState::new();
    let live_settings = live::settings::load();
    let live_shortcut = live_settings.hotkey.clone();
    let live_paste_shortcut = live_settings.paste_hotkey.clone();
    let runtime_state = runtime::RuntimeOrchestratorState::new();
    let live_runtime = live::runtime::LiveRuntime::new();
    let live_state = live::LiveSessionState::new(live_settings);
    let fallback_model_install_state = stt::fallback_model::FallbackModelInstallState::new();
    let live_runtime_for_monitor = live_runtime.clone();
    let live_runtime_for_exit = live_runtime.clone();
    let live_shortcut_interaction =
        Arc::new(Mutex::new(live::hotkeys::LiveShortcutInteraction::default()));

    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        live_runtime_for_monitor.unload_if_idle(std::time::Duration::from_secs(600));
    });

    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());

    #[cfg(feature = "wdio")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());

    builder
        .manage(stt_state)
        .manage(live_state)
        .manage(live_runtime)
        .manage(fallback_model_install_state)
        .manage(runtime_state)
        .setup(move |app| {
            let shortcut_interaction = Arc::clone(&live_shortcut_interaction);
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(move |app, shortcut, event| {
                        let snapshot = {
                            let live = app.state::<live::LiveSessionState>();
                            live.snapshot()
                        };
                        if live::actions::configured_hotkey_matches_shortcut(
                            &snapshot.paste_hotkey,
                            shortcut,
                        ) {
                            if event.state() == ShortcutState::Pressed {
                                live::actions::paste_last_live_transcript(app);
                            }
                            return;
                        }
                        if !live::actions::configured_hotkey_matches_shortcut(
                            &snapshot.hotkey,
                            shortcut,
                        ) {
                            return;
                        }
                        let action = {
                            let mut interaction = shortcut_interaction
                                .lock()
                                .expect("live shortcut state poisoned");
                            if snapshot.status == live::state::LiveSessionStatus::Saving {
                                interaction.reset();
                                return;
                            }
                            match event.state() {
                                ShortcutState::Pressed => interaction
                                    .pressed(Instant::now(), snapshot.active_capture_mode),
                                ShortcutState::Released => interaction
                                    .released(Instant::now(), snapshot.active_capture_mode),
                            }
                        };
                        live::actions::handle_live_shortcut_action(
                            app.clone(),
                            Arc::clone(&shortcut_interaction),
                            action,
                        );
                    })
                    .build(),
            )?;
            if let Some(hotkey) = live_shortcut.as_deref() {
                if let Ok(shortcut) = live::hotkeys::parse_hotkey(hotkey) {
                    if let Err(error) = app.handle().global_shortcut().register(shortcut) {
                        log_line(&format!("live hotkey unavailable: {error}"));
                        let live = app.state::<live::LiveSessionState>();
                        let view = live.update(|view| {
                            view.error = Some("Live shortcut is unavailable.".into());
                            view.route = live::state::LiveRoute::Blocked;
                            view.status = live::state::LiveSessionStatus::Blocked;
                        });
                        emit_live(app.handle(), &view);
                    }
                }
            }
            if let Some(hotkey) = live_paste_shortcut.as_deref() {
                if let Ok(shortcut) = live::hotkeys::parse_hotkey(hotkey) {
                    if let Err(error) = app.handle().global_shortcut().register(shortcut) {
                        log_line(&format!("live paste hotkey unavailable: {error}"));
                        let live = app.state::<live::LiveSessionState>();
                        let view = live.update(|view| {
                            view.error = Some("Paste shortcut is unavailable.".into());
                        });
                        emit_live(app.handle(), &view);
                    }
                }
            }
            tray::install(app.handle())?;
            {
                let app = app.handle().clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    live::overlay_window::recover(&app);
                });
            }
            let startup_live = app.state::<live::LiveSessionState>().snapshot();
            if startup_live.visibility == live::state::LiveOverlayVisibility::Enabled {
                let result = if startup_live.status == live::state::LiveSessionStatus::Idle {
                    live::overlay_window::ensure_idle(app.handle())
                } else {
                    live::overlay_window::ensure_active(app.handle())
                };
                if let Err(error) = result {
                    log_line(&format!("live overlay startup failed: {error}"));
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            setup_status,
            server_connection_status,
            fallback_model_status,
            fallback_model_install,
            fallback_model_cancel_install,
            fallback_model_verify,
            fallback_model_remove,
            fallback_model_set_enabled,
            fallback_model_open_folder,
            list_local_compute_targets,
            set_local_compute_target,
            live_status,
            show_live_overlay,
            hide_live_overlay,
            set_live_overlay_surface,
            set_live_overlay_enabled,
            set_live_hotkey,
            clear_live_hotkey,
            set_live_paste_hotkey,
            clear_live_paste_hotkey,
            set_live_capture_mode,
            list_input_devices,
            set_input_device,
            preflight_input_device,
            start_live_session,
            stop_live_session,
            list_saved_live_sessions,
            show_main_workspace,
            polish_num_gpu,
            start_transcribe,
            file_actions::allow_recording_playback_path,
            file_actions::restore_recording_playback_path,
            file_actions::read_text_file,
            file_actions::read_text_preview,
            file_actions::write_polished_text,
            file_actions::open_app_path,
            file_actions::reveal_app_path,
            file_actions::delete_history_entry_files
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(move |app_handle, event| match event {
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" => {
                api.prevent_close();
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } if label == live::overlay_window::WINDOW_LABEL => {
                api.prevent_close();
            }
            tauri::RunEvent::Exit => {
                live_runtime_for_exit.shutdown();
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_status_serializes_for_frontend() {
        let value = serde_json::to_value(SetupStatus {
            model: "model".into(),
            root: "root".into(),
            engine_ready: true,
            engine_binary_status: "Built in".into(),
            model_installed: true,
            fallback_enabled: true,
            engine_status: "Transcription engine ready".into(),
            setup_state: runtime::state::SetupState::FallbackReady,
        })
        .unwrap();

        assert_eq!(value["engineReady"], true);
        assert_eq!(value["engineBinaryStatus"], "Built in");
        assert_eq!(value["modelInstalled"], true);
        assert_eq!(value["fallbackEnabled"], true);
        assert_eq!(value["engineStatus"], "Transcription engine ready");
        assert!(value.get("python_ready").is_none());
    }

    #[test]
    fn server_state_serializes_for_frontend() {
        let value =
            serde_json::to_value(runtime::state::ServerConnectorState::SignInRequired).unwrap();

        assert_eq!(value, "sign_in_required");
    }

    #[test]
    fn disabled_status_wins() {
        assert_eq!(
            compose_engine_status(Err(stt::error::SttError::FallbackDisabled)),
            (
                runtime::state::SetupState::FallbackDisabled,
                false,
                "Local fallback disabled".into()
            )
        );
    }

    #[test]
    fn runtime_setup_state_preserves_model_failures() {
        let missing_model = SetupStatus {
            model: "model".into(),
            root: "root".into(),
            engine_ready: false,
            engine_binary_status: "Built in".into(),
            model_installed: false,
            fallback_enabled: true,
            engine_status: "Setup".into(),
            setup_state: runtime::state::SetupState::FallbackMissing,
        };

        assert_eq!(
            missing_model.runtime_setup_state(),
            runtime::state::SetupState::FallbackMissing
        );
    }

    #[test]
    fn corrupt_status_maps_to_setup_error() {
        assert_eq!(
            compose_engine_status(Err(stt::error::SttError::ModelCorrupt)),
            (
                runtime::state::SetupState::SetupError,
                false,
                stt::error::SttError::ModelCorrupt.user_message().into()
            )
        );
    }

    #[test]
    fn runtime_error_mapping_keeps_server_and_binary_errors_distinct() {
        assert_eq!(
            runtime_error_to_stt(runtime::RuntimeError::ServerUnavailable).code,
            stt::error::SttError::ServerUnavailable.code()
        );
        assert_eq!(
            runtime_error_to_stt(runtime::RuntimeError::SetupUnavailable).code,
            stt::error::SttError::SidecarUnreachable.code()
        );
        assert_eq!(
            runtime_error_to_stt(runtime::RuntimeError::SetupRequired).code,
            stt::error::SttError::ModelMissing.code()
        );
    }

    #[test]
    fn command_window_guards_keep_privileged_commands_main_only() {
        assert!(is_main_command_window("main"));
        assert!(!is_main_command_window(live::overlay_window::WINDOW_LABEL));
        assert!(!is_main_command_window("settings"));

        assert!(is_main_or_overlay_command_window("main"));
        assert!(is_main_or_overlay_command_window(
            live::overlay_window::WINDOW_LABEL
        ));
        assert!(!is_main_or_overlay_command_window("settings"));
    }

    #[test]
    fn unauthorized_stt_window_error_has_stable_code() {
        let error = forbidden_stt_command_window();

        assert_eq!(error.code, "UNAUTHORIZED_WINDOW");
        assert_eq!(error.message, "Command is not available from this window.");
    }

    #[test]
    fn start_live_setup_missing_blocks_without_claiming_server() {
        let live = live::LiveSessionState::new(live::settings::LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: None,
            capture_mode: live::state::LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });

        let view = block_live_for_setup(&live, runtime::state::SetupState::FallbackMissing);

        assert_eq!(view.status, live::state::LiveSessionStatus::Blocked);
        assert_eq!(view.route, live::state::LiveRoute::Blocked);
        assert_eq!(view.error.as_deref(), Some("Local fallback is not ready."));
    }
}
