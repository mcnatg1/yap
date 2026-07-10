use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tauri_plugin_global_shortcut::Shortcut;

use crate::live;

pub(crate) const DICTATION_UNAVAILABLE_ERROR: &str = "Live shortcut is unavailable.";
pub(crate) const PASTE_UNAVAILABLE_ERROR: &str = "Paste shortcut is unavailable.";

#[derive(Clone, Copy)]
enum LiveHotkeyKind {
    Dictation,
    PasteLast,
}

impl LiveHotkeyKind {
    fn current(self, view: &live::state::LiveSessionView) -> &str {
        match self {
            Self::Dictation => &view.hotkey,
            Self::PasteLast => &view.paste_hotkey,
        }
    }

    fn conflicting(self, view: &live::state::LiveSessionView) -> &str {
        match self {
            Self::Dictation => &view.paste_hotkey,
            Self::PasteLast => &view.hotkey,
        }
    }

    fn conflict_message(self) -> &'static str {
        match self {
            Self::Dictation => "Dictation shortcut must differ from paste shortcut.",
            Self::PasteLast => "Paste shortcut must differ from dictation shortcut.",
        }
    }

    fn update(self, view: &mut live::state::LiveSessionView, hotkey: String) {
        match self {
            Self::Dictation => view.hotkey = hotkey,
            Self::PasteLast => view.paste_hotkey = hotkey,
        }
    }

    fn startup_error(self) -> &'static str {
        match self {
            Self::Dictation => DICTATION_UNAVAILABLE_ERROR,
            Self::PasteLast => PASTE_UNAVAILABLE_ERROR,
        }
    }

    fn is_paste(self) -> bool {
        matches!(self, Self::PasteLast)
    }
}

fn apply_successful_hotkey_change(
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

#[tauri::command]
pub(crate) fn set_live_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    hotkey: String,
) -> Result<live::state::LiveSessionView, String> {
    change_live_hotkey(window, app, state, LiveHotkeyKind::Dictation, Some(hotkey))
}

#[tauri::command]
pub(crate) fn clear_live_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    change_live_hotkey(window, app, state, LiveHotkeyKind::Dictation, None)
}

#[tauri::command]
pub(crate) fn set_live_paste_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    hotkey: String,
) -> Result<live::state::LiveSessionView, String> {
    change_live_hotkey(window, app, state, LiveHotkeyKind::PasteLast, Some(hotkey))
}

#[tauri::command]
pub(crate) fn clear_live_paste_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    change_live_hotkey(window, app, state, LiveHotkeyKind::PasteLast, None)
}

fn change_live_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    kind: LiveHotkeyKind,
    hotkey: Option<String>,
) -> Result<live::state::LiveSessionView, String> {
    crate::ensure_main_command(&window)?;
    ensure_live_hotkey_idle(state.snapshot().status)?;

    let snapshot = state.snapshot();
    let next_value = hotkey
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let next = if next_value.is_empty() {
        None
    } else {
        Some(live::hotkeys::parse_hotkey(&next_value)?)
    };
    if live::hotkeys::configured_hotkeys_match(kind.conflicting(&snapshot), &next_value) {
        return Err(kind.conflict_message().into());
    }

    let previous = live::hotkeys::parse_hotkey(kind.current(&snapshot)).ok();
    let mut prospective = snapshot.clone();
    kind.update(&mut prospective, next_value.clone());
    replace_hotkey_registration(
        previous,
        next,
        |shortcut| {
            app.global_shortcut()
                .unregister(shortcut)
                .map_err(|error| format!("Failed to unregister previous shortcut: {error}"))
        },
        |shortcut| {
            app.global_shortcut()
                .register(shortcut)
                .map_err(|error| error.to_string())
        },
        || crate::persist_live_view(&prospective),
    )?;

    let recovered_startup_failure = state.take_startup_shortcut_failure(kind.is_paste());
    let view = state.update(|view| {
        apply_successful_hotkey_change(view, kind, next_value, recovered_startup_failure);
    });
    crate::emit_live(&app, &view);
    Ok(view)
}

