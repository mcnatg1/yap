mod commands;
mod enrollment;
mod kind;
mod registration;

use crate::live;

pub(crate) use enrollment::HotkeyEnrollmentGate;
use kind::LiveHotkeyKind;

pub(crate) const DICTATION_UNAVAILABLE_ERROR: &str = "Live shortcut is unavailable.";
pub(crate) const PASTE_UNAVAILABLE_ERROR: &str = "Paste shortcut is unavailable.";

#[tauri::command]
pub(crate) async fn record_live_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    gate: tauri::State<'_, HotkeyEnrollmentGate>,
) -> Result<live::state::LiveSessionView, String> {
    commands::record_hotkey(window, app, state, gate, LiveHotkeyKind::Dictation).await
}

#[tauri::command]
pub(crate) fn clear_live_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    commands::change_hotkey(window, app, state, LiveHotkeyKind::Dictation, None)
}

#[tauri::command]
pub(crate) fn reset_live_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    commands::change_hotkey(
        window,
        app,
        state,
        LiveHotkeyKind::Dictation,
        Some(live::settings::DEFAULT_HOTKEY.into()),
    )
}

#[tauri::command]
pub(crate) async fn record_live_paste_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    gate: tauri::State<'_, HotkeyEnrollmentGate>,
) -> Result<live::state::LiveSessionView, String> {
    commands::record_hotkey(window, app, state, gate, LiveHotkeyKind::PasteLast).await
}

#[tauri::command]
pub(crate) fn clear_live_paste_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    commands::change_hotkey(window, app, state, LiveHotkeyKind::PasteLast, None)
}

#[tauri::command]
pub(crate) fn reset_live_paste_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    commands::change_hotkey(
        window,
        app,
        state,
        LiveHotkeyKind::PasteLast,
        Some(live::settings::DEFAULT_PASTE_HOTKEY.into()),
    )
}

#[cfg(test)]
mod tests;
