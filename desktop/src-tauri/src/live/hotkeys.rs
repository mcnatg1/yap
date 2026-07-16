mod interaction;
mod parser;

pub use interaction::{
    LiveShortcutAction, LiveShortcutInteraction, SHORTCUT_DOUBLE_TAP_MS, SHORTCUT_HOLD_MS,
};
pub(crate) use parser::configured_hotkeys_match;
pub use parser::{
    default_shortcut, normalize_hotkey, normalize_hotkey_for, parse_hotkey, parse_hotkey_for,
    HotkeyPurpose,
};

#[cfg(test)]
mod tests;