fn ensure_live_hotkey_idle(status: live::state::LiveSessionStatus) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::{
        apply_successful_hotkey_change, ensure_live_hotkey_idle, replace_hotkey_registration,
        LiveHotkeyKind, DICTATION_UNAVAILABLE_ERROR,
    };
    use crate::live::{
        hotkeys::parse_hotkey,
        settings::LiveSettings,
        state::{LiveRoute, LiveSessionStatus, LiveSessionView},
    };

    #[test]
    fn successful_replacement_clears_matching_startup_failure() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.hotkey.clear();
        view.error = Some("Live shortcut is unavailable.".into());
        view.route = LiveRoute::Blocked;
        view.status = LiveSessionStatus::Blocked;

        apply_successful_hotkey_change(
            &mut view,
            LiveHotkeyKind::Dictation,
            "Ctrl+Shift+D".into(),
            true,
        );

        assert_eq!(view.hotkey, "Ctrl+Shift+D");
        assert_eq!(view.error, None);
        assert_eq!(view.route, LiveRoute::None);
        assert_eq!(view.status, LiveSessionStatus::Idle);
    }

    #[test]
    fn successful_replacement_preserves_unrelated_block() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.error = Some("Local model is unavailable.".into());
        view.route = LiveRoute::Blocked;
        view.status = LiveSessionStatus::Blocked;

        apply_successful_hotkey_change(
            &mut view,
            LiveHotkeyKind::Dictation,
            "Ctrl+Shift+D".into(),
            false,
        );

        assert_eq!(view.error.as_deref(), Some("Local model is unavailable."));
        assert_eq!(view.route, LiveRoute::Blocked);
        assert_eq!(view.status, LiveSessionStatus::Blocked);
    }

    #[test]
    fn startup_recovery_does_not_depend_on_mutable_error_copy() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.hotkey.clear();
        view.error = Some("Selected microphone unavailable. Using default.".into());
        view.route = LiveRoute::Blocked;
        view.status = LiveSessionStatus::Blocked;

        apply_successful_hotkey_change(
            &mut view,
            LiveHotkeyKind::Dictation,
            "Ctrl+Shift+D".into(),
            true,
        );

        assert_eq!(
            view.error.as_deref(),
            Some("Selected microphone unavailable. Using default.")
        );
        assert_eq!(view.route, LiveRoute::None);
        assert_eq!(view.status, LiveSessionStatus::Idle);
    }

    #[test]
    fn replacing_both_failed_shortcuts_clears_dictation_block_last() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.hotkey.clear();
        view.paste_hotkey.clear();
        view.error = Some(DICTATION_UNAVAILABLE_ERROR.into());
        view.route = LiveRoute::Blocked;
        view.status = LiveSessionStatus::Blocked;

        apply_successful_hotkey_change(
            &mut view,
            LiveHotkeyKind::PasteLast,
            "Ctrl+Shift+V".into(),
            true,
        );
        assert_eq!(view.error.as_deref(), Some(DICTATION_UNAVAILABLE_ERROR));
        assert_eq!(view.status, LiveSessionStatus::Blocked);

        apply_successful_hotkey_change(
            &mut view,
            LiveHotkeyKind::Dictation,
            "Ctrl+Shift+D".into(),
            true,
        );
        assert_eq!(view.error, None);
        assert_eq!(view.route, LiveRoute::None);
        assert_eq!(view.status, LiveSessionStatus::Idle);
    }

    #[test]
    fn persistence_failure_restores_previous_registration() {
        let events = RefCell::new(Vec::new());
        let previous = parse_hotkey("Ctrl+Shift+Space").unwrap();
        let next = parse_hotkey("Ctrl+Shift+V").unwrap();

        let result = replace_hotkey_registration(
            Some(previous),
            Some(next),
            |shortcut| {
                events.borrow_mut().push(format!("unregister:{shortcut:?}"));
                Ok(())
            },
            |shortcut| {
                events.borrow_mut().push(format!("register:{shortcut:?}"));
                Ok(())
            },
            || {
                events.borrow_mut().push("persist".into());
                Err("disk full".into())
            },
        );

        assert!(result.unwrap_err().contains("disk full"));
        assert_eq!(
            events
                .borrow()
                .iter()
                .map(|event| event.split(':').next().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "unregister",
                "register",
                "persist",
                "unregister",
                "register"
            ]
        );
    }

    #[test]
    fn failed_new_registration_restores_previous_registration_without_persisting() {
        let events = RefCell::new(Vec::new());
        let previous = parse_hotkey("Ctrl+Shift+Space").unwrap();
        let next = parse_hotkey("Ctrl+Shift+V").unwrap();
        let register_attempt = RefCell::new(0usize);

        let result = replace_hotkey_registration(
            Some(previous),
            Some(next),
            |shortcut| {
                events.borrow_mut().push(format!("unregister:{shortcut:?}"));
                Ok(())
            },
            |shortcut| {
                events.borrow_mut().push(format!("register:{shortcut:?}"));
                let mut attempt = register_attempt.borrow_mut();
                *attempt += 1;
                if *attempt == 1 {
                    Err("reserved".into())
                } else {
                    Ok(())
                }
            },
            || panic!("persistence must not run after registration failure"),
        );

        assert!(result.unwrap_err().contains("reserved"));
        assert_eq!(
            events
                .borrow()
                .iter()
                .map(|event| event.split(':').next().unwrap())
                .collect::<Vec<_>>(),
            vec!["unregister", "register", "register"]
        );
    }

    #[test]
    fn hotkey_mutation_is_idle_only() {
        use crate::live::state::LiveSessionStatus;

        for status in [
            LiveSessionStatus::Armed,
            LiveSessionStatus::Listening,
            LiveSessionStatus::Speaking,
            LiveSessionStatus::Settling,
            LiveSessionStatus::Saving,
        ] {
            assert_eq!(
                ensure_live_hotkey_idle(status),
                Err("Stop live before changing the shortcut.".into())
            );
        }
        for status in [LiveSessionStatus::Idle, LiveSessionStatus::Blocked] {
            assert_eq!(ensure_live_hotkey_idle(status), Ok(()));
        }
    }
}
