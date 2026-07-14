use std::sync::atomic::{AtomicBool, Ordering};

use tauri::Manager;

#[cfg(target_os = "windows")]
use windows::core::Free;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn, HRGN};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

pub(crate) const WINDOW_LABEL: &str = crate::authorization::LIVE_OVERLAY_WINDOW_LABEL;

static IDLE_COLLAPSED_ACTIVE: AtomicBool = AtomicBool::new(true);

const COMPACT_HEIGHT: f64 = 40.0;
const COLLAPSED_WIDTH: f64 = 104.0;
const EXPANDED_WIDTH: f64 = 180.0;
const EXPANDED_HEIGHT: f64 = 88.0;
const ACTIVE_WIDTH: f64 = 112.0;
const SUCCESS_WIDTH: f64 = 168.0;
const FEEDBACK_WIDTH: f64 = 252.0;
const CORNER_RADIUS: f64 = 14.0;
const TOP_BEZEL_OFFSET: f64 = 0.0;

pub(crate) fn ensure_active(app: &tauri::AppHandle) -> Result<(), String> {
    ensure_surface(app, "recording")
}

pub(crate) fn ensure_idle(app: &tauri::AppHandle) -> Result<(), String> {
    ensure_surface(app, "collapsed")
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

pub(crate) fn frame(surface: &str) -> Result<(f64, f64), String> {
    match surface {
        "collapsed" => Ok((COLLAPSED_WIDTH, COMPACT_HEIGHT)),
        "expanded" => Ok((EXPANDED_WIDTH, EXPANDED_HEIGHT)),
        "recording" | "processing" | "initializing" => Ok((ACTIVE_WIDTH, COMPACT_HEIGHT)),
        "success" => Ok((SUCCESS_WIDTH, COMPACT_HEIGHT)),
        "feedback" => Ok((FEEDBACK_WIDTH, COMPACT_HEIGHT)),
        _ => Err("Unsupported live overlay surface.".into()),
    }
}

pub(crate) fn ensure_surface(app: &tauri::AppHandle, surface: &str) -> Result<(), String> {
    let (width, height) = frame(surface)?;
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        ensure_dimensions(&window, width, height)?;
        position(app, &window, width)?;
        apply_visible_region(&window, width, height)?;
        IDLE_COLLAPSED_ACTIVE.store(surface == "collapsed", Ordering::Release);
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
    apply_visible_region(&window, width, height)?;
    IDLE_COLLAPSED_ACTIVE.store(surface == "collapsed", Ordering::Release);
    position(app, &window, width)?;
    Ok(())
}

pub(crate) fn follow_cursor_if_idle(app: &tauri::AppHandle) {
    if !IDLE_COLLAPSED_ACTIVE.load(Ordering::Acquire) {
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
    let _ = position_on_monitor(&window, &target_monitor, COLLAPSED_WIDTH);
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
    let position = monitor.position();
    let size = monitor.size();
    position_for_monitor_metrics(
        f64::from(position.x),
        f64::from(position.y),
        f64::from(size.width),
        scale,
        width,
    )
}

fn position_for_monitor_metrics(
    physical_x: f64,
    physical_y: f64,
    physical_width: f64,
    scale: f64,
    window_width: f64,
) -> (f64, f64) {
    let logical_x = physical_x / scale;
    let logical_y = physical_y / scale;
    let logical_width = physical_width / scale;
    (
        logical_x + ((logical_width - window_width) / 2.0).max(0.0),
        logical_y + TOP_BEZEL_OFFSET,
    )
}

fn same_monitor(left: &tauri::Monitor, right: &tauri::Monitor) -> bool {
    left.position() == right.position() && left.size() == right.size()
}

#[cfg(target_os = "windows")]
fn apply_visible_region(
    window: &tauri::WebviewWindow,
    window_width: f64,
    window_height: f64,
) -> Result<(), String> {
    let hwnd = window
        .hwnd()
        .map_err(|err| format!("Failed to read live overlay window handle: {err}"))?;
    let scale = window
        .scale_factor()
        .map_err(|err| format!("Failed to read live overlay scale: {err}"))?;
    let mut region = create_visible_region(window_width, window_height, scale)?;
    if unsafe { SetWindowRgn(hwnd, Some(region), true) } == 0 {
        unsafe { region.free() };
        return Err("Failed to apply live overlay interaction region.".into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn create_visible_region(
    window_width: f64,
    window_height: f64,
    scale: f64,
) -> Result<HRGN, String> {
    let physical_width = (window_width * scale).round().max(1.0) as i32;
    let physical_height = (window_height * scale).round().max(1.0) as i32;
    let corner_diameter = (CORNER_RADIUS * 2.0 * scale).round().max(1.0) as i32;
    let region = unsafe {
        CreateRoundRectRgn(
            0,
            0,
            physical_width,
            physical_height,
            corner_diameter,
            corner_diameter,
        )
    };
    if region.is_invalid() {
        return Err("Failed to create live overlay interaction region.".into());
    }
    Ok(region)
}

#[cfg(not(target_os = "windows"))]
fn apply_visible_region(
    _window: &tauri::WebviewWindow,
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
    fn frame_matches_visible_surface_contract() {
        assert_eq!(frame("collapsed"), Ok((104.0, 40.0)));
        assert_eq!(frame("expanded"), Ok((180.0, 88.0)));
        for surface in ["recording", "processing", "initializing"] {
            assert_eq!(frame(surface), Ok((112.0, 40.0)));
        }
        assert_eq!(frame("success"), Ok((168.0, 40.0)));
        assert_eq!(frame("feedback"), Ok((252.0, 40.0)));
    }

    #[test]
    fn feedback_width_is_static() {
        assert_eq!(frame("feedback"), Ok((252.0, 40.0)));
    }

    #[test]
    fn unknown_surface_cannot_allocate_an_arbitrary_native_window() {
        assert_eq!(
            frame("sensor"),
            Err("Unsupported live overlay surface.".into())
        );
    }

    #[test]
    fn top_center_position_handles_negative_multi_monitor_origins_and_dpi() {
        let collapsed = position_for_monitor_metrics(-1920.0, 0.0, 1920.0, 1.5, 104.0);
        let expanded = position_for_monitor_metrics(-1920.0, 0.0, 1920.0, 1.5, 180.0);

        assert_eq!(collapsed, (-692.0, 0.0));
        assert_eq!(expanded, (-730.0, 0.0));
        assert_eq!(collapsed.1, expanded.1);
    }

    #[test]
    fn top_center_position_uses_target_monitor_logical_width_at_two_x_dpi() {
        assert_eq!(
            position_for_monitor_metrics(1920.0, 0.0, 3840.0, 2.0, 104.0),
            (1868.0, 0.0)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn visible_region_excludes_rounded_transparent_corners() {
        use windows::Win32::Graphics::Gdi::PtInRegion;

        let mut region = create_visible_region(104.0, 40.0, 1.0).unwrap();
        assert!(unsafe { PtInRegion(region, 52, 20) }.as_bool());
        assert!(!unsafe { PtInRegion(region, 0, 0) }.as_bool());
        assert!(!unsafe { PtInRegion(region, 103, 39) }.as_bool());
        unsafe { region.free() };
    }
}
