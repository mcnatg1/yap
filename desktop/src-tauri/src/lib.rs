use std::sync::Arc;

pub mod stt;

#[tauri::command]
fn polish_num_gpu() -> u32 {
    stt::settings::polish_num_gpu_layers()
}

#[tauri::command]
fn setup_status(_state: tauri::State<'_, stt::dispatch::SttState>) -> SetupStatus {
    let root = project_root();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let binary_status = stt::binary::binary_install_status(&exe_dir)
        .unwrap_or(stt::binary::BinaryInstallStatus::Unsupported);
    let pin = stt::pin::load_pin().ok();
    let model_installed = pin
        .as_ref()
        .map(|pin| stt::model::is_installed(pin))
        .unwrap_or(false);
    let engine_ready = pin.is_some()
        && model_installed
        && matches!(binary_status, stt::binary::BinaryInstallStatus::Installed);
    log_line(&format!(
        "setup_status engine_ready={engine_ready} binary={binary_status:?}"
    ));

    SetupStatus {
        model: pin
            .as_ref()
            .map(|pin| pin.gguf_file.clone())
            .unwrap_or_else(|| "moonshine-streaming-tiny-q4_k.gguf".into()),
        root: root.display().to_string(),
        engine_ready,
        engine_binary_status: stt::dispatch::engine_binary_status_label(binary_status).to_string(),
        model_installed,
        engine_status: compose_engine_status(binary_status, model_installed),
    }
}

fn compose_engine_status(
    binary_status: stt::binary::BinaryInstallStatus,
    model_installed: bool,
) -> String {
    match binary_status {
        stt::binary::BinaryInstallStatus::Installed if model_installed => {
            "Transcription engine ready".to_string()
        }
        stt::binary::BinaryInstallStatus::Installed => "Local fallback model missing".into(),
        stt::binary::BinaryInstallStatus::Downloadable => "Local fallback not installed".into(),
        stt::binary::BinaryInstallStatus::Invalid => "Local fallback failed verification".into(),
        stt::binary::BinaryInstallStatus::Unsupported => {
            "Transcription engine requires manual install".into()
        }
    }
}

#[tauri::command]
fn transcribe_files(
    state: tauri::State<'_, stt::dispatch::SttState>,
    paths: Vec<String>,
) -> Result<Vec<stt::dispatch::TranscriptResult>, stt::dispatch::SttCommandError> {
    log_line(&format!("transcribe_files count={}", paths.len()));
    stt::dispatch::transcribe_paths(&state, paths, "en")
}

#[tauri::command]
fn start_transcribe(
    app: tauri::AppHandle,
    state: tauri::State<'_, stt::dispatch::SttState>,
    paths: Vec<String>,
) -> Result<(), stt::dispatch::SttCommandError> {
    if paths.is_empty() {
        return Ok(());
    }
    if state.is_transcribing() {
        return Err(stt::dispatch::SttCommandError {
            code: stt::error::SttError::Busy.code().to_string(),
            message: stt::error::SttError::Busy.user_message().to_string(),
        });
    }

    log_line(&format!("start_transcribe count={}", paths.len()));
    std::thread::spawn(move || {
        use tauri::{Emitter, Manager};

        let progress = stt::progress::ProgressSink::new({
            let app = app.clone();
            move |event| {
                let _ = app.emit("transcribe-progress", event);
            }
        });
        let on_file_complete = {
            let app = app.clone();
            Arc::new(move |event: stt::dispatch::TranscribeFileCompleteEvent| {
                let _ = app.emit("transcribe-file-complete", event);
            })
        };
        let on_batch_complete = {
            let app = app.clone();
            Arc::new(move |event: stt::dispatch::TranscribeBatchCompleteEvent| {
                let _ = app.emit("transcribe-complete", event);
            })
        };

        let state = app.state::<stt::dispatch::SttState>();
        let result = stt::dispatch::transcribe_paths_with_callbacks(
            &state,
            paths,
            "en",
            Some(progress),
            Some(on_file_complete),
            Some(on_batch_complete),
        );
        if let Err(error) = result {
            let _ = app.emit("transcribe-error", error);
        }
    });

    Ok(())
}

#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);
    let is_txt = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"));

    if !is_txt {
        return Err("Only transcript text files can be read.".into());
    }

    std::fs::read_to_string(&path).map_err(|err| format!("Failed to read transcript: {err}"))
}

#[tauri::command]
fn write_polished_text(path: String, text: String) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);
    let is_txt = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"));

    if !is_txt {
        return Err("Only transcript text files can be polished.".into());
    }

    let output = polished_path(&path)?;
    std::fs::write(&output, text)
        .map_err(|err| format!("Failed to save polished transcript: {err}"))?;
    Ok(output.display().to_string())
}

#[tauri::command]
fn open_devtools(window: tauri::WebviewWindow) {
    window.open_devtools();
}

fn polished_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| "Transcript path has no file name.".to_string())?;

    Ok(path.with_file_name(format!("{stem}.polished.txt")))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupStatus {
    model: String,
    root: String,
    engine_ready: bool,
    engine_binary_status: String,
    model_installed: bool,
    engine_status: String,
}

fn project_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn log_line(message: &str) {
    stt::log_yap(message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_status_serializes_for_frontend() {
        let value = serde_json::to_value(SetupStatus {
            model: "model".into(),
            root: "root".into(),
            engine_ready: true,
            engine_binary_status: "Installed".into(),
            model_installed: true,
            engine_status: "Transcription engine ready".into(),
        })
        .unwrap();

        assert_eq!(value["engineReady"], true);
        assert_eq!(value["engineBinaryStatus"], "Installed");
        assert_eq!(value["modelInstalled"], true);
        assert_eq!(value["engineStatus"], "Transcription engine ready");
        assert!(value.get("python_ready").is_none());
    }

    #[test]
    fn read_text_file_rejects_non_transcripts() {
        assert!(read_text_file("recording.mp3".into()).is_err());
    }

    #[test]
    fn polished_path_writes_sibling_file() {
        let path = polished_path(std::path::Path::new("C:/recordings/take.txt")).unwrap();
        assert_eq!(path.file_name().unwrap(), "take.polished.txt");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    std::panic::set_hook(Box::new(|panic| {
        log_line(&format!("panic: {panic}"));
    }));
    log_line("app start");

    let stt_state = stt::dispatch::SttState::new();
    let sidecar_for_monitor = std::sync::Arc::clone(&stt_state.sidecar);
    let sidecar_for_exit = std::sync::Arc::clone(&stt_state.sidecar);
    let transcribing_for_monitor = stt_state.transcribing_flag();

    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        if transcribing_for_monitor.load(std::sync::atomic::Ordering::Relaxed) {
            continue;
        }
        if let Ok(mut sidecar) = sidecar_for_monitor.lock() {
            sidecar.unload_if_idle();
        }
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(stt_state)
        .setup(|_app| Ok(()))
        .invoke_handler(tauri::generate_handler![
            setup_status,
            polish_num_gpu,
            transcribe_files,
            start_transcribe,
            read_text_file,
            write_polished_text,
            open_devtools
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(move |_app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                if let Ok(mut sidecar) = sidecar_for_exit.lock() {
                    sidecar.shutdown();
                }
            }
        });
}
