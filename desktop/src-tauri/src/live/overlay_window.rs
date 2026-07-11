use tauri::Manager;

#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

pub(crate) const WINDOW_LABEL: &str = crate::authorization::LIVE_OVERLAY_WINDOW_LABEL;

const COMPACT_HEIGHT: f64 = 40.0;
const DEFAULT_WIDTH: f64 = 104.0;
const HOVER_SENSOR_WIDTH: f64 = 260.0;
const HOVER_SENSOR_HEIGHT: f64 = 8.0;
const MIN_ERROR_WIDTH: f64 = 180.0;
const MAX_ERROR_WIDTH: f64 = 420.0;
const TOP_BEZEL_OFFSET: f64 = 0.0;

pub(crate) fn ensure_active(app: &tauri::AppHandle) -> Result<(), String> {
    ensure_size(app, HOVER_SENSOR_WIDTH, COMPACT_HEIGHT)
}

pub(crate) fn ensure_idle(app: &tauri::AppHandle) -> Result<(), String> {
    ensure_size(app, HOVER_SENSOR_WIDTH, HOVER_SENSOR_HEIGHT)
}

pub(crate) fn recover(app: &tauri::AppHandle) {
    let view = app.state::<crate::live::LiveSessionState>().snapshot();
    if view.visibility != crate::live::state::LiveOverlayVisibility::Enabled {
        return;
    }
    if app
        .get_webview_window(WINDOW_LABEL)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
    {
        return;
    }
    let result = if crate::live::state::is_live_session_started(view.status)
        || view.status == crate::live::state::LiveSessionStatus::Blocked
    {
        ensure_active(app)
    } else {
        ensure_idle(app)
    };
    if let Err(error) = result {
        crate::stt::log_yap(&format!("live overlay recovery failed: {error}"));
    }
}

pub(crate) fn frame(surface: &str, error_message: Option<&str>) -> (f64, f64) {
    let width = match surface {
        "sensor" | "peek" | "recording" | "processing" | "initializing" | "success" => {
            HOVER_SENSOR_WIDTH
        }
        "feedback" => error_message.map_or(DEFAULT_WIDTH, |message| {
            (message.len() as f64 * 6.8 + 74.0).clamp(MIN_ERROR_WIDTH, MAX_ERROR_WIDTH)
        }),
        _ => DEFAULT_WIDTH,
    };
    let height = if surface == "sensor" {
        HOVER_SENSOR_HEIGHT
    } else {
        COMPACT_HEIGHT
    };
    (width, height)
}

pub(crate) fn ensure_size(app: &tauri::AppHandle, width: f64, height: f64) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        window
            .set_size(tauri::LogicalSize::new(width, height))
            .map_err(|err| format!("Failed to size live overlay: {err}"))?;
        window
            .set_shadow(false)
            .map_err(|err| format!("Failed to hide live overlay shadow: {err}"))?;
        window
            .set_skip_taskbar(true)
            .map_err(|err| format!("Failed to hide live overlay from taskbar: {err}"))?;
        window
            .set_closable(false)
            .map_err(|err| format!("Failed to lock live overlay close control: {err}"))?;
        window
            .set_focusable(false)
            .map_err(|err| format!("Failed to keep live overlay unfocusable: {err}"))?;
        make_system_window(&window)?;
        position(app, &window, width)?;
        window
            .show()
            .map_err(|err| format!("Failed to show live overlay: {err}"))?;
        return Ok(());
    }

    let (x, y) = position_for_width(app, width);
    let window = tauri::WebviewWindowBuilder::new(
        app,
        WINDOW_LABEL,
        tauri::WebviewUrl::App("index.html?window=live-overlay".into()),
    )
    .title("Yap Live")
    .inner_size(width, height)
    .position(x, y)
    .decorations(false)
    .resizable(false)
    .closable(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .focusable(false)
    .build()
    .map_err(|err| format!("Failed to create live overlay: {err}"))?;
    window
        .set_focusable(false)
        .map_err(|err| format!("Failed to keep live overlay unfocusable: {err}"))?;
    make_system_window(&window)?;
    position(app, &window, width)?;
    Ok(())
}

fn position(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    width: f64,
) -> Result<(), String> {
    let (x, y) = position_for_width(app, width);
    window
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|err| format!("Failed to position live overlay: {err}"))
}

fn position_for_width(app: &tauri::AppHandle, width: f64) -> (f64, f64) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let position = monitor.position().to_logical::<f64>(scale);
        let size = monitor.size().to_logical::<f64>(scale);
        return (
            position.x + ((size.width - width) / 2.0).max(0.0),
            position.y + TOP_BEZEL_OFFSET,
        );
    }
    (8.0, TOP_BEZEL_OFFSET)
}

#[cfg(target_os = "windows")]
fn make_system_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    let hwnd = window
        .hwnd()
        .map_err(|err| format!("Failed to read live overlay window handle: {err}"))?;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let next_style = (style | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) & !WS_EX_APPWINDOW.0;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next_style as isize);
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .map_err(|err| format!("Failed to refresh live overlay window style: {err}"))?;
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn make_system_window(_window: &tauri::WebviewWindow) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_matches_frontend_surface_contract() {
        for surface in ["peek", "recording", "processing", "initializing", "success"] {
            assert_eq!(frame(surface, None), (HOVER_SENSOR_WIDTH, COMPACT_HEIGHT));
        }
        assert_eq!(
            frame("sensor", None),
            (HOVER_SENSOR_WIDTH, HOVER_SENSOR_HEIGHT)
        );
    }

    #[test]
    fn feedback_frame_clamps_error_width() {
        assert_eq!(frame("feedback", None), (DEFAULT_WIDTH, COMPACT_HEIGHT));
        assert_eq!(frame("feedback", Some("short")).0, MIN_ERROR_WIDTH);
        assert_eq!(frame("feedback", Some(&"x".repeat(200))).0, MAX_ERROR_WIDTH);
    }
}
