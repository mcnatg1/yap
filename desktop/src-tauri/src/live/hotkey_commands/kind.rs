use crate::live;

use super::{DICTATION_UNAVAILABLE_ERROR, PASTE_UNAVAILABLE_ERROR};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LiveHotkeyKind {
    Dictation,
    PasteLast,
}

impl LiveHotkeyKind {
    pub(super) fn current(self, view: &live::state::LiveSessionView) -> &str {
        match self {
            Self::Dictation => &view.hotkey,
            Self::PasteLast => &view.paste_hotkey,
        }
    }

    pub(super) fn conflicting(self, view: &live::state::LiveSessionView) -> &str {
        match self {
            Self::Dictation => &view.paste_hotkey,
            Self::PasteLast => &view.hotkey,
        }
    }

    pub(super) fn conflict_message(self) -> &'static str {
        match self {
            Self::Dictation => "Dictation shortcut must differ from paste shortcut.",
            Self::PasteLast => "Paste shortcut must differ from dictation shortcut.",
        }
    }

    pub(super) fn update(self, view: &mut live::state::LiveSessionView, hotkey: String) {
        match self {
            Self::Dictation => view.hotkey = hotkey,
            Self::PasteLast => view.paste_hotkey = hotkey,
        }
    }

    pub(super) fn startup_error(self) -> &'static str {
        match self {
            Self::Dictation => DICTATION_UNAVAILABLE_ERROR,
            Self::PasteLast => PASTE_UNAVAILABLE_ERROR,
        }
    }

    pub(super) fn is_paste(self) -> bool {
        matches!(self, Self::PasteLast)
    }

    pub(super) fn purpose(self) -> live::hotkeys::HotkeyPurpose {
        match self {
            Self::Dictation => live::hotkeys::HotkeyPurpose::Dictation,
            Self::PasteLast => live::hotkeys::HotkeyPurpose::PasteLast,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Dictation => "dictation",
            Self::PasteLast => "paste-last",
        }
    }

    pub(super) fn required_modifier_count(self) -> u32 {
        match self {
            Self::Dictation => 2,
            Self::PasteLast => 3,
        }
    }
}
