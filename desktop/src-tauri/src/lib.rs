use std::io::Write;

use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

const LIVE_OVERLAY_COMPACT_WIDTH: f64 = 92.0;
const LIVE_OVERLAY_COMPACT_HEIGHT: f64 = 38.0;
const LIVE_OVERLAY_HOVER_SENSOR_HEIGHT: f64 = 4.0;
const LIVE_OVERLAY_TOP_BEZEL_OFFSET: f64 = 0.0;
const LIVE_WAV_SAMPLE_RATE: u32 = 16_000;
const TRAY_SHOW_APP: &str = "show_app";
const TRAY_START_DICTATING: &str = "start_dictating";
const TRAY_STOP_RECORDING: &str = "stop_recording";
const TRAY_QUIT: &str = "quit";

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
    live_runtime: tauri::State<'_, live::runtime::LiveRuntime>,
) -> Result<SavedLiveSession, String> {
    let view = state.snapshot();
    let saved = save_live_session_files(&live_runtime, &view)?
        .ok_or_else(|| "Nothing to save yet.".to_string())?;
    emit_live_saved(&app, &saved);
    Ok(saved)
}

#[tauri::command]
fn list_saved_live_sessions(window: tauri::WebviewWindow) -> Result<Vec<SavedLiveSession>, String> {
    ensure_main_window(&window)?;
    list_saved_live_session_files()
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
    if live::state::is_live_session_started(live_state.snapshot().status) {
        return Err(live_setup_busy_error());
    }
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
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<SetupStatus, stt::dispatch::SttCommandError> {
    if live::state::is_live_session_started(live_state.snapshot().status) {
        return Err(live_setup_busy_error());
    }
    if let Ok(mut sidecar) = state.sidecar.lock() {
        sidecar.shutdown();
    }
    remove_local_fallback_files()?;
    Ok(current_setup_status())
}

#[tauri::command]
fn set_local_fallback_enabled(
    live_state: tauri::State<'_, live::LiveSessionState>,
    enabled: bool,
) -> Result<SetupStatus, stt::dispatch::SttCommandError> {
    if live::state::is_live_session_started(live_state.snapshot().status) {
        return Err(live_setup_busy_error());
    }
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
    let model_installed = pin.as_ref().map(stt::model::is_installed).unwrap_or(false);
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
    log_line(&format!(
        "start_transcribe blocked count={} reason=server_batch_unwired",
        paths.len()
    ));
    Err(runtime_error_to_stt(
        runtime::RuntimeError::ServerUnavailable,
    ))
}

#[tauri::command]
fn read_text_file(window: tauri::WebviewWindow, path: String) -> Result<String, String> {
    ensure_main_window(&window)?;
    read_text_file_at(path)
}

fn read_text_file_at(path: String) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);

    if !is_transcript_path(&path) {
        return Err("Only transcript text files can be read.".into());
    }

    let path = canonical_existing_path(&path)?;
    std::fs::read_to_string(&path).map_err(|err| format!("Failed to read transcript: {err}"))
}

#[tauri::command]
fn write_polished_text(
    window: tauri::WebviewWindow,
    path: String,
    text: String,
) -> Result<String, String> {
    ensure_main_window(&window)?;
    write_polished_text_at(path, text)
}

fn write_polished_text_at(path: String, text: String) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);

    if !is_transcript_path(&path) {
        return Err("Only transcript text files can be polished.".into());
    }

    let path = canonical_existing_path(&path)?;
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
fn open_app_path(window: tauri::WebviewWindow, path: String) -> Result<(), String> {
    ensure_main_window(&window)?;
    let path = openable_app_path(path)?;
    tauri_plugin_opener::open_path(&path, None::<&str>)
        .map_err(|err| format!("Failed to open file: {err}"))
}

#[tauri::command]
fn reveal_app_path(window: tauri::WebviewWindow, path: String) -> Result<(), String> {
    ensure_main_window(&window)?;
    let path = openable_app_path(path)?;
    tauri_plugin_opener::reveal_item_in_dir(path)
        .map_err(|err| format!("Failed to reveal file: {err}"))
}

