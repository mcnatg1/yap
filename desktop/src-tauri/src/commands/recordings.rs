use crate::{authorization, runtime};

#[tauri::command]
pub(super) fn server_connection_status(
    window: tauri::WebviewWindow,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
) -> Result<runtime::state::ServerConnectorState, String> {
    authorization::ensure_main(&window)?;
    Ok(runtime_state.with(|orchestrator| orchestrator.server()))
}
