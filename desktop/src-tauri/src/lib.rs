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
        model: "CohereLabs/cohere-transcribe-03-2026".into(),
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
        log_line(&format!(
            "transcription failed status={:?} stderr={} stdout={}",
            output.status.code(),
            stderr,
            stdout
        ));
        return Err(if stderr.is_empty() { stdout } else { stderr });
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
            read_text_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