#[tauri::command]
fn delete_history_entry_files(
    window: tauri::WebviewWindow,
    output_path: String,
    source_path: String,
) -> Result<(), String> {
    ensure_main_window(&window)?;
    delete_history_entry_files_at(output_path, source_path)
}

fn delete_history_entry_files_at(output_path: String, source_path: String) -> Result<(), String> {
    delete_history_entry_files_at_from_dir(output_path, source_path, &live_recordings_dir())
}

fn delete_history_entry_files_at_from_dir(
    output_path: String,
    source_path: String,
    owned_dir: &std::path::Path,
) -> Result<(), String> {
    let output = deletable_transcript_path(output_path)?;
    let source = deletable_yap_owned_recording_path_from_dir(source_path, owned_dir)?;

    if let Some(source) = source.filter(|source| source != &output) {
        std::fs::remove_file(&source)
            .map_err(|err| format!("Failed to delete recording: {err}"))?;
    }

    std::fs::remove_file(&output).map_err(|err| format!("Failed to delete transcript: {err}"))
}

fn openable_app_path(path: String) -> Result<std::path::PathBuf, String> {
    let path = std::path::PathBuf::from(path);
    if !is_yap_media_or_transcript_path(&path) {
        return Err("Only Yap recording and transcript files can be opened.".into());
    }
    let path = canonical_existing_path(&path)?;
    if !is_yap_media_or_transcript_path(&path) {
        return Err("Only Yap recording and transcript files can be opened.".into());
    }
    Ok(path)
}

fn deletable_transcript_path(path: String) -> Result<std::path::PathBuf, String> {
    let path = std::path::PathBuf::from(path);
    if !is_transcript_path(&path) {
        return Err("Only transcript text files can be deleted.".into());
    }
    let path = canonical_existing_path(&path)?;
    if !is_transcript_path(&path) {
        return Err("Only transcript text files can be deleted.".into());
    }
    Ok(path)
}

fn deletable_yap_owned_recording_path_from_dir(
    path: String,
    owned_dir: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, String> {
    let path = std::path::PathBuf::from(path);
    if !path.exists() {
        return Ok(None);
    }
    let path = path
        .canonicalize()
        .map_err(|err| format!("Failed to resolve recording path: {err}"))?;
    let Ok(owned_dir) = owned_dir.canonicalize() else {
        return Ok(None);
    };

    if path.starts_with(owned_dir) && is_yap_media_or_transcript_path(&path) {
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

fn canonical_existing_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if !path.exists() {
        return Err("File no longer exists.".into());
    }
    path.canonicalize()
        .map_err(|err| format!("Failed to resolve file path: {err}"))
}

fn ensure_main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("This file action is only available from the main window.".into())
    }
}

fn is_transcript_path(path: &std::path::Path) -> bool {
    has_extension(path, &["txt"])
}

fn is_yap_media_or_transcript_path(path: &std::path::Path) -> bool {
    has_extension(
        path,
        &["txt", "mp3", "m4a", "wav", "mp4", "flac", "ogg", "webm"],
    )
}

