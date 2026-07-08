use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

const LIVE_OVERLAY_COMPACT_WIDTH: f64 = 112.0;
const LIVE_OVERLAY_COMPACT_HEIGHT: f64 = 40.0;
const LIVE_OVERLAY_DEFAULT_WIDTH: f64 = 104.0;
const LIVE_OVERLAY_SUCCESS_WIDTH: f64 = 168.0;
const LIVE_OVERLAY_HOVER_SENSOR_WIDTH: f64 = 260.0;
const LIVE_OVERLAY_HOVER_SENSOR_HEIGHT: f64 = 8.0;
const LIVE_OVERLAY_MIN_ERROR_WIDTH: f64 = 180.0;
const LIVE_OVERLAY_MAX_ERROR_WIDTH: f64 = 420.0;
const LIVE_OVERLAY_TOP_BEZEL_OFFSET: f64 = 0.0;
const TRAY_SHOW_APP: &str = "show_app";
const TRAY_START_DICTATING: &str = "start_dictating";
const TRAY_STOP_RECORDING: &str = "stop_recording";
const TRAY_QUIT: &str = "quit";

mod file_actions;
pub mod live;
pub mod runtime;
pub mod stt;

#[tauri::command]
fn polish_num_gpu() -> u32 {
    stt::settings::polish_num_gpu_layers()
}

#[tauri::command]
fn setup_status(_state: tauri::State<'_, stt::dispatch::SttState>) -> SetupStatus {
    current_setup_status()
}

#[tauri::command]
fn fallback_model_status(
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    Ok(stt::fallback_model::status(install_state.inner()))
}

#[tauri::command]
async fn fallback_model_install(
    app: tauri::AppHandle,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
    live_state: tauri::State<'_, live::LiveSessionState>,
    force: Option<bool>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    ensure_fallback_setup_idle(&live_state)?;
    stt::fallback_model::install(app, install_state.inner().clone(), force.unwrap_or(false)).await
}

#[tauri::command]
fn fallback_model_cancel_install(
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    stt::fallback_model::cancel_install(install_state.inner())
}

#[tauri::command]
fn fallback_model_verify(
    app: tauri::AppHandle,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    ensure_fallback_setup_idle(&live_state)?;
    stt::fallback_model::verify(app, install_state.inner().clone())
}

#[tauri::command]
fn fallback_model_remove(
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    ensure_fallback_setup_idle(&live_state)?;
    stt::fallback_model::remove()
}

#[tauri::command]
fn fallback_model_set_enabled(
    live_state: tauri::State<'_, live::LiveSessionState>,
    enabled: bool,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    ensure_fallback_setup_idle(&live_state)?;
    stt::fallback_model::set_enabled(enabled)
}

#[tauri::command]
fn fallback_model_open_folder(
    _app: tauri::AppHandle,
) -> Result<(), stt::dispatch::SttCommandError> {
    stt::fallback_model::open_folder()
}

#[tauri::command]
fn list_local_compute_targets() -> Vec<LocalComputeTargetView> {
    local_compute_targets()
}

