use crate::live::actions;

use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

const SHOW_APP: &str = "show_app";
const START_DICTATING: &str = "start_dictating";
const STOP_RECORDING: &str = "stop_recording";
const QUIT: &str = "quit";

fn dispatch_menu_action(app: &tauri::AppHandle, action: &str) -> bool {
    match action {
        SHOW_APP => actions::show_main_window(app),
        START_DICTATING => actions::start_live_from_app(app),
        STOP_RECORDING => actions::stop_live_from_app(app),
        QUIT => actions::quit_from_app(app),
        _ => return false,
    }
    true
}

#[cfg(feature = "wdio")]
#[tauri::command]
pub(crate) fn wdio_dispatch_tray_action(
    app: tauri::AppHandle,
    action: String,
) -> Result<(), String> {
    if !matches!(action.as_str(), SHOW_APP | QUIT) {
        return Err("WDIO may dispatch only the restore and quit tray actions.".into());
    }
    dispatch_menu_action(&app, &action)
        .then_some(())
        .ok_or_else(|| "Unknown tray action.".into())
}

pub(crate) fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    let menu = MenuBuilder::new(app)
        .text(SHOW_APP, "Show Yap")
        .text(START_DICTATING, "Start Dictating")
        .text(STOP_RECORDING, "Stop Recording")
        .separator()
        .text(QUIT, "Quit")
        .build()?;

    let mut tray = TrayIconBuilder::with_id("yap")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Yap")
        .on_menu_event(|app, event| {
            dispatch_menu_action(app, event.id().as_ref());
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                actions::show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }

    tray.build(app)?;
    Ok(())
}
