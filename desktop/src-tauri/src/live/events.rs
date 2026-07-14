use tauri::Emitter;

use crate::{authorization, live};

pub(crate) fn emit_session(app: &tauri::AppHandle, view: &live::state::LiveSessionView) {
    let _ = app.emit_to(authorization::MAIN_WINDOW_LABEL, "live-session", view);
    let overlay = live::state::LiveOverlayView::from(view);
    let _ = app.emit_to(
        authorization::LIVE_OVERLAY_WINDOW_LABEL,
        "live-overlay-session",
        overlay,
    );
}

pub(crate) fn emit_level(app: &tauri::AppHandle, view: &live::state::LiveLevelView) {
    let _ = app.emit_to(authorization::LIVE_OVERLAY_WINDOW_LABEL, "live-level", view);
}

pub(crate) fn emit_saved(app: &tauri::AppHandle, saved: &live::recordings::SavedLiveSession) {
    let _ = app.emit_to(
        authorization::MAIN_WINDOW_LABEL,
        "live-session-saved",
        saved,
    );
}
