use std::time::{Duration, Instant};

use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

use super::{settings::DEFAULT_HOTKEY, state::LiveCaptureMode};

pub const SHORTCUT_DOUBLE_TAP_MS: u64 = 320;
pub const SHORTCUT_HOLD_MS: u64 = 160;

pub fn parse_hotkey(input: &str) -> Result<Shortcut, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Shortcut cannot be empty.".into());
    }

    let mut modifiers = Modifiers::empty();
    let mut key = None;
    for part in trimmed
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmdorctrl" | "commandorcontrol" => {
                modifiers |= Modifiers::CONTROL;
            }
            "shift" => modifiers |= Modifiers::SHIFT,
            "alt" | "option" => modifiers |= Modifiers::ALT,
            "meta" | "super" | "cmd" | "command" => modifiers |= Modifiers::SUPER,
            _ if key.is_none() => key = Some(parse_code(part)?),
            _ => return Err("Shortcut must contain one key.".into()),
        }
    }

    let key = key.ok_or_else(|| "Shortcut must contain a key.".to_string())?;
    if modifiers.is_empty() {
        return Err("Shortcut needs at least one modifier.".into());
    }
    Ok(Shortcut::new(Some(modifiers), key))
}

fn parse_code(part: &str) -> Result<Code, String> {
    let upper = part.to_ascii_uppercase();
    match upper.as_str() {
        "SPACE" => Ok(Code::Space),
        "ESC" | "ESCAPE" => Ok(Code::Escape),
        "ENTER" | "RETURN" => Ok(Code::Enter),
        "TAB" => Ok(Code::Tab),
        "BACKSPACE" => Ok(Code::Backspace),
        value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphabetic() => {
            letter_code(value.as_bytes()[0] as char)
        }
        value if value.len() == 1 && value.as_bytes()[0].is_ascii_digit() => {
            digit_code(value.as_bytes()[0] as char)
        }
        value if value.starts_with('F') => function_key_code(value),
        _ => Err("Unsupported shortcut key.".into()),
    }
}

fn letter_code(letter: char) -> Result<Code, String> {
    Ok(match letter {
        'A' => Code::KeyA,
        'B' => Code::KeyB,
        'C' => Code::KeyC,
        'D' => Code::KeyD,
        'E' => Code::KeyE,
        'F' => Code::KeyF,
        'G' => Code::KeyG,
        'H' => Code::KeyH,
        'I' => Code::KeyI,
        'J' => Code::KeyJ,
        'K' => Code::KeyK,
        'L' => Code::KeyL,
        'M' => Code::KeyM,
        'N' => Code::KeyN,
        'O' => Code::KeyO,
        'P' => Code::KeyP,
        'Q' => Code::KeyQ,
        'R' => Code::KeyR,
        'S' => Code::KeyS,
        'T' => Code::KeyT,
        'U' => Code::KeyU,
        'V' => Code::KeyV,
        'W' => Code::KeyW,
        'X' => Code::KeyX,
        'Y' => Code::KeyY,
        'Z' => Code::KeyZ,
        _ => return Err("Unsupported shortcut key.".into()),
    })
}

fn digit_code(digit: char) -> Result<Code, String> {
    Ok(match digit {
        '0' => Code::Digit0,
        '1' => Code::Digit1,
        '2' => Code::Digit2,
        '3' => Code::Digit3,
        '4' => Code::Digit4,
        '5' => Code::Digit5,
        '6' => Code::Digit6,
        '7' => Code::Digit7,
        '8' => Code::Digit8,
        '9' => Code::Digit9,
        _ => return Err("Unsupported shortcut key.".into()),
    })
}

fn function_key_code(value: &str) -> Result<Code, String> {
    Ok(match value {
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        _ => return Err("Unsupported function key.".into()),
    })
}

