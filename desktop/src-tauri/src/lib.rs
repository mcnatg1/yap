mod app;
mod atomic_text;
pub mod audio;
mod authorization;
mod commands;
mod file_actions;
mod install_identity;
pub mod jobs;
pub mod live;
pub(crate) mod media_protocol;
mod paths;
pub mod runtime;
mod runtime_policy;
pub mod server_connector;
pub mod stt;
mod tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    app::run();
}
