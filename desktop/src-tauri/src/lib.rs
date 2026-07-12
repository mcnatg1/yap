mod app;
pub mod audio;
mod authorization;
mod commands;
mod file_actions;
mod install_identity;
pub mod live;
mod paths;
pub mod runtime;
mod runtime_policy;
pub mod stt;
mod tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    app::run();
}
