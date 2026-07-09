use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

const SHOW_APP: &str = "show_app";
const START_DICTATING: &str = "start_dictating";
const STOP_RECORDING: &str = "stop_recording";
const QUIT: &str = "quit";

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
        .on_menu_event(|app, event| match event.id().as_ref() {
            SHOW_APP => crate::show_main_window(app),
            START_DICTATING => crate::start_live_from_app(app),
            STOP_RECORDING => crate::stop_live_from_app(app),
            QUIT => app.exit(0),
            _ => {}
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
                crate::show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }

    tray.build(app)?;
    Ok(())
}
