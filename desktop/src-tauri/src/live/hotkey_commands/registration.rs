use tauri_plugin_global_shortcut::Shortcut;

use crate::live;

use super::kind::LiveHotkeyKind;

pub(super) fn apply_successful_hotkey_change(
    view: &mut live::state::LiveSessionView,
    kind: LiveHotkeyKind,
    hotkey: String,
    recovered_startup_failure: bool,
) {
    kind.update(view, hotkey);
    if !recovered_startup_failure {
        return;
    }

    if view.error.as_deref() == Some(kind.startup_error()) {
        view.error = None;
    }
    if matches!(kind, LiveHotkeyKind::Dictation) {
        view.route = live::state::LiveRoute::None;
        view.status = live::state::LiveSessionStatus::Idle;
    }
}

pub(super) fn ensure_live_hotkey_idle(
    status: live::state::LiveSessionStatus,
) -> Result<(), String> {
    if live::state::is_live_session_started(status) {
        return Err("Stop live before changing the shortcut.".into());
    }
    Ok(())
}

pub(crate) fn replace_hotkey_registration(
    previous: Option<Shortcut>,
    next: Option<Shortcut>,
    mut unregister: impl FnMut(Shortcut) -> Result<(), String>,
    mut register: impl FnMut(Shortcut) -> Result<(), String>,
    persist: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if let Some(shortcut) = previous.as_ref() {
        unregister(*shortcut)?;
    }

    if let Some(shortcut) = next.as_ref() {
        if let Err(error) = register(*shortcut) {
            return match previous.as_ref() {
                Some(previous) => register(*previous).map_or_else(
                    |restore_error| {
                        Err(format!(
                            "Shortcut is unavailable: {error}; failed to restore previous shortcut: {restore_error}"
                        ))
                    },
                    |_| Err(format!("Shortcut is unavailable: {error}")),
                ),
                None => Err(format!("Shortcut is unavailable: {error}")),
            };
        }
    }

    if let Err(error) = persist() {
        let mut rollback_errors = Vec::new();
        if let Some(shortcut) = next.as_ref() {
            if let Err(rollback_error) = unregister(*shortcut) {
                rollback_errors.push(format!(
                    "failed to unregister new shortcut: {rollback_error}"
                ));
            }
        }
        if let Some(shortcut) = previous.as_ref() {
            if let Err(rollback_error) = register(*shortcut) {
                rollback_errors.push(format!(
                    "failed to restore previous shortcut: {rollback_error}"
                ));
            }
        }
        if rollback_errors.is_empty() {
            return Err(error);
        }
        return Err(format!("{error}; {}", rollback_errors.join("; ")));
    }

    Ok(())
}
