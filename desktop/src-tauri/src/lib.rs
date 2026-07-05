use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

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
fn live_status(state: tauri::State<'_, live::LiveSessionState>) -> live::state::LiveSessionView {
    state.update(|view| {
        let resolved = live::devices::resolve_input_device(view.input_device_id.as_deref());
        view.input_device_id = resolved.id;
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
    ensure_live_overlay(&app)?;
    emit_live(&app, &view);
    Ok(view)
}

#[tauri::command]
fn hide_live_overlay(
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    let view = state.update(|view| view.visibility = live::state::LiveOverlayVisibility::Hidden);
    persist_live_view(&view)?;
    if let Some(window) = app.get_webview_window("live-overlay") {
        window.hide().map_err(|err| format!("Failed to hide live overlay: {err}"))?;
    }
    emit_live(&app, &view);
    Ok(view)
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
fn get_live_hotkey(state: tauri::State<'_, live::LiveSessionState>) -> String {
    state.snapshot().hotkey
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
    let resolved = live::devices::resolve_input_device(device_id.as_deref());
    let recovered = resolved.recovered;
    let view = state.update(|view| {
        view.input_device_id = if device_id.is_none() { None } else { resolved.id };
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
    let selected = state.snapshot().input_device_id;
    let view = match live::devices::preflight_input_device(selected.as_deref()) {
        Ok(resolved) => state.update(|view| {
            view.input_device_id = resolved.id;
            view.input_device_label = resolved.label;
            view.level = Some(0.0);
            if live::state::is_live_session_started(view.status) {
                view.error = resolved.recovered.then(|| "Selected microphone unavailable. Using default.".into());
            } else {
                view.error = resolved.recovered.then(|| "Selected microphone unavailable. Using default.".into());
                view.route = live::state::LiveRoute::None;
                view.status = live::state::LiveSessionStatus::Idle;
            }
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
) -> live::state::LiveSessionView {
    start_live_runtime(app, &state, &live_runtime, &stt_state, &runtime_state)
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
fn save_live_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> live::state::LiveSessionView {
    let view = state.update(|view| {
        view.error = Some("Live audio saving is not implemented yet.".into());
    });
    emit_live(&app, &view);
    view
}

#[tauri::command]
async fn install_local_fallback() -> Result<SetupStatus, stt::dispatch::SttCommandError> {
    tauri::async_runtime::spawn_blocking(|| {
        stt::settings::set_local_fallback_enabled(true)?;
        stt::binary::ensure_binary()?;
        stt::model::ensure_model()?;
        Ok(current_setup_status())
    })
    .await
    .map_err(|_| stt::dispatch::SttCommandError::from(stt::error::SttError::SidecarCrash))?
}

#[tauri::command]
fn remove_local_fallback(
    state: tauri::State<'_, stt::dispatch::SttState>,
) -> Result<SetupStatus, stt::dispatch::SttCommandError> {
    if let Ok(mut sidecar) = state.sidecar.lock() {
        sidecar.shutdown();
    }
    remove_local_fallback_files()?;
    Ok(current_setup_status())
}

#[tauri::command]
fn set_local_fallback_enabled(
    enabled: bool,
) -> Result<SetupStatus, stt::dispatch::SttCommandError> {
    stt::settings::set_local_fallback_enabled(enabled)?;
    Ok(current_setup_status())
}

fn current_setup_status() -> SetupStatus {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let binary_status = stt::binary::binary_install_status(&exe_dir)
        .unwrap_or(stt::binary::BinaryInstallStatus::Unsupported);
    let pin = stt::pin::load_pin().ok();
    let model_installed = pin
        .as_ref()
        .map(stt::model::is_installed)
        .unwrap_or(false);
    let fallback_enabled = stt::settings::local_fallback_enabled();
    let engine_ready = fallback_enabled
        && pin.is_some()
        && model_installed
        && matches!(binary_status, stt::binary::BinaryInstallStatus::Installed);
    log_line(&format!(
        "setup_status engine_ready={engine_ready} fallback_enabled={fallback_enabled} binary={binary_status:?}"
    ));

    SetupStatus {
        model: pin
            .as_ref()
            .map(|pin| pin.gguf_file.clone())
            .unwrap_or_else(|| "moonshine-streaming-tiny-q4_k.gguf".into()),
        root: stt::model::models_dir().display().to_string(),
        engine_ready,
        engine_binary_status: binary_status.label().to_string(),
        model_installed,
        fallback_enabled,
        engine_status: compose_engine_status(binary_status, model_installed, fallback_enabled),
    }
}

fn compose_engine_status(
    binary_status: stt::binary::BinaryInstallStatus,
    model_installed: bool,
    fallback_enabled: bool,
) -> String {
    if !fallback_enabled {
        return "Local fallback disabled".into();
    }
    match binary_status {
        stt::binary::BinaryInstallStatus::Installed if model_installed => {
            "Transcription engine ready".to_string()
        }
        stt::binary::BinaryInstallStatus::Installed => "Local fallback model missing".into(),
        stt::binary::BinaryInstallStatus::Downloadable => "Local fallback not installed".into(),
        stt::binary::BinaryInstallStatus::Invalid => "Local fallback failed verification".into(),
        stt::binary::BinaryInstallStatus::Unsupported => {
            "Transcription engine requires manual install".into()
        }
    }
}

fn remove_local_fallback_files() -> Result<(), stt::dispatch::SttCommandError> {
    let pin = stt::pin::load_pin().map_err(|_| stt::error::SttError::ModelCorrupt)?;
    remove_if_exists(stt::binary::cached_binary_path(&pin.crispasr_version))?;
    remove_if_exists(stt::model::models_dir().join(&pin.gguf_file))?;
    remove_if_exists(stt::model::models_dir().join(&pin.tokenizer_file))?;
    remove_if_exists(stt::model::models_dir().join(&pin.punc_file))?;
    Ok(())
}

fn remove_if_exists(path: std::path::PathBuf) -> Result<(), stt::dispatch::SttCommandError> {
    remove_one(path.clone())?;
    let verified = path.with_extension("verified");
    remove_one(verified)
}

fn remove_one(path: std::path::PathBuf) -> Result<(), stt::dispatch::SttCommandError> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(path)
        .map_err(|_| stt::dispatch::SttCommandError::from(stt::error::SttError::ModelMissing))
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
    log_line(&format!("start_transcribe blocked count={} reason=server_batch_unwired", paths.len()));
    Err(runtime_error_to_stt(runtime::RuntimeError::ServerUnavailable))
}

#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);

    if !is_transcript_path(&path) {
        return Err("Only transcript text files can be read.".into());
    }

    std::fs::read_to_string(&path).map_err(|err| format!("Failed to read transcript: {err}"))
}

#[tauri::command]
fn write_polished_text(path: String, text: String) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);

    if !is_transcript_path(&path) {
        return Err("Only transcript text files can be polished.".into());
    }

    let output = polished_path(&path)?;
    std::fs::write(&output, text)
        .map_err(|err| format!("Failed to save polished transcript: {err}"))?;
    Ok(output.display().to_string())
}

fn polished_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| "Transcript path has no file name.".to_string())?;

    Ok(path.with_file_name(format!("{stem}.polished.txt")))
}

#[tauri::command]
fn open_app_path(path: String) -> Result<(), String> {
    let path = openable_app_path(path)?;
    tauri_plugin_opener::open_path(&path, None::<&str>)
        .map_err(|err| format!("Failed to open file: {err}"))
}

#[tauri::command]
fn reveal_app_path(path: String) -> Result<(), String> {
    let path = openable_app_path(path)?;
    tauri_plugin_opener::reveal_item_in_dir(path)
        .map_err(|err| format!("Failed to reveal file: {err}"))
}

fn openable_app_path(path: String) -> Result<std::path::PathBuf, String> {
    let path = std::path::PathBuf::from(path);
    if !is_yap_media_or_transcript_path(&path) {
        return Err("Only Yap recording and transcript files can be opened.".into());
    }
    if !path.exists() {
        return Err("File no longer exists.".into());
    }
    Ok(path)
}

fn is_transcript_path(path: &std::path::Path) -> bool {
    has_extension(path, &["txt"])
}

fn is_yap_media_or_transcript_path(path: &std::path::Path) -> bool {
    has_extension(path, &["txt", "mp3", "m4a", "wav", "mp4", "flac", "ogg", "webm"])
}

fn has_extension(path: &std::path::Path, allowed: &[&str]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| allowed.iter().any(|allowed| extension.eq_ignore_ascii_case(allowed)))
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
}

impl SetupStatus {
    fn runtime_setup_state(&self) -> runtime::state::SetupState {
        if !self.fallback_enabled {
            return runtime::state::SetupState::FallbackDisabled;
        }
        if self.engine_ready && self.model_installed {
            return runtime::state::SetupState::FallbackReady;
        }
        if self.engine_binary_status != stt::binary::BinaryInstallStatus::Installed.label() {
            return runtime::state::SetupState::SetupError;
        }
        runtime::state::SetupState::FallbackMissing
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

fn start_live_runtime(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    stt: &stt::dispatch::SttState,
    orchestrator: &runtime::RuntimeOrchestratorState,
) -> live::state::LiveSessionView {
    if stt.is_transcribing() {
        let view = live.block_with_error(stt::error::SttError::Busy.user_message());
        emit_live(&app, &view);
        return view;
    }

    let setup = current_setup_status().runtime_setup_state();
    orchestrator.with(|orchestrator| orchestrator.set_setup(setup));
    if live::state::live_route_for(setup, false) == live::state::LiveRoute::Blocked {
        let view = block_live_for_setup(live, setup);
        emit_live(&app, &view);
        return view;
    }

    if let Err(error) = orchestrator.with(|orchestrator| orchestrator.start_fallback()) {
        let view = live.block_with_error(&runtime_error_to_stt(error).message);
        emit_live(&app, &view);
        return view;
    }

    let resolved = match live::devices::preflight_input_device(live.snapshot().input_device_id.as_deref()) {
        Ok(resolved) => resolved,
        Err(message) => {
            orchestrator.with(|orchestrator| orchestrator.finish_active_work());
            let view = live.block_with_error(&message);
            emit_live(&app, &view);
            return view;
        }
    };

    let view = live.update(|view| {
        view.error = resolved
            .recovered
            .then(|| "Selected microphone unavailable. Using default.".into());
        view.input_device_id = resolved.id.clone();
        view.input_device_label = resolved.label.clone();
        view.level = Some(0.0);
        view.route = live::state::LiveRoute::LocalFallback;
        view.status = live::state::LiveSessionStatus::Armed;
    });
    emit_live(&app, &view);

    match live_runtime.start_local(app.clone(), resolved.id) {
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
    live_runtime.stop();
    orchestrator.with(|orchestrator| orchestrator.finish_active_work());
    let view = live.stop();
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
    if let Some(window) = app.get_webview_window("live-overlay") {
        position_live_overlay(app, &window)?;
        window.show().map_err(|err| format!("Failed to show live overlay: {err}"))?;
        return Ok(());
    }

    let (x, y) = live_overlay_position(app, 420.0);
    let window = tauri::WebviewWindowBuilder::new(
        app,
        "live-overlay",
        tauri::WebviewUrl::App("index.html?window=live-overlay".into()),
    )
    .title("Yap Live")
    .inner_size(420.0, 110.0)
    .position(x, y)
    .decorations(false)
    .resizable(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .build()
    .map_err(|err| format!("Failed to create live overlay: {err}"))?;
    position_live_overlay(app, &window)?;
    Ok(())
}

fn position_live_overlay(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
) -> Result<(), String> {
    let (x, y) = live_overlay_position(app, 420.0);
    window
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|err| format!("Failed to position live overlay: {err}"))
}

fn live_overlay_position(app: &tauri::AppHandle, width: f64) -> (f64, f64) {
    let monitor = app
        .get_webview_window("main")
        .and_then(|window| window.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let position = monitor.position().to_logical::<f64>(scale);
        let size = monitor.size().to_logical::<f64>(scale);
        return (position.x + ((size.width - width) / 2.0).max(8.0), position.y + 8.0);
    }
    (8.0, 8.0)
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
    let sidecar_for_monitor = std::sync::Arc::clone(&stt_state.sidecar);
    let sidecar_for_exit = std::sync::Arc::clone(&stt_state.sidecar);
    let transcribing_for_monitor = stt_state.transcribing_flag();
    let live_runtime_for_monitor = live_runtime.clone();
    let live_runtime_for_exit = live_runtime.clone();

    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        live_runtime_for_monitor.unload_if_idle(std::time::Duration::from_secs(600));
        if transcribing_for_monitor.load(std::sync::atomic::Ordering::Relaxed) {
            continue;
        }
        if let Ok(mut sidecar) = sidecar_for_monitor.lock() {
            sidecar.unload_if_idle();
        }
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(stt_state)
        .manage(live_state)
        .manage(live_runtime)
        .manage(runtime_state)
        .setup(move |app| {
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(|app, _shortcut, event| {
                        let live = app.state::<live::LiveSessionState>();
                        let live_runtime = app.state::<live::runtime::LiveRuntime>();
                        let stt = app.state::<stt::dispatch::SttState>();
                        let orchestrator = app.state::<runtime::RuntimeOrchestratorState>();
                        match event.state() {
                            ShortcutState::Pressed => {
                                let view = if live.snapshot().capture_mode == live::state::LiveCaptureMode::Toggle
                                    && live::state::is_live_session_started(live.snapshot().status)
                                {
                                    stop_live_runtime(app.clone(), &live, &live_runtime, &orchestrator)
                                } else {
                                    start_live_runtime(
                                        app.clone(),
                                        &live,
                                        &live_runtime,
                                        &stt,
                                        &orchestrator,
                                    )
                                };
                                let _ = view;
                            }
                            ShortcutState::Released => {
                                if live.snapshot().capture_mode == live::state::LiveCaptureMode::PushToTalk {
                                    let _ = stop_live_runtime(
                                        app.clone(),
                                        &live,
                                        &live_runtime,
                                        &orchestrator,
                                    );
                                }
                            }
                        }
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
            if app.state::<live::LiveSessionState>().snapshot().visibility
                == live::state::LiveOverlayVisibility::Enabled
            {
                if let Err(error) = ensure_live_overlay(app.handle()) {
                    log_line(&format!("live overlay startup failed: {error}"));
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            setup_status,
            install_local_fallback,
            remove_local_fallback,
            set_local_fallback_enabled,
            live_status,
            show_live_overlay,
            hide_live_overlay,
            set_live_overlay_enabled,
            get_live_hotkey,
            set_live_hotkey,
            clear_live_hotkey,
            set_live_capture_mode,
            list_input_devices,
            set_input_device,
            preflight_input_device,
            start_live_session,
            stop_live_session,
            save_live_session,
            polish_num_gpu,
            start_transcribe,
            read_text_file,
            write_polished_text,
            open_app_path,
            reveal_app_path
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(move |_app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                live_runtime_for_exit.shutdown();
                if let Ok(mut sidecar) = sidecar_for_exit.lock() {
                    sidecar.shutdown();
                }
            }
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
            engine_binary_status: "Installed".into(),
            model_installed: true,
            fallback_enabled: true,
            engine_status: "Transcription engine ready".into(),
        })
        .unwrap();

        assert_eq!(value["engineReady"], true);
        assert_eq!(value["engineBinaryStatus"], "Installed");
        assert_eq!(value["modelInstalled"], true);
        assert_eq!(value["fallbackEnabled"], true);
        assert_eq!(value["engineStatus"], "Transcription engine ready");
        assert!(value.get("python_ready").is_none());
    }

    #[test]
    fn disabled_status_wins() {
        assert_eq!(
            compose_engine_status(stt::binary::BinaryInstallStatus::Installed, true, false),
            "Local fallback disabled"
        );
    }

    #[test]
    fn runtime_setup_state_preserves_binary_and_model_failures() {
        let missing_binary = SetupStatus {
            model: "model".into(),
            root: "root".into(),
            engine_ready: false,
            engine_binary_status: stt::binary::BinaryInstallStatus::Downloadable
                .label()
                .into(),
            model_installed: true,
            fallback_enabled: true,
            engine_status: "Setup".into(),
        };
        let missing_model = SetupStatus {
            model: "model".into(),
            root: "root".into(),
            engine_ready: false,
            engine_binary_status: stt::binary::BinaryInstallStatus::Installed.label().into(),
            model_installed: false,
            fallback_enabled: true,
            engine_status: "Setup".into(),
        };

        assert_eq!(
            missing_binary.runtime_setup_state(),
            runtime::state::SetupState::SetupError
        );
        assert_eq!(
            missing_model.runtime_setup_state(),
            runtime::state::SetupState::FallbackMissing
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

    #[test]
    fn read_text_file_rejects_non_transcripts() {
        assert!(read_text_file("recording.mp3".into()).is_err());
    }

    #[test]
    fn app_open_path_allows_only_recordings_and_transcripts() {
        assert!(is_yap_media_or_transcript_path(std::path::Path::new("recording.mp3")));
        assert!(is_yap_media_or_transcript_path(std::path::Path::new("recording.MP4")));
        assert!(is_yap_media_or_transcript_path(std::path::Path::new("recording.txt")));
        assert!(!is_yap_media_or_transcript_path(std::path::Path::new("script.ps1")));
    }

    #[test]
    fn polished_path_writes_sibling_file() {
        let path = polished_path(std::path::Path::new("C:/recordings/take.txt")).unwrap();
        assert_eq!(path.file_name().unwrap(), "take.polished.txt");
    }

}
