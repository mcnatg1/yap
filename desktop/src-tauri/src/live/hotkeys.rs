use std::time::{Duration, Instant};

use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

use super::{settings::DEFAULT_HOTKEY, state::LiveCaptureMode};

pub const SHORTCUT_DOUBLE_TAP_MS: u64 = 320;
pub const SHORTCUT_HOLD_MS: u64 = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyPurpose {
    Dictation,
    PasteLast,
}

pub fn parse_hotkey(input: &str) -> Result<Shortcut, String> {
    parse_hotkey_for(input, HotkeyPurpose::Dictation)
}

pub fn parse_hotkey_for(input: &str, purpose: HotkeyPurpose) -> Result<Shortcut, String> {
    let (modifiers, key) = parse_hotkey_parts(input, purpose)?;
    Ok(Shortcut::new(Some(modifiers), key))
}

pub fn normalize_hotkey(input: &str) -> Result<String, String> {
    normalize_hotkey_for(input, HotkeyPurpose::Dictation)
}

pub fn normalize_hotkey_for(input: &str, purpose: HotkeyPurpose) -> Result<String, String> {
    let (modifiers, key) = parse_hotkey_parts(input, purpose)?;
    let mut parts = Vec::with_capacity(5);
    if modifiers.contains(Modifiers::CONTROL) {
        parts.push("Ctrl");
    }
    if modifiers.contains(Modifiers::SHIFT) {
        parts.push("Shift");
    }
    if modifiers.contains(Modifiers::ALT) {
        parts.push("Alt");
    }
    if modifiers.contains(Modifiers::SUPER) {
        parts.push("Meta");
    }
    parts.push(code_name(key)?);
    Ok(parts.join("+"))
}

fn parse_hotkey_parts(input: &str, purpose: HotkeyPurpose) -> Result<(Modifiers, Code), String> {
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
    if is_windows_reserved(modifiers, key) {
        return Err("Shortcut is reserved by Windows.".into());
    }
    let modifier_count = modifiers.bits().count_ones();
    let required_modifiers = match purpose {
        HotkeyPurpose::Dictation => 2,
        HotkeyPurpose::PasteLast => 3,
    };
    if modifier_count < required_modifiers {
        return Err(match purpose {
            HotkeyPurpose::Dictation => {
                "Dictation shortcut needs at least two modifier keys.".into()
            }
            HotkeyPurpose::PasteLast => {
                "Paste-last shortcut needs at least three modifier keys.".into()
            }
        });
    }
    Ok((modifiers, key))
}

fn is_windows_reserved(modifiers: Modifiers, key: Code) -> bool {
    key == Code::F12
        || modifiers.contains(Modifiers::SUPER)
        || (modifiers.contains(Modifiers::ALT)
            && matches!(key, Code::F4 | Code::Tab | Code::Space | Code::Escape))
        || (modifiers.contains(Modifiers::CONTROL) && key == Code::Escape)
}

pub(crate) fn configured_hotkeys_match(left: &str, right: &str) -> bool {
    !left.trim().is_empty()
        && !right.trim().is_empty()
        && parse_hotkey(left)
            .and_then(|left| parse_hotkey(right).map(|right| left == right))
            .unwrap_or(false)
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

fn code_name(code: Code) -> Result<&'static str, String> {
    Ok(match code {
        Code::Space => "Space",
        Code::Escape => "Escape",
        Code::Enter => "Enter",
        Code::Tab => "Tab",
        Code::Backspace => "Backspace",
        Code::KeyA => "A",
        Code::KeyB => "B",
        Code::KeyC => "C",
        Code::KeyD => "D",
        Code::KeyE => "E",
        Code::KeyF => "F",
        Code::KeyG => "G",
        Code::KeyH => "H",
        Code::KeyI => "I",
        Code::KeyJ => "J",
        Code::KeyK => "K",
        Code::KeyL => "L",
        Code::KeyM => "M",
        Code::KeyN => "N",
        Code::KeyO => "O",
        Code::KeyP => "P",
        Code::KeyQ => "Q",
        Code::KeyR => "R",
        Code::KeyS => "S",
        Code::KeyT => "T",
        Code::KeyU => "U",
        Code::KeyV => "V",
        Code::KeyW => "W",
        Code::KeyX => "X",
        Code::KeyY => "Y",
        Code::KeyZ => "Z",
        Code::Digit0 => "0",
        Code::Digit1 => "1",
        Code::Digit2 => "2",
        Code::Digit3 => "3",
        Code::Digit4 => "4",
        Code::Digit5 => "5",
        Code::Digit6 => "6",
        Code::Digit7 => "7",
        Code::Digit8 => "8",
        Code::Digit9 => "9",
        Code::F1 => "F1",
        Code::F2 => "F2",
        Code::F3 => "F3",
        Code::F4 => "F4",
        Code::F5 => "F5",
        Code::F6 => "F6",
        Code::F7 => "F7",
        Code::F8 => "F8",
        Code::F9 => "F9",
        Code::F10 => "F10",
        Code::F11 => "F11",
        Code::F12 => "F12",
        _ => return Err("Unsupported shortcut key.".into()),
    })
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
    active_mode: Option<LiveCaptureMode>,
    key_down: bool,
    last_tap_at: Option<Instant>,
    pending_press_at: Option<Instant>,
    pending_press_id: u64,
    pending_start_mode: Option<LiveCaptureMode>,
    suppress_release: bool,
    toggle_stop_armed: bool,
}

