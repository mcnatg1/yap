use crate::{authorization, live, runtime_policy, stt};

#[tauri::command]
pub(super) fn polish_num_gpu(window: tauri::WebviewWindow) -> Result<u32, String> {
    authorization::ensure_main(&window)?;
    Ok(stt::settings::polish_num_gpu_layers())
}

#[tauri::command]
pub(super) fn setup_status(
    window: tauri::WebviewWindow,
    _state: tauri::State<'_, stt::dispatch::SttState>,
) -> Result<runtime_policy::SetupStatus, String> {
    authorization::ensure_main(&window)?;
    Ok(runtime_policy::current_setup_status())
}

#[tauri::command]
pub(super) fn fallback_model_status(
    window: tauri::WebviewWindow,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    authorization::ensure_main_stt(&window)?;
    Ok(stt::fallback_model::status(install_state.inner()))
}

#[tauri::command]
pub(super) async fn fallback_model_install(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
    live_state: tauri::State<'_, live::LiveSessionState>,
    force: Option<bool>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    authorization::ensure_main_stt(&window)?;
    ensure_fallback_setup_idle(&live_state)?;
    stt::fallback_model::install(app, install_state.inner().clone(), force.unwrap_or(false)).await
}

#[tauri::command]
pub(super) fn fallback_model_cancel_install(
    window: tauri::WebviewWindow,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    authorization::ensure_main_stt(&window)?;
    stt::fallback_model::cancel_install(install_state.inner())
}

#[tauri::command]
pub(super) async fn fallback_model_verify(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    authorization::ensure_main_stt(&window)?;
    ensure_fallback_setup_idle(&live_state)?;
    stt::fallback_model::verify(app, install_state.inner().clone()).await
}

#[tauri::command]
pub(super) fn fallback_model_remove(
    window: tauri::WebviewWindow,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    authorization::ensure_main_stt(&window)?;
    ensure_fallback_setup_idle(&live_state)?;
    stt::fallback_model::remove(install_state.inner())
}

#[tauri::command]
pub(super) fn fallback_model_set_enabled(
    window: tauri::WebviewWindow,
    install_state: tauri::State<'_, stt::fallback_model::FallbackModelInstallState>,
    live_state: tauri::State<'_, live::LiveSessionState>,
    enabled: bool,
) -> Result<stt::nemotron::FallbackModelView, stt::dispatch::SttCommandError> {
    authorization::ensure_main_stt(&window)?;
    ensure_fallback_setup_idle(&live_state)?;
    stt::fallback_model::set_enabled(install_state.inner(), enabled)
}

#[tauri::command]
pub(super) fn fallback_model_open_folder(
    window: tauri::WebviewWindow,
    _app: tauri::AppHandle,
) -> Result<(), stt::dispatch::SttCommandError> {
    authorization::ensure_main_stt(&window)?;
    stt::fallback_model::open_folder()
}

#[tauri::command]
pub(super) fn list_local_compute_targets(
    window: tauri::WebviewWindow,
) -> Result<Vec<LocalComputeTargetView>, String> {
    authorization::ensure_main(&window)?;
    Ok(local_compute_targets())
}

#[tauri::command]
pub(super) fn set_local_compute_target(
    window: tauri::WebviewWindow,
    live_state: tauri::State<'_, live::LiveSessionState>,
    target_id: String,
) -> Result<Vec<LocalComputeTargetView>, String> {
    authorization::ensure_main(&window)?;
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

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LocalComputeTargetView {
    id: String,
    label: String,
    selected: bool,
}

fn live_setup_busy_error() -> stt::dispatch::SttCommandError {
    stt::dispatch::SttCommandError {
        code: stt::error::SttError::Busy.code().to_string(),
        message: "Stop live before changing local fallback.".into(),
    }
}
