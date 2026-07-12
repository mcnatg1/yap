use std::sync::atomic::{AtomicBool, Ordering};

use tauri::Manager;

#[cfg(target_os = "windows")]
use windows::core::Free;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::{CreateRectRgn, SetWindowRgn};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

pub(crate) const WINDOW_LABEL: &str = crate::authorization::LIVE_OVERLAY_WINDOW_LABEL;

static IDLE_SENSOR_ACTIVE: AtomicBool = AtomicBool::new(true);

const COMPACT_HEIGHT: f64 = 40.0;
const HOVER_SENSOR_WIDTH: f64 = 260.0;
const HOVER_SENSOR_HEIGHT: f64 = 8.0;
const ACTIVE_INTERACTION_WIDTH: f64 = 252.0;
const TOP_BEZEL_OFFSET: f64 = 0.0;

pub(crate) fn ensure_active(app: &tauri::AppHandle) -> Result<(), String> {
    ensure_surface(app, "recording", None)
}

pub(crate) fn ensure_idle(app: &tauri::AppHandle) -> Result<(), String> {
    ensure_surface(app, "sensor", None)
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
    let _ = (surface, error_message);
    (HOVER_SENSOR_WIDTH, COMPACT_HEIGHT)
}

pub(crate) fn ensure_surface(
    app: &tauri::AppHandle,
    surface: &str,
    error_message: Option<&str>,
) -> Result<(), String> {
    let (width, height) = frame(surface, error_message);
    let interaction_width = interaction_width(surface);
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        ensure_dimensions(&window, width, height)?;
        if matches!(surface, "sensor" | "peek") {
            position(app, &window, width)?;
        }
        apply_interaction_region(&window, surface, interaction_width, width, height)?;
        IDLE_SENSOR_ACTIVE.store(surface == "sensor", Ordering::Release);
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
    apply_interaction_region(&window, surface, interaction_width, width, height)?;
    IDLE_SENSOR_ACTIVE.store(surface == "sensor", Ordering::Release);
    position(app, &window, width)?;
    Ok(())
}

pub(crate) fn follow_cursor_if_idle(app: &tauri::AppHandle) {
    if !IDLE_SENSOR_ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let Some(target_monitor) = monitor_for_cursor(app) else {
        return;
    };
    if window
        .current_monitor()
        .ok()
        .flatten()
        .is_some_and(|current| same_monitor(&current, &target_monitor))
    {
        return;
    }
    let _ = position_on_monitor(&window, &target_monitor, HOVER_SENSOR_WIDTH);
}

fn ensure_dimensions(window: &tauri::WebviewWindow, width: f64, height: f64) -> Result<(), String> {
    let scale = window
        .scale_factor()
        .map_err(|err| format!("Failed to read live overlay scale: {err}"))?;
    let current = window
        .inner_size()
        .map_err(|err| format!("Failed to read live overlay size: {err}"))?
        .to_logical::<f64>(scale);
    if (current.width - width).abs() <= 0.5 && (current.height - height).abs() <= 0.5 {
        return Ok(());
    }
    window
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|err| format!("Failed to size live overlay: {err}"))
}

fn interaction_width(surface: &str) -> f64 {
    if surface == "sensor" {
        HOVER_SENSOR_WIDTH
    } else {
        ACTIVE_INTERACTION_WIDTH
    }
}

fn position(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    width: f64,
) -> Result<(), String> {
    let Some(monitor) = monitor_for_cursor(app) else {
        return window
            .set_position(tauri::LogicalPosition::new(8.0, TOP_BEZEL_OFFSET))
            .map_err(|err| format!("Failed to position live overlay: {err}"));
    };
    position_on_monitor(window, &monitor, width)
}

fn position_on_monitor(
    window: &tauri::WebviewWindow,
    monitor: &tauri::Monitor,
    width: f64,
) -> Result<(), String> {
    let (x, y) = position_for_monitor(monitor, width);
    window
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|err| format!("Failed to position live overlay: {err}"))
}

fn position_for_width(app: &tauri::AppHandle, width: f64) -> (f64, f64) {
    monitor_for_cursor(app)
        .map(|monitor| position_for_monitor(&monitor, width))
        .unwrap_or((8.0, TOP_BEZEL_OFFSET))
}

fn monitor_for_cursor(app: &tauri::AppHandle) -> Option<tauri::Monitor> {
    app.cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
}

fn position_for_monitor(monitor: &tauri::Monitor, width: f64) -> (f64, f64) {
    let scale = monitor.scale_factor();
    let position = monitor.position().to_logical::<f64>(scale);
    let size = monitor.size().to_logical::<f64>(scale);
    (
        position.x + ((size.width - width) / 2.0).max(0.0),
        position.y + TOP_BEZEL_OFFSET,
    )
}

fn same_monitor(left: &tauri::Monitor, right: &tauri::Monitor) -> bool {
    left.position() == right.position() && left.size() == right.size()
}

#[cfg(target_os = "windows")]
fn apply_interaction_region(
    window: &tauri::WebviewWindow,
    surface: &str,
    island_width: f64,
    window_width: f64,
    window_height: f64,
) -> Result<(), String> {
    let hwnd = window
        .hwnd()
        .map_err(|err| format!("Failed to read live overlay window handle: {err}"))?;
    let scale = window
        .scale_factor()
        .map_err(|err| format!("Failed to read live overlay scale: {err}"))?;
    let physical_width = (window_width * scale).round().max(1.0) as i32;
    let physical_height = (window_height * scale).round().max(1.0) as i32;
    let mut region = if surface == "sensor" {
        let sensor_height = (HOVER_SENSOR_HEIGHT * scale).ceil().max(1.0) as i32;
        unsafe { CreateRectRgn(0, 0, physical_width, sensor_height) }
    } else {
        let island_width = (island_width * scale)
            .round()
            .clamp(1.0, f64::from(physical_width)) as i32;
        let left = (physical_width - island_width) / 2;
        unsafe { CreateRectRgn(left, 0, left + island_width, physical_height) }
    };
    if region.is_invalid() {
        return Err("Failed to create live overlay interaction region.".into());
    }
    if unsafe { SetWindowRgn(hwnd, Some(region), true) } == 0 {
        unsafe { region.free() };
        return Err("Failed to apply live overlay interaction region.".into());
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn apply_interaction_region(
    _window: &tauri::WebviewWindow,
    _surface: &str,
    _island_width: f64,
    _window_width: f64,
    _window_height: f64,
) -> Result<(), String> {
    Ok(())
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
        assert_eq!(frame("sensor", None), (HOVER_SENSOR_WIDTH, COMPACT_HEIGHT));
    }

    #[test]
    fn feedback_uses_the_fixed_frame_and_active_interaction_region() {
        assert_eq!(
            frame("feedback", Some(&"x".repeat(200))),
            (HOVER_SENSOR_WIDTH, COMPACT_HEIGHT)
        );
        assert_eq!(interaction_width("feedback"), ACTIVE_INTERACTION_WIDTH);
    }

    #[test]
    fn active_interaction_region_does_not_change_between_surfaces() {
        assert_eq!(interaction_width("sensor"), HOVER_SENSOR_WIDTH);
        for surface in [
            "peek",
            "recording",
            "processing",
            "initializing",
            "success",
            "feedback",
        ] {
            assert_eq!(interaction_width(surface), ACTIVE_INTERACTION_WIDTH);
        }
    }
}
