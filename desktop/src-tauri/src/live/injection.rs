use tauri::Manager;

#[cfg(target_os = "windows")]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, VIRTUAL_KEY,
};

const CF_UNICODETEXT: u32 = 13;
const CLIPBOARD_OPEN_ATTEMPTS: usize = 6;
const CLIPBOARD_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(10);
const MODIFIER_RELEASE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(300);
const MODIFIER_POLL_DELAY: std::time::Duration = std::time::Duration::from_millis(5);
const UNICODE_INPUT_CHUNK: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InjectionOutcome {
    Ignored,
    Injected,
    CopiedOnly(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectionTarget {
    focused_control: Option<isize>,
    window: isize,
    process_id: u32,
}

pub fn inject_text(
    app: &tauri::AppHandle,
    target: Option<InjectionTarget>,
    text: &str,
) -> Result<InjectionOutcome, String> {
    inject_text_with(
        text,
        |text| inject_into_target(target, text),
        |text| write_clipboard(app, text),
    )
}

fn inject_text_with(
    text: &str,
    inject: impl FnOnce(&str) -> Result<(), String>,
    copy: impl FnOnce(&str) -> Result<(), String>,
) -> Result<InjectionOutcome, String> {
    let Some(text) = normalized_injection_text(text) else {
        return Ok(InjectionOutcome::Ignored);
    };
    match inject(&text) {
        Ok(()) => Ok(InjectionOutcome::Injected),
        Err(injection_error) => match copy(&text) {
            Ok(()) => Ok(InjectionOutcome::CopiedOnly(injection_error)),
            Err(copy_error) => Err(format!(
                "{injection_error}; clipboard fallback failed: {copy_error}"
            )),
        },
    }
}

pub(crate) fn normalized_injection_text(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

pub(crate) fn should_synthesize_paste(
    foreground_process_id: Option<u32>,
    current_process_id: u32,
) -> bool {
    matches!(foreground_process_id, Some(process_id) if process_id != 0 && process_id != current_process_id)
}

#[cfg(target_os = "windows")]
pub fn capture_target() -> Option<InjectionTarget> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
    };

    let window = unsafe { GetForegroundWindow() };
    if window.0.is_null() {
        return None;
    }
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
    let mut gui = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    let focused_control = unsafe { GetGUIThreadInfo(thread_id, &mut gui) }
        .ok()
        .and_then(|_| (!gui.hwndFocus.0.is_null()).then_some(gui.hwndFocus.0 as isize));
    should_synthesize_paste(Some(process_id), std::process::id()).then_some(InjectionTarget {
        focused_control,
        window: window.0 as isize,
        process_id,
    })
}

#[cfg(not(target_os = "windows"))]
pub fn capture_target() -> Option<InjectionTarget> {
    None
}

#[cfg(target_os = "windows")]
fn inject_into_target(target: Option<InjectionTarget>, text: &str) -> Result<(), String> {
    let target = target.ok_or_else(|| "No external text target was focused.".to_string())?;
    if !target_is_foreground(target) {
        return Err("Focus changed before the transcript was ready.".into());
    }
    wait_for_modifiers_released()?;
    if !target_is_foreground(target) {
        return Err("Focus changed before the transcript was ready.".into());
    }
    send_unicode_text(target, text)
}

#[cfg(not(target_os = "windows"))]
fn inject_into_target(_target: Option<InjectionTarget>, _text: &str) -> Result<(), String> {
    Err("Focused-field injection is currently available only on Windows.".into())
}

#[cfg(target_os = "windows")]
fn target_is_foreground(target: InjectionTarget) -> bool {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
        },
    };

    let expected = HWND(target.window as *mut std::ffi::c_void);
    let foreground = unsafe { GetForegroundWindow() };
    if foreground != expected {
        return false;
    }
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(foreground, Some(&mut process_id)) };
    if process_id != target.process_id {
        return false;
    }
    let Some(expected_focus) = target.focused_control else {
        return true;
    };
    let mut gui = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetGUIThreadInfo(thread_id, &mut gui) }.is_ok()
        && gui.hwndFocus.0 as isize == expected_focus
}

#[cfg(target_os = "windows")]
fn wait_for_modifiers_released() -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };

    let deadline = std::time::Instant::now() + MODIFIER_RELEASE_TIMEOUT;
    loop {
        let any_pressed = [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
            .into_iter()
            .any(|key| unsafe { GetAsyncKeyState(key.0 as i32) } < 0);
        if !any_pressed {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err("Shortcut modifiers were still pressed.".into());
        }
        std::thread::sleep(MODIFIER_POLL_DELAY);
    }
}