impl LiveShortcutInteraction {
    pub fn reset(&mut self) {
        self.active_mode = None;
        self.key_down = false;
        self.last_tap_at = None;
        self.pending_press_at = None;
        self.pending_start_mode = None;
        self.suppress_release = false;
        self.toggle_stop_armed = false;
    }

    pub fn finish_start(&mut self, active_mode: Option<LiveCaptureMode>) {
        let Some(started_mode) = self.pending_start_mode.take() else {
            return;
        };
        if self.active_mode == Some(started_mode) && active_mode != Some(started_mode) {
            self.clear_active_session();
            self.suppress_release = self.key_down;
        }
    }

    pub fn pressed(
        &mut self,
        now: Instant,
        projected_mode: Option<LiveCaptureMode>,
    ) -> LiveShortcutAction {
        self.reconcile(projected_mode);
        if self.key_down {
            return LiveShortcutAction::None;
        }
        self.key_down = true;
        match self.active_mode {
            Some(LiveCaptureMode::Toggle) if self.toggle_stop_armed => {
                self.clear_active_session();
                self.suppress_release = true;
                self.pending_press_at = None;
                self.last_tap_at = None;
                return LiveShortcutAction::Stop;
            }
            Some(_) => return LiveShortcutAction::None,
            None => {}
        }
        if self.last_tap_at.is_some_and(|then| {
            now.duration_since(then) <= Duration::from_millis(SHORTCUT_DOUBLE_TAP_MS)
        }) {
            self.pending_press_at = None;
            self.last_tap_at = None;
            self.begin_session(LiveCaptureMode::Toggle);
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
        projected_mode: Option<LiveCaptureMode>,
    ) -> LiveShortcutAction {
        self.reconcile(projected_mode);
        let Some(pressed_at) = self.pending_press_at else {
            return LiveShortcutAction::None;
        };
        if press_id != self.pending_press_id
            || self.active_mode.is_some()
            || now.duration_since(pressed_at) < Duration::from_millis(SHORTCUT_HOLD_MS)
        {
            return LiveShortcutAction::None;
        }

        self.pending_press_at = None;
        self.last_tap_at = None;
        self.begin_session(LiveCaptureMode::PushToTalk);
        LiveShortcutAction::Start(LiveCaptureMode::PushToTalk)
    }

    pub fn released(
        &mut self,
        now: Instant,
        projected_mode: Option<LiveCaptureMode>,
    ) -> LiveShortcutAction {
        self.reconcile(projected_mode);
        self.key_down = false;
        if self.suppress_release {
            self.suppress_release = false;
            return LiveShortcutAction::None;
        }
        match self.active_mode {
            Some(LiveCaptureMode::PushToTalk) => {
                self.clear_active_session();
                return LiveShortcutAction::Stop;
            }
            Some(LiveCaptureMode::Toggle) => {
                self.toggle_stop_armed = true;
                return LiveShortcutAction::None;
            }
            None => {}
        }
        if self.pending_press_at.take().is_some() {
            self.last_tap_at = Some(now);
        }
        LiveShortcutAction::None
    }

    fn begin_session(&mut self, mode: LiveCaptureMode) {
        self.active_mode = Some(mode);
        self.pending_start_mode = Some(mode);
        self.toggle_stop_armed = false;
    }

    fn clear_active_session(&mut self) {
        self.active_mode = None;
        self.toggle_stop_armed = false;
    }

    fn reconcile(&mut self, projected_mode: Option<LiveCaptureMode>) {
        if self.pending_start_mode.is_some() || self.suppress_release {
            return;
        }
        match (self.active_mode, projected_mode) {
            (None, Some(mode)) => {
                self.active_mode = Some(mode);
                self.toggle_stop_armed = mode == LiveCaptureMode::Toggle;
            }
            (Some(active), Some(projected)) if active != projected => {
                self.active_mode = Some(projected);
                self.toggle_stop_armed = projected == LiveCaptureMode::Toggle;
            }
            (Some(_), None) => self.clear_active_session(),
            _ => {}
        }
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
        assert!(parse_hotkey_for(
            super::super::settings::DEFAULT_PASTE_HOTKEY,
            HotkeyPurpose::PasteLast
        )
        .is_ok());
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
            shortcut.released(now + Duration::from_millis(150), None),
            LiveShortcutAction::None
        );
        assert_eq!(
            shortcut.pressed(now + Duration::from_millis(240), None),
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
    fn shortcut_release_during_push_to_talk_start_stops_without_waiting_for_projection() {
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
            LiveShortcutAction::Stop
        );
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

    #[test]
    fn projected_session_end_clears_owned_mode_after_start_is_acknowledged() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        assert_eq!(
            shortcut.released(now + Duration::from_millis(30), None),
            LiveShortcutAction::None
        );
        assert_eq!(
            shortcut.pressed(now + Duration::from_millis(90), None),
            LiveShortcutAction::Start(LiveCaptureMode::Toggle)
        );
        assert_eq!(
            shortcut.released(now + Duration::from_millis(120), None),
            LiveShortcutAction::None
        );
        shortcut.finish_start(Some(LiveCaptureMode::Toggle));

        assert_eq!(
            shortcut.pressed(now + Duration::from_millis(500), None),
            LiveShortcutAction::ScheduleHold(2)
        );
    }

    #[test]
    fn failed_toggle_start_clears_the_toggle_stop_latch() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        shortcut.released(now + Duration::from_millis(25), None);
        assert_eq!(
            shortcut.pressed(now + Duration::from_millis(75), None),
            LiveShortcutAction::Start(LiveCaptureMode::Toggle)
        );
        shortcut.released(now + Duration::from_millis(100), None);
        shortcut.finish_start(None);

        assert_eq!(
            shortcut.pressed(now + Duration::from_millis(450), None),
            LiveShortcutAction::ScheduleHold(2)
        );
    }

