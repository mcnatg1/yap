mod stt;

#[tauri::command]
fn setup_status(state: tauri::State<'_, stt::dispatch::SttState>) -> SetupStatus {
    let root = project_root();
    let python = python_path(&root);
    let script = root.join("transcribe.py");
    let engine_ready = stt::dispatch::engine_ready();
    let using_fallback = state.fell_back();
    let readiness = stt::dispatch::engine_readiness(engine_ready, using_fallback);
    log_line(&format!(
        "setup_status engine_ready={engine_ready} using_fallback={using_fallback}"
    ));

    SetupStatus {
        model: std::env::var("YAP_MODEL_ID")
            .unwrap_or_else(|_| "ZoOtMcNoOt/yap-cohere-transcribe-03-2026".into()),
        root: root.display().to_string(),
        python_ready: python.exists(),
        script_ready: script.exists(),
        python: python.display().to_string(),
        engine_ready,
        using_fallback,
        engine_status: stt::dispatch::engine_status_label(readiness).to_string(),
    }
}

#[tauri::command]
fn transcribe_files(
    state: tauri::State<'_, stt::dispatch::SttState>,
    paths: Vec<String>,
) -> Result<Vec<stt::dispatch::TranscriptResult>, stt::dispatch::SttCommandError> {
    log_line(&format!("transcribe_files count={}", paths.len()));
    stt::dispatch::transcribe_paths(&state, project_root(), paths, "en")
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
    std::fs::write(&output, text).map_err(|err| format!("Failed to save polished transcript: {err}"))?;
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
    python_ready: bool,
    script_ready: bool,
    python: String,
    engine_ready: bool,
    using_fallback: bool,
    engine_status: String,
}

fn project_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn python_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".venv").join("Scripts").join("python.exe")
}

fn log_line(message: &str) {
    use std::io::Write;

    let log_path = project_root().join("local-transcribe.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(
            file,
            "{} {}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or_default(),
            message
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_status_serializes_for_frontend() {
        let value = serde_json::to_value(SetupStatus {
            model: "model".into(),
            root: "root".into(),
            python_ready: true,
            script_ready: true,
            python: "python.exe".into(),
            engine_ready: true,
            using_fallback: false,
            engine_status: "Transcription engine ready".into(),
        })
        .unwrap();

        assert_eq!(value["pythonReady"], true);
        assert_eq!(value["scriptReady"], true);
        assert_eq!(value["engineReady"], true);
        assert_eq!(value["usingFallback"], false);
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

    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        if let Ok(mut sidecar) = sidecar_for_monitor.lock() {
            sidecar.unload_if_idle();
        }
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(stt_state)
        .invoke_handler(tauri::generate_handler![
            setup_status,
            transcribe_files,
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