pub fn default_shortcut() -> Shortcut {
    parse_hotkey(DEFAULT_HOTKEY).expect("default live hotkey must be valid")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveShortcutAction {
    None,
    ScheduleHold(u64),
    Start(LiveCaptureMode),
    Stop,
}

#[derive(Debug, Default)]
pub struct LiveShortcutInteraction {
    ignore_next_release: bool,
    key_down: bool,
    last_tap_at: Option<Instant>,
    pending_press_at: Option<Instant>,
    pending_press_id: u64,
    starting_push_to_talk: bool,
    stop_push_to_talk_after_start: bool,
}

impl LiveShortcutInteraction {
    pub fn reset(&mut self) {
        self.ignore_next_release = false;
        self.key_down = false;
        self.last_tap_at = None;
        self.pending_press_at = None;
        self.starting_push_to_talk = false;
        self.stop_push_to_talk_after_start = false;
    }

    pub fn finish_push_to_talk_start(&mut self) -> bool {
        self.starting_push_to_talk = false;
        std::mem::take(&mut self.stop_push_to_talk_after_start)
    }

    pub fn pressed(
        &mut self,
        now: Instant,
        active_mode: Option<LiveCaptureMode>,
    ) -> LiveShortcutAction {
        if self.key_down {
            return LiveShortcutAction::None;
        }
        self.key_down = true;
        if active_mode == Some(LiveCaptureMode::Toggle) {
            self.ignore_next_release = true;
            self.pending_press_at = None;
            self.last_tap_at = None;
            return LiveShortcutAction::Stop;
        }
        if active_mode.is_some() {
            return LiveShortcutAction::None;
        }
        if self.last_tap_at.is_some_and(|then| {
            now.duration_since(then) <= Duration::from_millis(SHORTCUT_DOUBLE_TAP_MS)
        }) {
            self.pending_press_at = None;
            self.last_tap_at = None;
            return LiveShortcutAction::Start(LiveCaptureMode::Toggle);
        }

        self.pending_press_id = self.pending_press_id.wrapping_add(1);
        self.pending_press_at = Some(now);
        self.last_tap_at = None;
        LiveShortcutAction::ScheduleHold(self.pending_press_id)
    }

    pub fn hold_elapsed(
        &mut self,
        press_id: u64,
        now: Instant,
        active_mode: Option<LiveCaptureMode>,
    ) -> LiveShortcutAction {
        let Some(pressed_at) = self.pending_press_at else {
            return LiveShortcutAction::None;
        };
        if press_id != self.pending_press_id
            || active_mode.is_some()
            || now.duration_since(pressed_at) < Duration::from_millis(SHORTCUT_HOLD_MS)
        {
            return LiveShortcutAction::None;
        }

        self.pending_press_at = None;
        self.last_tap_at = None;
        self.starting_push_to_talk = true;
        LiveShortcutAction::Start(LiveCaptureMode::PushToTalk)
    }

    pub fn released(
        &mut self,
        now: Instant,
        active_mode: Option<LiveCaptureMode>,
    ) -> LiveShortcutAction {
        self.key_down = false;
        if self.ignore_next_release {
            self.ignore_next_release = false;
            return LiveShortcutAction::None;
        }
        if active_mode == Some(LiveCaptureMode::PushToTalk) {
            return LiveShortcutAction::Stop;
        }
        if active_mode == Some(LiveCaptureMode::Toggle) {
            return LiveShortcutAction::None;
        }
        if self.starting_push_to_talk {
            self.stop_push_to_talk_after_start = true;
            return LiveShortcutAction::None;
        }
        if self.pending_press_at.take().is_some() {
            self.last_tap_at = Some(now);
        }
        LiveShortcutAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn shortcut_double_tap_starts_hands_free_and_release_is_ignored() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.released(now + Duration::from_millis(40), None),
            LiveShortcutAction::None
        );
        assert_eq!(
            shortcut.pressed(now + Duration::from_millis(120), None),
            LiveShortcutAction::Start(LiveCaptureMode::Toggle)
        );
        assert_eq!(
            shortcut.released(
                now + Duration::from_millis(150),
                Some(LiveCaptureMode::Toggle),
            ),
            LiveShortcutAction::None
        );
        assert_eq!(
            shortcut.pressed(
                now + Duration::from_millis(240),
                Some(LiveCaptureMode::Toggle),
            ),
            LiveShortcutAction::Stop
        );
        assert_eq!(
            shortcut.released(now + Duration::from_millis(260), None),
            LiveShortcutAction::None
        );
    }

    #[test]
    fn shortcut_reset_clears_stale_tap_state() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.released(now + Duration::from_millis(40), None),
            LiveShortcutAction::None
        );
        shortcut.reset();

        assert_eq!(
            shortcut.pressed(now + Duration::from_millis(120), None),
            LiveShortcutAction::ScheduleHold(2)
        );
    }

    #[test]
    fn shortcut_ignores_repeated_pressed_events_until_release() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.pressed(now + Duration::from_millis(20), None),
            LiveShortcutAction::None
        );
        assert_eq!(
            shortcut.hold_elapsed(1, now + Duration::from_millis(SHORTCUT_HOLD_MS + 1), None,),
            LiveShortcutAction::Start(LiveCaptureMode::PushToTalk)
        );
    }

    #[test]
    fn shortcut_release_during_push_to_talk_start_requests_stop_after_start() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.hold_elapsed(1, now + Duration::from_millis(SHORTCUT_HOLD_MS + 1), None,),
            LiveShortcutAction::Start(LiveCaptureMode::PushToTalk)
        );
        assert_eq!(
            shortcut.released(now + Duration::from_millis(180), None),
            LiveShortcutAction::None
        );
        assert!(shortcut.finish_push_to_talk_start());
    }

    #[test]
    fn shortcut_hold_starts_push_to_talk_and_release_stops() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.hold_elapsed(1, now + Duration::from_millis(SHORTCUT_HOLD_MS + 1), None,),
            LiveShortcutAction::Start(LiveCaptureMode::PushToTalk)
        );
        assert_eq!(
            shortcut.released(
                now + Duration::from_millis(260),
                Some(LiveCaptureMode::PushToTalk),
            ),
            LiveShortcutAction::Stop
        );
    }

    #[test]
    fn shortcut_single_tap_does_not_start_recording() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.released(now + Duration::from_millis(40), None),
            LiveShortcutAction::None
        );
        assert_eq!(
            shortcut.hold_elapsed(1, now + Duration::from_millis(SHORTCUT_HOLD_MS + 1), None,),
            LiveShortcutAction::None
        );
    }
}