    #[test]
    fn projected_modes_stop_with_their_own_contract_without_cross_mode_taps() {
        let mut push_to_talk = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            push_to_talk.pressed(now, Some(LiveCaptureMode::PushToTalk)),
            LiveShortcutAction::None
        );
        assert_eq!(
            push_to_talk.released(
                now + Duration::from_millis(20),
                Some(LiveCaptureMode::PushToTalk),
            ),
            LiveShortcutAction::Stop
        );
        assert_eq!(
            push_to_talk.released(now + Duration::from_millis(40), None),
            LiveShortcutAction::None
        );

        let mut toggle = LiveShortcutInteraction::default();
        assert_eq!(
            toggle.pressed(now, Some(LiveCaptureMode::Toggle)),
            LiveShortcutAction::Stop
        );
        assert_eq!(
            toggle.released(now + Duration::from_millis(20), None),
            LiveShortcutAction::None
        );
    }

    #[test]
    fn delayed_hold_timer_cannot_convert_a_double_tap_into_push_to_talk() {
        let mut shortcut = LiveShortcutInteraction::default();
        let now = Instant::now();

        assert_eq!(
            shortcut.pressed(now, None),
            LiveShortcutAction::ScheduleHold(1)
        );
        shortcut.released(now + Duration::from_millis(30), None);
        assert_eq!(
            shortcut.pressed(now + Duration::from_millis(80), None),
            LiveShortcutAction::Start(LiveCaptureMode::Toggle)
        );
        assert_eq!(
            shortcut.hold_elapsed(1, now + Duration::from_millis(SHORTCUT_HOLD_MS + 40), None,),
            LiveShortcutAction::None
        );
    }
}
