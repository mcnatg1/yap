use super::super::*;
use crate::live::settings::{DEFAULT_HOTKEY, DEFAULT_PASTE_HOTKEY};

#[test]
fn parses_default_hotkey() {
    assert_eq!(parse_hotkey(DEFAULT_HOTKEY), Ok(default_shortcut()));
}

#[test]
fn rejects_empty_or_modifier_only_hotkeys() {
    assert!(parse_hotkey("").is_err());
    assert!(parse_hotkey("Ctrl+Shift").is_err());
}

#[test]
fn rejects_ordinary_single_modifier_dictation_chords() {
    for hotkey in ["Ctrl+C", "Ctrl+V", "Ctrl+S", "Alt+Enter"] {
        assert_eq!(
            parse_hotkey_for(hotkey, HotkeyPurpose::Dictation),
            Err("Dictation shortcut needs at least two modifier keys.".into()),
            "{hotkey}"
        );
    }
    assert!(parse_hotkey_for(DEFAULT_HOTKEY, HotkeyPurpose::Dictation).is_ok());
}

#[test]
fn paste_last_requires_a_three_modifier_deliberate_chord() {
    for hotkey in ["Ctrl+V", "Ctrl+Shift+V", "Alt+Shift+V"] {
        assert!(
            parse_hotkey_for(hotkey, HotkeyPurpose::PasteLast).is_err(),
            "{hotkey}"
        );
    }
    assert!(parse_hotkey_for(DEFAULT_PASTE_HOTKEY, HotkeyPurpose::PasteLast).is_ok());
}

#[test]
fn rejects_windows_reserved_hotkeys_before_registration() {
    for hotkey in [
        "Alt+F4",
        "Alt+Tab",
        "Ctrl+Alt+Tab",
        "Alt+Space",
        "Alt+Escape",
        "Ctrl+Escape",
        "Ctrl+Shift+Escape",
        "Meta+L",
        "Ctrl+Meta+7",
        "Super+R",
        "Ctrl+F12",
    ] {
        assert_eq!(
            parse_hotkey(hotkey),
            Err("Shortcut is reserved by Windows.".into()),
            "{hotkey}"
        );
    }
}

#[test]
fn normalizes_aliases_and_modifier_order_before_persistence() {
    assert_eq!(
        normalize_hotkey(" option + control + shift + v "),
        Ok("Ctrl+Shift+Alt+V".into())
    );
    assert_eq!(
        normalize_hotkey("cmd+f1"),
        Err("Shortcut is reserved by Windows.".into())
    );
}
