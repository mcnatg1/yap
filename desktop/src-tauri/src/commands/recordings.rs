use crate::{authorization, batch_recordings, file_actions, runtime, runtime_policy, stt};

#[tauri::command]
pub(super) fn server_connection_status(
    window: tauri::WebviewWindow,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
) -> Result<runtime::state::ServerConnectorState, String> {
    authorization::ensure_main(&window)?;
    Ok(runtime_state.with(|orchestrator| orchestrator.server()))
}

#[tauri::command]
pub(super) fn start_transcribe(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, stt::dispatch::SttState>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
    paths: Vec<String>,
) -> Result<(), stt::dispatch::SttCommandError> {
    authorization::ensure_main_stt(&window)?;
    if paths.is_empty() {
        return Ok(());
    }
    let paths = batch_recordings::validate_recording_paths(&paths)?;
    file_actions::ensure_registered_recording_paths(&paths).map_err(|message| {
        stt::dispatch::SttCommandError {
            code: stt::error::SttError::AudioDecode.code().to_string(),
            message,
        }
    })?;
    if state.is_transcribing() {
        return Err(stt::dispatch::SttCommandError {
            code: stt::error::SttError::Busy.code().to_string(),
            message: stt::error::SttError::Busy.user_message().to_string(),
        });
    }

    let setup = runtime_policy::current_setup_status();
    runtime_state
        .with(|orchestrator| {
            orchestrator.set_setup(setup.runtime_setup_state());
            orchestrator.route_recording(true)
        })
        .map_err(runtime_policy::runtime_error_to_stt)?;
    stt::log_yap(&format!(
        "start_transcribe blocked count={} reason=server_batch_unwired",
        paths.len()
    ));
    Err(runtime_policy::runtime_error_to_stt(
        runtime::RuntimeError::ServerUnavailable,
    ))
}
