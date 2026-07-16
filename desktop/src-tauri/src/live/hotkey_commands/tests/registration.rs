use std::cell::RefCell;

use crate::live::{
    hotkey_commands::{
        kind::LiveHotkeyKind,
        registration::{
            apply_successful_hotkey_change, ensure_live_hotkey_idle, replace_hotkey_registration,
        },
        DICTATION_UNAVAILABLE_ERROR,
    },
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