#[tauri::command]
fn set_local_compute_target(
    live_state: tauri::State<'_, live::LiveSessionState>,
    target_id: String,
) -> Result<Vec<LocalComputeTargetView>, String> {
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
fn live_status(state: tauri::State<'_, live::LiveSessionState>) -> live::state::LiveSessionView {
    state.update(|view| {
        let requested_id = view.input_device_id.clone();
        let resolved = live::devices::resolve_input_device(requested_id.as_deref());
        if requested_id.is_some() {
            view.input_device_id = resolved.id;
        }
        view.input_device_label = resolved.label;
        if resolved.recovered {
            view.error = Some("Selected microphone unavailable. Using default.".into());
        }
    })
}

#[tauri::command]
async fn show_live_overlay(
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    let view = state.update(|view| view.visibility = live::state::LiveOverlayVisibility::Enabled);
    persist_live_view(&view)?;
    if view.status == live::state::LiveSessionStatus::Idle {
        ensure_idle_live_overlay(&app)?;
    } else {
        ensure_live_overlay(&app)?;
    }
    emit_live(&app, &view);
    Ok(view)
}

#[tauri::command]
fn hide_live_overlay(
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    if live::state::is_live_session_started(state.snapshot().status) {
        return Err("Stop live before hiding the pill.".into());
    }
    let view = state.update(|view| view.visibility = live::state::LiveOverlayVisibility::Hidden);
    persist_live_view(&view)?;
    if let Some(window) = app.get_webview_window("live-overlay") {
        window
            .hide()
            .map_err(|err| format!("Failed to hide live overlay: {err}"))?;
    }
    emit_live(&app, &view);
    Ok(view)
}

#[tauri::command]
fn set_live_overlay_surface(
    app: tauri::AppHandle,
    surface: String,
    error_message: Option<String>,
) -> Result<(), String> {
    let (width, height) = live_overlay_frame(&surface, error_message.as_deref());
    ensure_live_overlay_size(&app, width, height)
}

#[tauri::command]
async fn set_live_overlay_enabled(
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    enabled: bool,
) -> Result<live::state::LiveSessionView, String> {
    if enabled {
        show_live_overlay(app, state).await
    } else {
        hide_live_overlay(app, state)
    }
}

#[tauri::command]
fn set_live_hotkey(
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    hotkey: String,
) -> Result<live::state::LiveSessionView, String> {
    let next = live::hotkeys::parse_hotkey(&hotkey)?;
    let previous = state.snapshot().hotkey;
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
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
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
fn set_live_capture_mode(
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    capture_mode: live::state::LiveCaptureMode,
) -> Result<live::state::LiveSessionView, String> {
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
    state: tauri::State<'_, live::LiveSessionState>,
) -> Vec<live::state::LiveInputDeviceView> {
    let view = state.snapshot();
    live::devices::list_input_devices(view.input_device_id.as_deref())
}

#[tauri::command]
fn set_input_device(
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    device_id: Option<String>,
) -> Result<live::state::LiveSessionView, String> {
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
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> live::state::LiveSessionView {
    let snapshot = state.snapshot();
    if live::state::is_live_session_started(snapshot.status) {
        return snapshot;
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
    view
}

#[tauri::command]
fn start_live_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
    stt_state: tauri::State<'_, stt::dispatch::SttState>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
    active_capture_mode: Option<live::state::LiveCaptureMode>,
) -> live::state::LiveSessionView {
    let capture_mode = active_capture_mode.unwrap_or_else(|| state.snapshot().capture_mode);
    start_live_runtime(
        app,
        &state,
        &live_runtime,
        &stt_state,
        &runtime_state,
        capture_mode,
    )
}

#[tauri::command]
fn stop_live_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
) -> live::state::LiveSessionView {
    stop_live_runtime(app, &state, &live_runtime, &runtime_state)
}

#[tauri::command]
fn list_saved_live_sessions(
    window: tauri::WebviewWindow,
) -> Result<Vec<live::recordings::SavedLiveSession>, String> {
    file_actions::ensure_main_window(&window)?;
    live::recordings::list_session_files()
}

#[tauri::command]
fn show_main_workspace(app: tauri::AppHandle, workspace: String) -> Result<(), String> {
    match workspace.as_str() {
        "home" | "transcribe" | "polish" => {
            show_main_window(&app);
            let _ = app.emit("open-workspace", workspace);
            Ok(())
        }
        _ => Err("Unsupported workspace.".into()),
    }
}

#[tauri::command]
async fn install_local_fallback(
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<SetupStatus, stt::dispatch::SttCommandError> {
    ensure_fallback_setup_idle(&live_state)?;
    tauri::async_runtime::spawn_blocking(|| {
        let previous_enabled = stt::settings::local_fallback_enabled();
        stt::settings::set_local_fallback_enabled(true)?;
        match stt::nemotron::local_fallback_start_paths() {
            Ok(_) => Ok(current_setup_status()),
            Err(error) => {
                let _ = stt::settings::set_local_fallback_enabled(previous_enabled);
                Err(stt::dispatch::SttCommandError::from(error))
            }
        }
    })
    .await
    .map_err(|_| stt::dispatch::SttCommandError::from(stt::error::SttError::SidecarCrash))?
}

#[tauri::command]
fn remove_local_fallback(
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<SetupStatus, stt::dispatch::SttCommandError> {
    ensure_fallback_setup_idle(&live_state)?;
    remove_local_fallback_files()?;
    stt::settings::set_local_fallback_enabled(false)?;
    Ok(current_setup_status())
}

#[tauri::command]
fn set_local_fallback_enabled(
    live_state: tauri::State<'_, live::LiveSessionState>,
    enabled: bool,
) -> Result<SetupStatus, stt::dispatch::SttCommandError> {
    ensure_fallback_setup_idle(&live_state)?;
    stt::settings::set_local_fallback_enabled(enabled)?;
    Ok(current_setup_status())
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

fn remove_local_fallback_files() -> Result<(), stt::dispatch::SttCommandError> {
    stt::nemotron::remove_model().map_err(stt::dispatch::SttCommandError::from)
}

#[tauri::command]
fn start_transcribe(
    state: tauri::State<'_, stt::dispatch::SttState>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
    paths: Vec<String>,
) -> Result<(), stt::dispatch::SttCommandError> {
    if paths.is_empty() {
        return Ok(());
    }
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

fn persist_live_view(view: &live::state::LiveSessionView) -> Result<(), String> {
    live::settings::save(&live::settings::LiveSettings {
        overlay_enabled: view.visibility == live::state::LiveOverlayVisibility::Enabled,
        hotkey: (!view.hotkey.is_empty()).then(|| view.hotkey.clone()),
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

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn start_live_from_app(app: &tauri::AppHandle) {
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

fn stop_live_from_app(app: &tauri::AppHandle) {
    let live = app.state::<live::LiveSessionState>();
    let live_runtime = app.state::<live::runtime::LiveRuntime>();
    let orchestrator = app.state::<runtime::RuntimeOrchestratorState>();
    let _ = stop_live_runtime(app.clone(), &live, &live_runtime, &orchestrator);
}

fn handle_live_shortcut_action(
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

fn install_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let menu = MenuBuilder::new(app)
        .text(TRAY_SHOW_APP, "Show Yap")
        .text(TRAY_START_DICTATING, "Start Dictating")
        .text(TRAY_STOP_RECORDING, "Stop Recording")
        .separator()
        .text(TRAY_QUIT, "Quit")
        .build()?;

    let mut tray = TrayIconBuilder::with_id("yap")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Yap")
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_SHOW_APP => show_main_window(app),
            TRAY_START_DICTATING => start_live_from_app(app),
            TRAY_STOP_RECORDING => stop_live_from_app(app),
            TRAY_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }

    tray.build(app)?;
    Ok(())
}

fn start_live_runtime(
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
            if let Err(error) = ensure_live_overlay(&app) {
                log_line(&format!("live overlay busy show failed: {error}"));
            }
        }
        emit_live(&app, &view);
        return view;
    }

    let setup = current_setup_status().runtime_setup_state();
    orchestrator.with(|orchestrator| orchestrator.set_setup(setup));
    if live::state::live_route_for(setup, false) == live::state::LiveRoute::Blocked {
        let view = block_live_for_setup(live, setup);
        if view.visibility == live::state::LiveOverlayVisibility::Enabled {
            if let Err(error) = ensure_live_overlay(&app) {
                log_line(&format!("live overlay blocked show failed: {error}"));
            }
        }
        emit_live(&app, &view);
        return view;
    }

    if let Err(error) = orchestrator.with(|orchestrator| orchestrator.start_fallback()) {
        let view = live.block_with_error(&runtime_error_to_stt(error).message);
        if view.visibility == live::state::LiveOverlayVisibility::Enabled {
            if let Err(error) = ensure_live_overlay(&app) {
                log_line(&format!("live overlay route error show failed: {error}"));
            }
        }
        emit_live(&app, &view);
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
    if let Err(error) = ensure_live_overlay(&app) {
        log_line(&format!("live overlay start show failed: {error}"));
    }
    emit_live(&app, &view);

    match live_runtime.start_local(app.clone(), requested_device_id) {
        Ok(()) => live.snapshot(),
        Err(message) => {
            orchestrator.with(|orchestrator| orchestrator.finish_active_work());
            let view = live.block_with_error(&message);
            emit_live(&app, &view);
            view
        }
    }
}

fn stop_live_runtime(
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
    emit_live(&app, &saving);
    live_runtime.stop();
    let before_stop = live.snapshot();
    orchestrator.with(|orchestrator| orchestrator.finish_active_work());
    let view = live.stop();
    match live::recordings::save_session_files(live_runtime, &before_stop) {
        Ok(Some(saved)) => emit_live_saved(&app, &saved),
        Ok(None) => {}
        Err(error) => log_line(&format!("live save failed: {error}")),
    }
    if view.visibility == live::state::LiveOverlayVisibility::Enabled {
        if let Err(error) = ensure_idle_live_overlay(&app) {
            log_line(&format!("live overlay idle show failed: {error}"));
        }
    } else if let Some(window) = app.get_webview_window("live-overlay") {
        let _ = window.hide();
    }
    emit_live(&app, &view);
    view
}

fn block_live_for_setup(
    live: &live::LiveSessionState,
    setup: runtime::state::SetupState,
) -> live::state::LiveSessionView {
    live.start(setup, false)
}

fn ensure_live_overlay(app: &tauri::AppHandle) -> Result<(), String> {
    ensure_live_overlay_size(app, LIVE_OVERLAY_COMPACT_WIDTH, LIVE_OVERLAY_COMPACT_HEIGHT)
}

fn ensure_idle_live_overlay(app: &tauri::AppHandle) -> Result<(), String> {
    ensure_live_overlay_size(
        app,
        LIVE_OVERLAY_HOVER_SENSOR_WIDTH,
        LIVE_OVERLAY_HOVER_SENSOR_HEIGHT,
    )
}

fn live_overlay_frame(surface: &str, error_message: Option<&str>) -> (f64, f64) {
    let width = match surface {
        "sensor" | "peek" => LIVE_OVERLAY_HOVER_SENSOR_WIDTH,
        "recording" | "processing" | "initializing" => LIVE_OVERLAY_COMPACT_WIDTH,
        "success" => LIVE_OVERLAY_SUCCESS_WIDTH,
        "feedback" => error_message.map_or(LIVE_OVERLAY_DEFAULT_WIDTH, |message| {
            (message.len() as f64 * 6.8 + 74.0)
                .clamp(LIVE_OVERLAY_MIN_ERROR_WIDTH, LIVE_OVERLAY_MAX_ERROR_WIDTH)
        }),
        _ => LIVE_OVERLAY_DEFAULT_WIDTH,
    };
    let height = if surface == "sensor" {
        LIVE_OVERLAY_HOVER_SENSOR_HEIGHT
    } else {
        LIVE_OVERLAY_COMPACT_HEIGHT
    };
    (width, height)
}

fn ensure_live_overlay_size(app: &tauri::AppHandle, width: f64, height: f64) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("live-overlay") {
        window
            .set_size(tauri::LogicalSize::new(width, height))
            .map_err(|err| format!("Failed to size live overlay: {err}"))?;
        window
            .set_shadow(false)
            .map_err(|err| format!("Failed to hide live overlay shadow: {err}"))?;
        window
            .set_skip_taskbar(true)
            .map_err(|err| format!("Failed to hide live overlay from taskbar: {err}"))?;
        window
            .set_closable(false)
            .map_err(|err| format!("Failed to lock live overlay close control: {err}"))?;
        window
            .set_focusable(false)
            .map_err(|err| format!("Failed to keep live overlay unfocusable: {err}"))?;
        make_live_overlay_system_window(&window)?;
        position_live_overlay(app, &window, width)?;
        window
            .show()
            .map_err(|err| format!("Failed to show live overlay: {err}"))?;
        return Ok(());
    }

    let (x, y) = live_overlay_position(app, width);
    let window = tauri::WebviewWindowBuilder::new(
        app,
        "live-overlay",
        tauri::WebviewUrl::App("index.html?window=live-overlay".into()),
    )
    .title("Yap Live")
    .inner_size(width, height)
    .position(x, y)
    .decorations(false)
    .resizable(false)
    .closable(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .focusable(false)
    .build()
    .map_err(|err| format!("Failed to create live overlay: {err}"))?;
    window
        .set_focusable(false)
        .map_err(|err| format!("Failed to keep live overlay unfocusable: {err}"))?;
    make_live_overlay_system_window(&window)?;
    position_live_overlay(app, &window, width)?;
    Ok(())
}

fn position_live_overlay(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    width: f64,
) -> Result<(), String> {
    let (x, y) = live_overlay_position(app, width);
    window
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|err| format!("Failed to position live overlay: {err}"))
}

fn live_overlay_position(app: &tauri::AppHandle, width: f64) -> (f64, f64) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let position = monitor.position().to_logical::<f64>(scale);
        let size = monitor.size().to_logical::<f64>(scale);
        return (
            position.x + ((size.width - width) / 2.0).max(0.0),
            position.y + LIVE_OVERLAY_TOP_BEZEL_OFFSET,
        );
    }
    (8.0, LIVE_OVERLAY_TOP_BEZEL_OFFSET)
}

#[cfg(target_os = "windows")]
fn make_live_overlay_system_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    let hwnd = window
        .hwnd()
        .map_err(|err| format!("Failed to read live overlay window handle: {err}"))?;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let next_style = (style | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) & !WS_EX_APPWINDOW.0;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next_style as isize);
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .map_err(|err| format!("Failed to refresh live overlay window style: {err}"))?;
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn make_live_overlay_system_window(_window: &tauri::WebviewWindow) -> Result<(), String> {
    Ok(())
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
                    .with_handler(move |app, _shortcut, event| {
                        let snapshot = {
                            let live = app.state::<live::LiveSessionState>();
                            live.snapshot()
                        };
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
                        handle_live_shortcut_action(
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
            install_tray(app.handle())?;
            let startup_live = app.state::<live::LiveSessionState>().snapshot();
            if startup_live.visibility == live::state::LiveOverlayVisibility::Enabled {
                let result = if startup_live.status == live::state::LiveSessionStatus::Idle {
                    ensure_idle_live_overlay(app.handle())
                } else {
                    ensure_live_overlay(app.handle())
                };
                if let Err(error) = result {
                    log_line(&format!("live overlay startup failed: {error}"));
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            setup_status,
            fallback_model_status,
            fallback_model_install,
            fallback_model_cancel_install,
            fallback_model_verify,
            fallback_model_remove,
            fallback_model_set_enabled,
            fallback_model_open_folder,
            list_local_compute_targets,
            set_local_compute_target,
            install_local_fallback,
            remove_local_fallback,
            set_local_fallback_enabled,
            live_status,
            show_live_overlay,
            hide_live_overlay,
            set_live_overlay_surface,
            set_live_overlay_enabled,
            set_live_hotkey,
            clear_live_hotkey,
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
            file_actions::read_text_file,
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
            } if label == "live-overlay" => {
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
    fn start_live_setup_missing_blocks_without_claiming_server() {
        let live = live::LiveSessionState::new(live::settings::LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            capture_mode: live::state::LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });

        let view = block_live_for_setup(&live, runtime::state::SetupState::FallbackMissing);

        assert_eq!(view.status, live::state::LiveSessionStatus::Blocked);
        assert_eq!(view.route, live::state::LiveRoute::Blocked);
        assert_eq!(view.error.as_deref(), Some("Local fallback is not ready."));
    }
}
