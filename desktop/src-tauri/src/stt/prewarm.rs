use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};

use crate::stt::binary::{self, BinaryInstallStatus};
use crate::stt::bootstrap::EngineBootstrapProgressEvent;
use crate::stt::dispatch::SttState;

pub fn spawn_background(app: AppHandle) {
    std::thread::spawn(move || prewarm(&app));
}

fn prewarm(app: &AppHandle) {
    let exe_dir = current_exe_dir();
    let Ok(binary_status) = binary::binary_install_status(&exe_dir) else {
        crate::stt::log_yap("prewarm skipped: could not read binary status");
        return;
    };
    if binary_status != BinaryInstallStatus::Installed {
        crate::stt::log_yap(&format!("prewarm skipped: binary_status={binary_status:?}"));
        return;
    }
    let Ok(pin) = crate::stt::pin::load_pin() else {
        crate::stt::log_yap("prewarm skipped: pin file missing");
        return;
    };
    if !crate::stt::model::is_installed(&pin) {
        crate::stt::log_yap("prewarm skipped: model not installed");
        return;
    }

    crate::stt::log_yap("prewarm: loading transcription engine in background");
    crate::stt::log_stt("prewarm: background sidecar startup");
    let _ = app.emit(
        "engine-bootstrap-progress",
        EngineBootstrapProgressEvent {
            message: "Loading transcription model into memory…".into(),
        },
    );

    let state = app.state::<SttState>();
    let sidecar = Arc::clone(&state.sidecar);
    let result = sidecar.lock().map_err(|_| ()).and_then(|mut guard| guard.ensure_ready().map_err(|_| ()));
    match result {
        Ok(url) => {
            crate::stt::log_yap(&format!("prewarm complete: {url}"));
            let _ = app.emit("engine-bootstrap-complete", ());
        }
        Err(()) => {
            crate::stt::log_yap("prewarm failed: sidecar did not become ready");
            let _ = app.emit(
                "engine-bootstrap-error",
                crate::stt::bootstrap::EngineBootstrapErrorEvent {
                    message: "Engine failed to load. Check logs in %LOCALAPPDATA%\\Yap\\logs\\.".into(),
                },
            );
        }
    }
}

fn current_exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}
