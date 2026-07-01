mod stt;

#[tauri::command]
fn setup_status() -> SetupStatus {
    let root = project_root();
    let python = python_path(&root);
    let script = root.join("transcribe.py");
    log_line(&format!(
        "setup_status root={} python_ready={} script_ready={}",
        root.display(),
        python.exists(),
        script.exists()
    ));

    SetupStatus {
        model: std::env::var("YAP_MODEL_ID")
            .unwrap_or_else(|_| "ZoOtMcNoOt/yap-cohere-transcribe-03-2026".into()),
        root: root.display().to_string(),
        python_ready: python.exists(),
        script_ready: script.exists(),
        python: python.display().to_string(),
    }
}

#[tauri::command]
fn transcribe_files(paths: Vec<String>) -> Result<Vec<TranscriptResult>, String> {
    log_line(&format!("transcribe_files count={}", paths.len()));

    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let root = project_root();
    let python = python_path(&root);
    let script = root.join("transcribe.py");

    if !python.exists() {
        log_line(&format!("missing python {}", python.display()));
        return Err(format!("Missing Python venv: {}", python.display()));
    }

    if !script.exists() {
        log_line(&format!("missing runner {}", script.display()));
        return Err(format!("Missing runner: {}", script.display()));
    }

    let mut command = std::process::Command::new(&python);
    command.current_dir(&root).arg(&script).args(&paths);
    hide_child_console(&mut command);

    let output = command.output().map_err(|err| {
        log_line(&format!("failed to start transcription: {err}"));
        format!("Failed to start transcription: {err}")
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let message = command_failure_message(&stderr, &stdout);
        log_line(&format!(
            "transcription failed status={:?} stderr={} stdout={}",
            output.status.code(),
            stderr,
            stdout
        ));
        return Err(message);
    }

    let outputs: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect();

    log_line(&format!("transcription complete outputs={}", outputs.len()));

    Ok(paths
        .into_iter()
        .enumerate()
        .map(|(index, input)| TranscriptResult {
            input: input.clone(),
            output: outputs.get(index).cloned().unwrap_or_else(|| {
                std::path::Path::new(&input)
                    .with_extension("txt")
                    .display()
                    .to_string()
            }),
        })
        .collect())
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
}

#[derive(serde::Serialize)]
struct TranscriptResult {
    input: String,
    output: String,
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

fn command_failure_message(stderr: &str, stdout: &str) -> String {
    let message = if stderr.is_empty() { stdout } else { stderr };
    if let Some(index) = message.rfind("Traceback") {
        return message[index..].trim().to_string();
    }

    const MAX_CHARS: usize = 4000;
    if message.chars().count() <= MAX_CHARS {
        return message.to_string();
    }

    let tail: String = message
        .chars()
        .rev()
        .take(MAX_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("...{tail}")
}

#[cfg(windows)]
fn hide_child_console(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
}

#[cfg(not(windows))]
fn hide_child_console(_: &mut std::process::Command) {}

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
        })
        .unwrap();

        assert_eq!(value["pythonReady"], true);
        assert_eq!(value["scriptReady"], true);
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

    #[test]
    fn command_failure_message_uses_traceback_tail() {
        let message = command_failure_message("Loading weights: 100%\nTraceback sad", "");
        assert_eq!(message, "Traceback sad");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    std::panic::set_hook(Box::new(|panic| {
        log_line(&format!("panic: {panic}"));
    }));
    log_line("app start");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            setup_status,
            transcribe_files,
            read_text_file,
            write_polished_text,
            open_devtools
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