fn has_extension(path: &std::path::Path, allowed: &[&str]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            allowed
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
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

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedLiveSession {
    name: String,
    source_path: String,
    output_path: String,
    created_at_ms: u64,
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

fn emit_live_saved(app: &tauri::AppHandle, saved: &SavedLiveSession) {
    let _ = app.emit("live-session-saved", saved);
}

fn live_recordings_dir_from<F>(env: F) -> std::path::PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = env("YAP_LIVE_RECORDINGS_DIR") {
        return std::path::PathBuf::from(dir);
    }
    if let Some(local) = env("LOCALAPPDATA") {
        return std::path::PathBuf::from(local)
            .join("Yap")
            .join("live-recordings");
    }
    std::path::PathBuf::from(".").join("live-recordings")
}

fn live_recordings_dir() -> std::path::PathBuf {
    live_recordings_dir_from(|key| std::env::var(key).ok())
}

fn save_live_session_files(
    live_runtime: &live::runtime::LiveRuntime,
    view: &live::state::LiveSessionView,
) -> Result<Option<SavedLiveSession>, String> {
    let transcript = live_transcript_text(view);
    let pcm = live_runtime.recorded_pcm();
    if transcript.is_none() && pcm.is_empty() {
        return Ok(None);
    }

    let dir = live_recordings_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create live recordings folder: {err}"))?;
    let created_at_ms = unix_millis_now()?;
    let name = format!("live-{created_at_ms}");
    let transcript_path = dir.join(format!("{name}.txt"));
    let audio_path = dir.join(format!("{name}.wav"));
    let transcript_body = transcript.unwrap_or_else(|| "No live transcript captured.".into());

    if !pcm.is_empty() {
        write_pcm16_wav(&audio_path, &pcm)?;
    }
    std::fs::write(&transcript_path, format!("{transcript_body}\n"))
        .map_err(|err| format!("Failed to save live transcript: {err}"))?;

    Ok(Some(SavedLiveSession {
        name,
        source_path: if pcm.is_empty() {
            transcript_path.display().to_string()
        } else {
            audio_path.display().to_string()
        },
        output_path: transcript_path.display().to_string(),
        created_at_ms,
    }))
}

fn list_saved_live_session_files() -> Result<Vec<SavedLiveSession>, String> {
    list_saved_live_session_files_from_dir(&live_recordings_dir())
}

fn list_saved_live_session_files_from_dir(
    dir: &std::path::Path,
) -> Result<Vec<SavedLiveSession>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in
        std::fs::read_dir(dir).map_err(|err| format!("Failed to read live recordings: {err}"))?
    {
        let entry = entry.map_err(|err| format!("Failed to read live recording: {err}"))?;
        let path = entry.path();
        if !is_transcript_path(&path) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !stem.starts_with("live-") {
            continue;
        }

        let audio_path = path.with_extension("wav");
        let source_path = if audio_path.exists() {
            audio_path
        } else {
            path.clone()
        };
        let created_at_ms = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(system_time_to_unix_millis)
            .unwrap_or(0);
        sessions.push(SavedLiveSession {
            name: stem.to_string(),
            source_path: source_path.display().to_string(),
            output_path: path.display().to_string(),
            created_at_ms,
        });
    }

    sessions.sort_by(|a, b| {
        b.created_at_ms
            .cmp(&a.created_at_ms)
            .then_with(|| b.name.cmp(&a.name))
    });
    Ok(sessions)
}

fn unix_millis_now() -> Result<u64, String> {
    system_time_to_unix_millis(std::time::SystemTime::now())
        .ok_or_else(|| "System clock error: timestamp out of range.".to_string())
}

fn system_time_to_unix_millis(time: std::time::SystemTime) -> Option<u64> {
    let millis = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    u64::try_from(millis).ok()
}

