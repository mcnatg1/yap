pub(crate) mod batch;
mod boundary;
mod client;
pub mod config;
mod core;
mod desktop;
mod state;

pub use boundary::ServerConnectorBoundary;
pub(crate) use core::BatchConnectionLease;
pub use core::ServerConnector;
pub use state::{ServerCapabilities, ServerConnectionSnapshot};

fn allow_insecure_private_server() -> bool {
    std::env::var("YAP_ALLOW_INSECURE_PRIVATE_SERVER").as_deref() == Ok("1")
}

#[tauri::command]
pub(crate) fn server_connection_status(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    connector: tauri::State<'_, ServerConnector>,
) -> Result<ServerConnectionSnapshot, String> {
    desktop::connection_status(window, app, connector)
}

#[tauri::command]
pub(crate) async fn refresh_server_connection(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    connector: tauri::State<'_, ServerConnector>,
) -> Result<ServerConnectionSnapshot, String> {
    desktop::refresh_connection(window, app, connector).await
}

#[tauri::command]
pub(crate) fn server_settings(
    window: tauri::WebviewWindow,
) -> Result<config::ServerSettings, String> {
    desktop::load_settings(window)
}

#[tauri::command]
pub(crate) async fn set_server_settings(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    connector: tauri::State<'_, ServerConnector>,
    settings: config::ServerSettings,
) -> Result<config::ServerSettings, String> {
    desktop::save_settings(window, app, connector, settings).await
}

#[cfg(test)]
mod tests;
