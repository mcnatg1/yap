use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Emitter};

use crate::stt::binary::{self, BinaryInstallStatus};
use crate::stt::error::SttError;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineBootstrapProgressEvent {
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineBootstrapErrorEvent {
    pub message: String,
}

fn bootstrap_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn emit_progress(app: &AppHandle, message: &str) {
    let _ = app.emit(
        "engine-bootstrap-progress",
        EngineBootstrapProgressEvent {
            message: message.to_string(),
        },
    );
}

/// Fallback when the sidecar or model was not installed (dev without fetch, interrupted install, corrupt cache).
/// Shipped installers download a SHA-verified crispasr.exe and GGUF model during NSIS post-install.
pub fn spawn(app: AppHandle) {
    match binary::binary_install_status(&exe_dir()) {
        Ok(BinaryInstallStatus::Installed) => return,
        Ok(BinaryInstallStatus::Unsupported) => return,
        Ok(BinaryInstallStatus::Downloadable | BinaryInstallStatus::Invalid) | Err(_) => {}
    }

    std::thread::spawn(move || {
        let _guard = match bootstrap_lock().lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        run(&app);
    });
}

pub fn run_blocking(app: &AppHandle) -> Result<(), String> {
    let _guard = bootstrap_lock()
        .lock()
        .map_err(|_| "Install already in progress.".to_string())?;
    run(app);
    Ok(())
}

fn run(app: &AppHandle) {
    crate::stt::log_stt("bootstrap fallback start");

    match binary::binary_install_status(&exe_dir()) {
        Ok(BinaryInstallStatus::Installed) => {
            crate::stt::log_stt("bootstrap fallback: engine already installed");
            let _ = app.emit("engine-bootstrap-complete", ());
            return;
        }
        Ok(BinaryInstallStatus::Downloadable | BinaryInstallStatus::Invalid) => {
            emit_progress(app, "Downloading transcription engine (fallback)…");
            match binary::ensure_binary() {
                Ok(path) => {
                    crate::stt::log_stt(&format!("bootstrap fallback engine installed at {}", path.display()));
                }
                Err(error) => {
                    let message = bootstrap_error_message(&error);
                    crate::stt::log_stt(&format!("bootstrap fallback engine failed: {message}"));
                    let _ = app.emit(
                        "engine-bootstrap-error",
                        EngineBootstrapErrorEvent {
                            message: message.clone(),
                        },
                    );
                    return;
                }
            }
        }
        Ok(BinaryInstallStatus::Unsupported) => {
            let _ = app.emit(
                "engine-bootstrap-error",
                EngineBootstrapErrorEvent {
                    message: "Set YAP_CRISPASR_BIN or run npm run fetch:crispasr.".into(),
                },
            );
            return;
        }
        Err(error) => {
            let message = bootstrap_error_message(&error);
            let _ = app.emit(
                "engine-bootstrap-error",
                EngineBootstrapErrorEvent {
                    message: message.clone(),
                },
            );
            return;
        }
    }

    crate::stt::log_stt("bootstrap fallback complete");
    let _ = app.emit("engine-bootstrap-complete", ());
}

fn bootstrap_error_message(error: &SttError) -> String {
    match error {
        SttError::ModelCorrupt => "Download failed verification (SHA-256 mismatch).".into(),
        SttError::ModelMissing => "Download failed — check your network connection.".into(),
        SttError::SidecarUnreachable => "Transcription engine could not be installed.".into(),
        other => other.user_message().to_string(),
    }
}