fn live_transcript_text(view: &live::state::LiveSessionView) -> Option<String> {
    view.final_text
        .as_deref()
        .or(view.partial_text.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn write_pcm16_wav(path: &std::path::Path, pcm: &[u8]) -> Result<(), String> {
    let data_len =
        u32::try_from(pcm.len()).map_err(|_| "Live recording is too large to save.".to_string())?;
    let riff_len = 36u32
        .checked_add(data_len)
        .ok_or_else(|| "Live recording is too large to save.".to_string())?;
    let byte_rate = LIVE_WAV_SAMPLE_RATE * 2;
    let mut file =
        std::fs::File::create(path).map_err(|err| format!("Failed to save live audio: {err}"))?;

    file.write_all(b"RIFF").map_err(wav_write_error)?;
    file.write_all(&riff_len.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(b"WAVEfmt ").map_err(wav_write_error)?;
    file.write_all(&16u32.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(&1u16.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(&1u16.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(&LIVE_WAV_SAMPLE_RATE.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(&byte_rate.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(&2u16.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(&16u16.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(b"data").map_err(wav_write_error)?;
    file.write_all(&data_len.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(pcm).map_err(wav_write_error)
}

fn wav_write_error(err: std::io::Error) -> String {
    format!("Failed to save live audio: {err}")
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
    let _ = start_live_runtime(app.clone(), &live, &live_runtime, &stt, &orchestrator);
}

fn stop_live_from_app(app: &tauri::AppHandle) {
    let live = app.state::<live::LiveSessionState>();
    let live_runtime = app.state::<live::runtime::LiveRuntime>();
    let orchestrator = app.state::<runtime::RuntimeOrchestratorState>();
    let _ = stop_live_runtime(app.clone(), &live, &live_runtime, &orchestrator);
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
) -> live::state::LiveSessionView {
    if live::state::is_live_session_started(live.snapshot().status) || live_runtime.is_active() {
        return live.snapshot();
    }

    if stt.is_transcribing() {
        let view = live.block_with_error(stt::error::SttError::Busy.user_message());
        if let Err(error) = ensure_live_overlay(&app) {
            log_line(&format!("live overlay busy show failed: {error}"));
        }
        emit_live(&app, &view);
        return view;
    }

    let setup = current_setup_status().runtime_setup_state();
    orchestrator.with(|orchestrator| orchestrator.set_setup(setup));
    if live::state::live_route_for(setup, false) == live::state::LiveRoute::Blocked {
        let view = block_live_for_setup(live, setup);
        if let Err(error) = ensure_live_overlay(&app) {
            log_line(&format!("live overlay blocked show failed: {error}"));
        }
        emit_live(&app, &view);
        return view;
    }

    if let Err(error) = orchestrator.with(|orchestrator| orchestrator.start_fallback()) {
        let view = live.block_with_error(&runtime_error_to_stt(error).message);
        if let Err(error) = ensure_live_overlay(&app) {
            log_line(&format!("live overlay route error show failed: {error}"));
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
        view.visibility = live::state::LiveOverlayVisibility::Enabled;
    });
    let _ = persist_live_view(&view);
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
    live_runtime.stop();
    let before_stop = live.snapshot();
    orchestrator.with(|orchestrator| orchestrator.finish_active_work());
    let view = live.stop();
    match save_live_session_files(live_runtime, &before_stop) {
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
        LIVE_OVERLAY_COMPACT_WIDTH,
        LIVE_OVERLAY_HOVER_SENSOR_HEIGHT,
    )
}

fn ensure_live_overlay_size(app: &tauri::AppHandle, width: f64, height: f64) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("live-overlay") {
        window
            .set_size(tauri::LogicalSize::new(width, height))
            .map_err(|err| format!("Failed to size live overlay: {err}"))?;
        window
            .set_shadow(false)
            .map_err(|err| format!("Failed to hide live overlay shadow: {err}"))?;
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
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .build()
    .map_err(|err| format!("Failed to create live overlay: {err}"))?;
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
        .get_webview_window("main")
        .and_then(|window| window.current_monitor().ok().flatten())
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

    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());

    #[cfg(feature = "wdio")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());

    builder
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
                                let snapshot = live.snapshot();
                                if live::state::is_live_session_started(snapshot.status)
                                    && snapshot.capture_mode
                                        == live::state::LiveCaptureMode::PushToTalk
                                {
                                    return;
                                }
                                let view = if snapshot.capture_mode
                                    == live::state::LiveCaptureMode::Toggle
                                    && live::state::is_live_session_started(snapshot.status)
                                {
                                    stop_live_runtime(
                                        app.clone(),
                                        &live,
                                        &live_runtime,
                                        &orchestrator,
                                    )
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
                                if live.snapshot().capture_mode
                                    == live::state::LiveCaptureMode::PushToTalk
                                {
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
            list_saved_live_sessions,
            show_main_workspace,
            polish_num_gpu,
            start_transcribe,
            read_text_file,
            write_polished_text,
            open_app_path,
            reveal_app_path,
            delete_history_entry_files
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
            tauri::RunEvent::Exit => {
                live_runtime_for_exit.shutdown();
                if let Ok(mut sidecar) = sidecar_for_exit.lock() {
                    sidecar.shutdown();
                }
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yap-{name}-{}-{}",
            std::process::id(),
            unix_millis_now().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

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
    fn live_transcript_text_prefers_final_then_partial() {
        let mut view = live::state::LiveSessionView {
            visibility: live::state::LiveOverlayVisibility::Enabled,
            status: live::state::LiveSessionStatus::Idle,
            route: live::state::LiveRoute::None,
            capture_mode: live::state::LiveCaptureMode::PushToTalk,
            hotkey: String::new(),
            input_device_id: None,
            input_device_label: None,
            level: None,
            partial_text: Some("partial".into()),
            final_text: Some("final".into()),
            error: None,
        };

        assert_eq!(live_transcript_text(&view).as_deref(), Some("final"));
        view.final_text = None;
        assert_eq!(live_transcript_text(&view).as_deref(), Some("partial"));
    }

    #[test]
    fn write_pcm16_wav_writes_standard_header_and_data() {
        let path = std::env::temp_dir().join(format!("yap-live-{}.wav", std::process::id()));
        let pcm = [0, 0, 255, 127];

        write_pcm16_wav(&path, &pcm).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 4);
        assert_eq!(&bytes[44..], pcm);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn saved_live_session_scan_pairs_transcripts_with_audio() {
        let dir = std::env::temp_dir().join(format!("yap-live-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-200.txt");
        let audio = dir.join("live-200.wav");
        let ignored = dir.join("note.txt");
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::write(&audio, b"RIFF").unwrap();
        std::fs::write(&ignored, "not a live session\n").unwrap();

        let sessions = list_saved_live_session_files_from_dir(&dir).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "live-200");
        assert_eq!(sessions[0].output_path, transcript.display().to_string());
        assert_eq!(sessions[0].source_path, audio.display().to_string());
        std::fs::remove_dir_all(dir).ok();
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
        assert!(read_text_file_at("recording.mp3".into()).is_err());
    }

    #[test]
    fn app_open_path_allows_only_recordings_and_transcripts() {
        assert!(is_yap_media_or_transcript_path(std::path::Path::new(
            "recording.mp3"
        )));
        assert!(is_yap_media_or_transcript_path(std::path::Path::new(
            "recording.MP4"
        )));
        assert!(is_yap_media_or_transcript_path(std::path::Path::new(
            "recording.txt"
        )));
        assert!(!is_yap_media_or_transcript_path(std::path::Path::new(
            "script.ps1"
        )));
    }

    #[test]
    fn delete_history_entry_files_removes_owned_live_audio() {
        let dir = temp_test_dir("delete-owned-live");
        let transcript = dir.join("live-300.txt");
        let audio = dir.join("live-300.wav");
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::write(&audio, b"RIFF").unwrap();

        delete_history_entry_files_at_from_dir(
            transcript.display().to_string(),
            audio.display().to_string(),
            &dir,
        )
        .unwrap();

        assert!(!transcript.exists());
        assert!(!audio.exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn delete_history_entry_files_keeps_imported_source_audio() {
        let owned_dir = temp_test_dir("delete-owned-dir");
        let imported_dir = temp_test_dir("delete-imported-source");
        let transcript = imported_dir.join("clip.txt");
        let audio = imported_dir.join("clip.wav");
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::write(&audio, b"RIFF").unwrap();

        delete_history_entry_files_at_from_dir(
            transcript.display().to_string(),
            audio.display().to_string(),
            &owned_dir,
        )
        .unwrap();

        assert!(!transcript.exists());
        assert!(audio.exists());
        std::fs::remove_dir_all(owned_dir).ok();
        std::fs::remove_dir_all(imported_dir).ok();
    }

    #[test]
    fn polished_path_writes_sibling_file() {
        let path = polished_path(std::path::Path::new("C:/recordings/take.txt")).unwrap();
        assert_eq!(path.file_name().unwrap(), "take.polished.txt");
    }
}