#[cfg(target_os = "windows")]
fn send_unicode_text(target: InjectionTarget, text: &str) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    };

    let mut inputs = Vec::with_capacity(text.encode_utf16().count() * 2);
    for unit in text.encode_utf16() {
        inputs.push(unicode_input(unit, KEYEVENTF_UNICODE));
        inputs.push(unicode_input(unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    for chunk in inputs.chunks(UNICODE_INPUT_CHUNK) {
        if !target_is_foreground(target) {
            return Err("Focus changed during text insertion.".into());
        }
        let sent = unsafe { SendInput(chunk, std::mem::size_of::<INPUT>() as i32) };
        if sent != chunk.len() as u32 {
            return Err("Windows blocked text insertion.".into());
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn unicode_input(unit: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(target_os = "windows")]
fn write_clipboard(app: &tauri::AppHandle, text: &str) -> Result<(), String> {
    use std::ptr::copy_nonoverlapping;

    use windows::Win32::{
        Foundation::{GlobalFree, HANDLE},
        System::{
            DataExchange::{EmptyClipboard, SetClipboardData},
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
        },
    };

    let owner = app
        .get_webview_window(crate::MAIN_WINDOW_LABEL)
        .ok_or_else(|| "Yap main window is unavailable for clipboard fallback.".to_string())?
        .hwnd()
        .map_err(|error| format!("Failed to read Yap clipboard owner: {error}"))?;

    let mut wide = text.encode_utf16().collect::<Vec<_>>();
    wide.push(0);
    let byte_len = wide.len() * std::mem::size_of::<u16>();

    let handle = unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, byte_len)
            .map_err(|error| format!("Failed to allocate clipboard memory: {error}"))?;
        let buffer = GlobalLock(handle).cast::<u16>();
        if buffer.is_null() {
            let _ = GlobalFree(Some(handle));
            return Err("Failed to lock clipboard memory.".into());
        }
        copy_nonoverlapping(wide.as_ptr(), buffer, wide.len());
        let _ = GlobalUnlock(handle);
        handle
    };

    if let Err(error) = open_clipboard_with_retry(owner) {
        unsafe {
            let _ = GlobalFree(Some(handle));
        }
        return Err(error);
    }
    let _guard = ClipboardGuard;

    unsafe {
        if let Err(error) = EmptyClipboard() {
            let _ = GlobalFree(Some(handle));
            return Err(format!("Failed to clear clipboard: {error}"));
        }
        if let Err(error) = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(handle.0))) {
            let _ = GlobalFree(Some(handle));
            return Err(format!("Failed to write clipboard: {error}"));
        }
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn open_clipboard_with_retry(owner: windows::Win32::Foundation::HWND) -> Result<(), String> {
    let mut last_error = None;
    for attempt in 0..CLIPBOARD_OPEN_ATTEMPTS {
        match unsafe { windows::Win32::System::DataExchange::OpenClipboard(Some(owner)) } {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < CLIPBOARD_OPEN_ATTEMPTS {
            std::thread::sleep(CLIPBOARD_RETRY_DELAY);
        }
    }
    Err(format!(
        "Failed to open clipboard: {}",
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "unknown error".into())
    ))
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

#[cfg(not(target_os = "windows"))]
fn write_clipboard(_app: &tauri::AppHandle, _text: &str) -> Result<(), String> {
    Err("Focused-field injection is currently available only on Windows.".into())
}

#[cfg(test)]
mod tests {
    use super::{
        inject_text_with, normalized_injection_text, should_synthesize_paste, InjectionOutcome,
    };

    #[test]
    fn injection_ignores_empty_text_and_trims_transcripts() {
        assert_eq!(normalized_injection_text("  "), None);
        assert_eq!(
            normalized_injection_text("  hello from Yap\n"),
            Some("hello from Yap".into())
        );
    }

    #[test]
    fn paste_synthesis_requires_an_external_foreground_process() {
        assert!(!should_synthesize_paste(None, 42));
        assert!(!should_synthesize_paste(Some(0), 42));
        assert!(!should_synthesize_paste(Some(42), 42));
        assert!(should_synthesize_paste(Some(7), 42));
    }

    #[test]
    fn blocked_paste_retains_a_clipboard_fallback() {
        let outcome = inject_text_with("hello", |_| Err("blocked".into()), |_| Ok(())).unwrap();

        assert_eq!(outcome, InjectionOutcome::CopiedOnly("blocked".into()));
    }

    #[test]
    fn clipboard_failure_does_not_claim_a_fallback() {
        let result = inject_text_with(
            "hello",
            |_| Err("injection blocked".into()),
            |_| Err("clipboard busy".into()),
        );

        assert_eq!(
            result,
            Err("injection blocked; clipboard fallback failed: clipboard busy".into())
        );
    }
}
