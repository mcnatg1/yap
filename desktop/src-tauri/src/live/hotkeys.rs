use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

use super::settings::DEFAULT_HOTKEY;

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
}
