use tauri::Emitter;

use crate::live;

pub(crate) fn emit_session(app: &tauri::AppHandle, view: &live::state::LiveSessionView) {
    let _ = app.emit("live-session", view);
}

pub(crate) fn emit_saved(app: &tauri::AppHandle, saved: &live::recordings::SavedLiveSession) {
    let _ = app.emit("live-session-saved", saved);
}
