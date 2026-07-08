use std::{
    io::Write,
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
const LIVE_SHORTCUT_DOUBLE_TAP_MS: u64 = 320;
const LIVE_SHORTCUT_HOLD_MS: u64 = 160;
const LIVE_WAV_SAMPLE_RATE: u32 = 16_000;
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
fn list_saved_live_sessions(window: tauri::WebviewWindow) -> Result<Vec<SavedLiveSession>, String> {
    file_actions::ensure_main_window(&window)?;
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
        stt::nemotron::ensure_model()?;
        Ok(current_setup_status())
    })
    .await
    .map_err(|_| stt::dispatch::SttCommandError::from(stt::error::SttError::SidecarCrash))?
}

#[tauri::command]
fn remove_local_fallback(
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<SetupStatus, stt::dispatch::SttCommandError> {
    if live::state::is_live_session_started(live_state.snapshot().status) {
        return Err(live_setup_busy_error());
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
    let model_installed = stt::nemotron::is_installed();
    let fallback_enabled = stt::settings::local_fallback_enabled();
    let engine_ready = fallback_enabled && model_installed;
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
        engine_status: compose_engine_status(model_installed, fallback_enabled),
    }
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

fn compose_engine_status(model_installed: bool, fallback_enabled: bool) -> String {
    if !fallback_enabled {
        return "Local fallback disabled".into();
    }
    if model_installed {
        "Transcription engine ready".into()
    } else {
        "Local fallback model missing".into()
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
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalComputeTargetView {
    id: String,
    label: String,
    selected: bool,
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
    let transcript_body =
        transcript.unwrap_or_else(|| "Transcript unavailable for this live recording.".into());

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
        if !file_actions::is_transcript_path(&path) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !stem.starts_with("live-") {
            continue;
        }

        normalize_saved_live_transcript(&path)?;

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
        .map(clean_live_transcript_text)
        .filter(|text| !text.is_empty())
}

fn clean_live_transcript_text(text: &str) -> String {
    if text.trim() == "No live transcript captured." {
        return "Transcript unavailable for this live recording.".into();
    }

    let mut cleaned = text
        .split_whitespace()
        .map(fix_word_casing)
        .collect::<Vec<_>>()
        .join(" ");
    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", ".");
    }
    cleaned
}

fn fix_word_casing(word: &str) -> String {
    let mut chars = word.chars();
    let (Some(first), Some(second), Some(third)) = (chars.next(), chars.next(), chars.next())
    else {
        return word.to_string();
    };

    if first.is_uppercase() && second.is_uppercase() && third.is_lowercase() {
        let mut fixed = String::new();
        fixed.push(first);
        fixed.extend(second.to_lowercase());
        fixed.push(third);
        fixed.extend(chars);
        fixed
    } else {
        word.to_string()
    }
}

fn normalize_saved_live_transcript(path: &std::path::Path) -> Result<(), String> {
    let current = std::fs::read_to_string(path)
        .map_err(|err| format!("Failed to read saved live transcript: {err}"))?;
    let cleaned = clean_live_transcript_text(&current);
    if cleaned.is_empty() || cleaned.trim_end() == current.trim_end() {
        return Ok(());
    }

    std::fs::write(path, format!("{cleaned}\n"))
        .map_err(|err| format!("Failed to repair saved live transcript: {err}"))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveShortcutAction {
    None,
    ScheduleHold(u64),
    Start(live::state::LiveCaptureMode),
    Stop,
}

#[derive(Debug, Default)]
struct LiveShortcutInteraction {
    ignore_next_release: bool,
    key_down: bool,
    last_tap_at: Option<Instant>,
    pending_press_at: Option<Instant>,
    pending_press_id: u64,
    starting_push_to_talk: bool,
    stop_push_to_talk_after_start: bool,
}

impl LiveShortcutInteraction {
    fn reset(&mut self) {
        self.ignore_next_release = false;
        self.key_down = false;
        self.last_tap_at = None;
        self.pending_press_at = None;
        self.starting_push_to_talk = false;
        self.stop_push_to_talk_after_start = false;
    }

    fn finish_push_to_talk_start(&mut self) -> bool {
        self.starting_push_to_talk = false;
        std::mem::take(&mut self.stop_push_to_talk_after_start)
    }

    fn pressed(
        &mut self,
        now: Instant,
        active_mode: Option<live::state::LiveCaptureMode>,
    ) -> LiveShortcutAction {
        if self.key_down {
            return LiveShortcutAction::None;
        }
        self.key_down = true;
        if active_mode == Some(live::state::LiveCaptureMode::Toggle) {
            self.ignore_next_release = true;
            self.pending_press_at = None;
            self.last_tap_at = None;
            return LiveShortcutAction::Stop;
        }
        if active_mode.is_some() {
            return LiveShortcutAction::None;
        }
        if self.last_tap_at.is_some_and(|then| {
            now.duration_since(then) <= Duration::from_millis(LIVE_SHORTCUT_DOUBLE_TAP_MS)
        }) {
            self.pending_press_at = None;
            self.last_tap_at = None;
            return LiveShortcutAction::Start(live::state::LiveCaptureMode::Toggle);
        }

        self.pending_press_id = self.pending_press_id.wrapping_add(1);
        self.pending_press_at = Some(now);
        self.last_tap_at = None;
        LiveShortcutAction::ScheduleHold(self.pending_press_id)
    }

    fn hold_elapsed(
        &mut self,
        press_id: u64,
        now: Instant,
        active_mode: Option<live::state::LiveCaptureMode>,
    ) -> LiveShortcutAction {
        let Some(pressed_at) = self.pending_press_at else {
            return LiveShortcutAction::None;
        };
        if press_id != self.pending_press_id
            || active_mode.is_some()
            || now.duration_since(pressed_at) < Duration::from_millis(LIVE_SHORTCUT_HOLD_MS)
        {
            return LiveShortcutAction::None;
        }

        self.pending_press_at = None;
        self.last_tap_at = None;
        self.starting_push_to_talk = true;
        LiveShortcutAction::Start(live::state::LiveCaptureMode::PushToTalk)
    }

    fn released(
        &mut self,
        now: Instant,
        active_mode: Option<live::state::LiveCaptureMode>,
    ) -> LiveShortcutAction {
        self.key_down = false;
        if self.ignore_next_release {
            self.ignore_next_release = false;
            return LiveShortcutAction::None;
        }
        if active_mode == Some(live::state::LiveCaptureMode::PushToTalk) {
            return LiveShortcutAction::Stop;
        }
        if active_mode == Some(live::state::LiveCaptureMode::Toggle) {
            return LiveShortcutAction::None;
        }
        if self.starting_push_to_talk {
            self.stop_push_to_talk_after_start = true;
            return LiveShortcutAction::None;
        }
        if self.pending_press_at.take().is_some() {
            self.last_tap_at = Some(now);
        }
        LiveShortcutAction::None
    }
}

fn handle_live_shortcut_action(
    app: tauri::AppHandle,
    interaction: Arc<Mutex<LiveShortcutInteraction>>,
    action: LiveShortcutAction,
) {
    match action {
        LiveShortcutAction::None => {}
        LiveShortcutAction::ScheduleHold(press_id) => {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(LIVE_SHORTCUT_HOLD_MS));
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
        LiveShortcutAction::Start(capture_mode) => {
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
        LiveShortcutAction::Stop => {
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
    let live_runtime_for_monitor = live_runtime.clone();
    let live_runtime_for_exit = live_runtime.clone();
    let live_shortcut_interaction = Arc::new(Mutex::new(LiveShortcutInteraction::default()));

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
    fn shortcut_double_tap_starts_hands_free_and_release_is_ignored() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.released(now + Duration::from_millis(40), None),
            LiveShortcutAction::None
        );
        assert_eq!(
            shortcut.pressed(now + Duration::from_millis(120), None),
            LiveShortcutAction::Start(live::state::LiveCaptureMode::Toggle)
        );
        assert_eq!(
            shortcut.released(
                now + Duration::from_millis(150),
                Some(live::state::LiveCaptureMode::Toggle),
            ),
            LiveShortcutAction::None
        );
        assert_eq!(
            shortcut.pressed(
                now + Duration::from_millis(240),
                Some(live::state::LiveCaptureMode::Toggle),
            ),
            LiveShortcutAction::Stop
        );
        assert_eq!(
            shortcut.released(now + Duration::from_millis(260), None),
            LiveShortcutAction::None
        );
    }

    #[test]
    fn shortcut_reset_clears_stale_tap_state() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.released(now + Duration::from_millis(40), None),
            LiveShortcutAction::None
        );
        shortcut.reset();

        assert_eq!(
            shortcut.pressed(now + Duration::from_millis(120), None),
            LiveShortcutAction::ScheduleHold(2)
        );
    }

    #[test]
    fn shortcut_ignores_repeated_pressed_events_until_release() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.pressed(now + Duration::from_millis(20), None),
            LiveShortcutAction::None
        );
        assert_eq!(
            shortcut.hold_elapsed(
                1,
                now + Duration::from_millis(LIVE_SHORTCUT_HOLD_MS + 1),
                None,
            ),
            LiveShortcutAction::Start(live::state::LiveCaptureMode::PushToTalk)
        );
    }

    #[test]
    fn shortcut_release_during_push_to_talk_start_requests_stop_after_start() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.hold_elapsed(
                1,
                now + Duration::from_millis(LIVE_SHORTCUT_HOLD_MS + 1),
                None,
            ),
            LiveShortcutAction::Start(live::state::LiveCaptureMode::PushToTalk)
        );
        assert_eq!(
            shortcut.released(now + Duration::from_millis(180), None),
            LiveShortcutAction::None
        );
        assert!(shortcut.finish_push_to_talk_start());
    }

    #[test]
    fn shortcut_hold_starts_push_to_talk_and_release_stops() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.hold_elapsed(
                1,
                now + Duration::from_millis(LIVE_SHORTCUT_HOLD_MS + 1),
                None,
            ),
            LiveShortcutAction::Start(live::state::LiveCaptureMode::PushToTalk)
        );
        assert_eq!(
            shortcut.released(
                now + Duration::from_millis(260),
                Some(live::state::LiveCaptureMode::PushToTalk),
            ),
            LiveShortcutAction::Stop
        );
    }

    #[test]
    fn shortcut_single_tap_does_not_start_recording() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.released(now + Duration::from_millis(40), None),
            LiveShortcutAction::None
        );
        assert_eq!(
            shortcut.hold_elapsed(
                1,
                now + Duration::from_millis(LIVE_SHORTCUT_HOLD_MS + 1),
                None,
            ),
            LiveShortcutAction::None
        );
    }

    #[test]
    fn disabled_status_wins() {
        assert_eq!(
            compose_engine_status(true, false),
            "Local fallback disabled"
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
        };

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
            active_capture_mode: None,
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
    fn live_transcript_text_cleans_streaming_artifacts() {
        let mut view = live::state::LiveSessionView {
            visibility: live::state::LiveOverlayVisibility::Enabled,
            status: live::state::LiveSessionStatus::Idle,
            route: live::state::LiveRoute::None,
            capture_mode: live::state::LiveCaptureMode::PushToTalk,
            active_capture_mode: None,
            hotkey: String::new(),
            input_device_id: None,
            input_device_label: None,
            level: None,
            partial_text: None,
            final_text: Some("  THank   you.. ".into()),
            error: None,
        };

        assert_eq!(live_transcript_text(&view).as_deref(), Some("Thank you."));
        view.final_text = Some("NASA called.".into());
        assert_eq!(live_transcript_text(&view).as_deref(), Some("NASA called."));
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
    fn saved_live_session_scan_repairs_streaming_artifacts() {
        let dir = std::env::temp_dir().join(format!("yap-live-clean-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-201.txt");
        std::fs::write(&transcript, "  THank   you.. \n").unwrap();

        let sessions = list_saved_live_session_files_from_dir(&dir).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&transcript).unwrap(),
            "Thank you.\n"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saved_live_session_scan_repairs_old_empty_placeholder() {
        let dir = std::env::temp_dir().join(format!("yap-live-placeholder-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-202.txt");
        std::fs::write(&transcript, "No live transcript captured.\n").unwrap();

        let sessions = list_saved_live_session_files_from_dir(&dir).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&transcript).unwrap(),
            "Transcript unavailable for this live recording.\n"
        );
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
}
