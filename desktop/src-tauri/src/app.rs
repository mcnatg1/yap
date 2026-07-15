use tauri::Manager;

use crate::{authorization, commands, jobs, live, paths, runtime, stt, tray};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitRequestDisposition {
    PreventAndFinalize,
    Allow,
}

fn exit_request_disposition(exit_authorized: bool) -> ExitRequestDisposition {
    if exit_authorized {
        ExitRequestDisposition::Allow
    } else {
        ExitRequestDisposition::PreventAndFinalize
    }
}

fn is_allowed_app_navigation(url: &tauri::Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match (url.scheme(), url.host_str(), url.port()) {
        ("tauri", Some("localhost"), None) => true,
        ("http" | "https", Some("tauri.localhost"), None) => true,
        ("http", Some("localhost"), Some(1420)) if cfg!(debug_assertions) => true,
        ("about", None, None) => url.path() == "blank" && url.query().is_none(),
        _ => false,
    }
}

fn navigation_guard<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("navigation-guard")
        .on_navigation(|_, url| is_allowed_app_navigation(url))
        .build()
}

fn write_startup_migration_diagnostic(
    directory: &std::path::Path,
    detail: &str,
) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for attempt in 0..100_u8 {
        let path = directory.join(format!(
            "Yap-startup-migration-error-{}-{nonce}-{attempt}.log",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(mut file) => {
                writeln!(file, "Yap stopped before startup to protect existing data.")?;
                writeln!(file, "Migration error: {detail}")?;
                file.flush()?;
                file.sync_all()?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique Yap startup diagnostic",
    ))
}

fn stop_for_migration_error(error: &std::io::Error) -> ! {
    let diagnostic =
        write_startup_migration_diagnostic(&std::env::temp_dir(), &error.to_string()).ok();
    let diagnostic_detail = diagnostic
        .as_ref()
        .map(|path| format!("\n\nDiagnostic: {}", path.display()))
        .unwrap_or_else(|| "\n\nA diagnostic file could not be created.".to_string());
    let message = format!(
        "Yap did not start because its existing data could not be migrated safely. No source data was intentionally deleted. Close any other Yap process and inspect the conflict before trying again.\n\nReason: {error}{diagnostic_detail}"
    );

    #[cfg(windows)]
    show_startup_error_dialog(&message);
    #[cfg(not(windows))]
    eprintln!("{message}");

    std::process::exit(1)
}

#[cfg(windows)]
fn show_startup_error_dialog(message: &str) {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
    use windows::{
        core::PCWSTR,
        Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK},
    };

    let message = OsStr::new(message)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let title = OsStr::new("Yap startup stopped")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub(crate) fn run() {
    if let Err(error) = paths::migrate_legacy_app_data() {
        stop_for_migration_error(&error);
    }
    std::panic::set_hook(Box::new(|panic| {
        stt::log_yap(&format!("panic: {panic}"));
    }));
    stt::log_yap("app start");

    let stt_state = stt::dispatch::SttState::new();
    let live_settings = live::settings::load();
    let live_shortcuts = live::shortcut_runtime::prepare(&live_settings);
    let runtime_state = runtime::RuntimeOrchestratorState::new();
    let live_runtime = live::runtime::LiveRuntime::new();
    let live_state = live::LiveSessionState::new(live_settings);
    let fallback_model_install_state = stt::fallback_model::FallbackModelInstallState::new();
    let live_runtime_for_monitor = live_runtime.clone();
    let live_runtime_for_exit = live_runtime.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        live_runtime_for_monitor.unload_if_idle(std::time::Duration::from_secs(600));
    });

    let builder = tauri::Builder::default()
        .plugin(navigation_guard())
        .plugin(tauri_plugin_dialog::init());

    #[cfg(feature = "wdio")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());

    let builder = builder
        .manage(stt_state)
        .manage(live_state)
        .manage(live_runtime)
        .manage(live::actions::QuitCoordinator::new())
        .manage(fallback_model_install_state)
        .manage(runtime_state)
        .setup(move |app| {
            live::shortcut_runtime::install(app, live_shortcuts)?;
            tray::install(app.handle())?;
            jobs::start_remote_job_drain(app.handle());
            {
                let app = app.handle().clone();
                std::thread::spawn(move || {
                    let mut recovery_ticks = 0_u8;
                    loop {
                        std::thread::sleep(std::time::Duration::from_millis(125));
                        live::overlay_window::follow_cursor_if_idle(&app);
                        recovery_ticks = recovery_ticks.saturating_add(1);
                        if recovery_ticks >= 16 {
                            live::overlay_window::recover(&app);
                            recovery_ticks = 0;
                        }
                    }
                });
            }
            let startup_live = app.state::<live::LiveSessionState>().snapshot();
            if startup_live.visibility == live::state::LiveOverlayVisibility::Enabled {
                let result = if startup_live.status == live::state::LiveSessionStatus::Idle {
                    live::overlay_window::ensure_idle(app.handle())
                } else {
                    live::overlay_window::ensure_active(app.handle())
                };
                if let Err(error) = result {
                    stt::log_yap(&format!("live overlay startup failed: {error}"));
                }
            }
            let live_runtime = app.state::<live::runtime::LiveRuntime>();
            live::actions::warm_on_intent(app.handle(), &live_runtime);
            Ok(())
        });

    commands::register(builder)
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(move |app_handle, event| match event {
            tauri::RunEvent::WebviewEvent {
                label,
                event: tauri::WebviewEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }),
                ..
            } if label == authorization::MAIN_WINDOW_LABEL => {
                let app = app_handle.clone();
                std::thread::spawn(move || {
                    if let Err(error) = jobs::commands::import_native_paths(&app, paths) {
                        jobs::commands::emit_native_import_error(&app, &error);
                    }
                });
            }
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } if label == authorization::MAIN_WINDOW_LABEL => {
                api.prevent_close();
                if let Some(window) =
                    app_handle.get_webview_window(authorization::MAIN_WINDOW_LABEL)
                {
                    let _ = window.hide();
                }
            }
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } if label == authorization::LIVE_OVERLAY_WINDOW_LABEL => {
                api.prevent_close();
            }
            tauri::RunEvent::ExitRequested { api, .. } => {
                let quit = app_handle.state::<live::actions::QuitCoordinator>();
                if exit_request_disposition(quit.exit_authorized())
                    == ExitRequestDisposition::PreventAndFinalize
                {
                    api.prevent_exit();
                    live::actions::quit_from_app(app_handle);
                }
            }
            tauri::RunEvent::Exit => {
                let quit = app_handle.state::<live::actions::QuitCoordinator>();
                if !quit.exit_authorized() {
                    stt::log_yap("process exit reached degraded live shutdown fallback");
                    live_runtime_for_exit.shutdown();
                }
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::{
        exit_request_disposition, is_allowed_app_navigation, write_startup_migration_diagnostic,
        ExitRequestDisposition,
    };

    #[test]
    fn exit_request_requires_semantic_quit_authorization() {
        assert_eq!(
            exit_request_disposition(false),
            ExitRequestDisposition::PreventAndFinalize
        );
        assert_eq!(
            exit_request_disposition(true),
            ExitRequestDisposition::Allow
        );
    }

    #[test]
    fn navigation_guard_allows_only_application_origins() {
        for allowed in [
            "tauri://localhost/index.html",
            "http://tauri.localhost/index.html",
            "https://tauri.localhost/live-overlay.html",
            "about:blank",
        ] {
            assert!(is_allowed_app_navigation(
                &tauri::Url::parse(allowed).unwrap()
            ));
        }
        for blocked in [
            "https://example.com/",
            "https://tauri.localhost.example.com/",
            "https://user@tauri.localhost/",
            "data:text/html,blocked",
            "file:///C:/private.txt",
        ] {
            assert!(!is_allowed_app_navigation(
                &tauri::Url::parse(blocked).unwrap()
            ));
        }
    }

    #[test]
    fn startup_migration_diagnostic_is_created_outside_app_data() {
        let root = std::env::temp_dir().join(format!(
            "yap-startup-diagnostic-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let path = write_startup_migration_diagnostic(&root, "migration conflict").unwrap();

        assert_eq!(path.parent(), Some(root.as_path()));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("migration conflict"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
