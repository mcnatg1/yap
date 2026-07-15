use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

use crate::live::settings::DEFAULT_HOTKEY;

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
