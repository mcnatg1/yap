#[cfg(target_os = "windows")]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, VIRTUAL_KEY,
};

pub fn paste_text(text: &str) -> Result<(), String> {
    let Some(text) = pasteable_text(text) else {
        return Ok(());
    };
    write_clipboard(&text)?;
    send_paste_keystroke()
}

pub(crate) fn pasteable_text(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(target_os = "windows")]
fn write_clipboard(text: &str) -> Result<(), String> {
    use std::ptr::copy_nonoverlapping;

    use windows::Win32::{
        Foundation::HANDLE,
        System::{
            DataExchange::{EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
        },
    };

    const CF_UNICODETEXT: u32 = 13;
    let mut wide = text.encode_utf16().collect::<Vec<_>>();
    wide.push(0);
    let byte_len = wide.len() * std::mem::size_of::<u16>();

    unsafe {
        OpenClipboard(None).map_err(|err| format!("Failed to open clipboard: {err}"))?;
        let _guard = ClipboardGuard;
        EmptyClipboard().map_err(|err| format!("Failed to clear clipboard: {err}"))?;
        let handle = GlobalAlloc(GMEM_MOVEABLE, byte_len)
            .map_err(|err| format!("Failed to allocate clipboard memory: {err}"))?;
        let buffer = GlobalLock(handle).cast::<u16>();
        if buffer.is_null() {
            return Err("Failed to lock clipboard memory.".into());
        }
        copy_nonoverlapping(wide.as_ptr(), buffer, wide.len());
        let _ = GlobalUnlock(handle);
        SetClipboardData(CF_UNICODETEXT, Some(HANDLE(handle.0)))
            .map_err(|err| format!("Failed to write clipboard: {err}"))?;
    }

    Ok(())
}

#[cfg(target_os = "windows")]
struct ClipboardGuard;

#[cfg(target_os = "windows")]
impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::DataExchange::CloseClipboard();
        }
    }
}

#[cfg(target_os = "windows")]
fn send_paste_keystroke() -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, KEYEVENTF_KEYUP, VK_CONTROL, VK_V,
    };

    std::thread::sleep(std::time::Duration::from_millis(60));

    let inputs = [
        keyboard_input(VK_CONTROL, KEYBD_EVENT_FLAGS(0)),
        keyboard_input(VK_V, KEYBD_EVENT_FLAGS(0)),
        keyboard_input(VK_V, KEYEVENTF_KEYUP),
        keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        return Err("Failed to send paste keystroke.".into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn keyboard_input(key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(not(target_os = "windows"))]
fn write_clipboard(_text: &str) -> Result<(), String> {
    Err("Paste is only available on Windows.".into())
}

#[cfg(not(target_os = "windows"))]
fn send_paste_keystroke() -> Result<(), String> {
    Err("Paste is only available on Windows.".into())
}

#[cfg(test)]
mod tests {
    use super::pasteable_text;

    #[test]
    fn pasteable_text_trims_empty_input() {
        assert_eq!(pasteable_text("  "), None);
        assert_eq!(pasteable_text("  hello\n"), Some("hello".into()));
    }
}
