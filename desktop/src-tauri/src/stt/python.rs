use std::path::{Path, PathBuf};
use std::process::Command;

use crate::stt::backend::SttBackend;
use crate::stt::error::SttError;

pub struct PythonBackend {
    root: PathBuf,
}

impl PythonBackend {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn python(&self) -> PathBuf {
        self.root.join(".venv").join("Scripts").join("python.exe")
    }

    fn script(&self) -> PathBuf {
        self.root.join("transcribe.py")
    }

    pub fn build_command(&self, files: &[PathBuf], language: &str, out_dir: &Path) -> Command {
        let mut command = Command::new(self.python());
        command.current_dir(&self.root).arg(self.script());
        for file in files {
            command.arg(file);
        }
        command.arg("--language").arg(language).arg("--out-dir").arg(out_dir);
        crate::stt::hide_child_console(&mut command);
        command
    }
}

impl SttBackend for PythonBackend {
    fn transcribe(&self, audio: &Path, language: &str) -> Result<String, SttError> {
        let files = [audio.to_path_buf()];
        self.transcribe_batch(&files, language)
            .into_iter()
            .next()
            .unwrap_or(Err(SttError::AudioDecode))
    }

    fn transcribe_batch(&self, files: &[PathBuf], language: &str) -> Vec<Result<String, SttError>> {
        if files.is_empty() {
            return Vec::new();
        }
        if !self.python().exists() || !self.script().exists() {
            return errors_for(files.len(), SttError::SidecarUnreachable);
        }
        let out_dir = match temp_out_dir() {
            Ok(dir) => dir,
            Err(_) => return errors_for(files.len(), SttError::SidecarCrash),
        };
        let output = self.build_command(files, language, &out_dir).output();
        let result = match output {
            Ok(output) if output.status.success() => {
                let produced: Vec<String> = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(String::from)
                    .collect();
                files
                    .iter()
                    .enumerate()
                    .map(|(index, _)| match produced.get(index) {
                        Some(path) => std::fs::read_to_string(path)
                            .map(|text| text.trim().to_string())
                            .map_err(|_| SttError::AudioDecode),
                        None => Err(SttError::AudioDecode),
                    })
                    .collect()
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                crate::stt::log_stt(&format!("python backend failed: {}", stderr.trim()));
                errors_for(files.len(), classify_python_failure(&stderr))
            }
            Err(err) => {
                crate::stt::log_stt(&format!("python backend spawn error: {err}"));
                errors_for(files.len(), SttError::SidecarUnreachable)
            }
        };
        let _ = std::fs::remove_dir_all(&out_dir);
        result
    }
}

fn errors_for(count: usize, error: SttError) -> Vec<Result<String, SttError>> {
    (0..count).map(|_| Err(error)).collect()
}

fn temp_out_dir() -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("yap-stt-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn classify_python_failure(stderr: &str) -> SttError {
    let lower = stderr.to_lowercase();
    if lower.contains("out of memory") || lower.contains("memoryerror") {
        SttError::Oom
    } else if lower.contains("gated") || lower.contains("requires approval") || lower.contains("access denied") {
        SttError::ModelMissing
    } else if lower.contains("soundfile") || lower.contains("load_audio") || lower.contains("ffmpeg") {
        SttError::AudioDecode
    } else {
        SttError::SidecarCrash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command_targets_venv_python_and_script() {
        let backend = PythonBackend::new(PathBuf::from("C:/proj"));
        let files = [PathBuf::from("C:/clips/a.wav")];
        let command = backend.build_command(&files, "en", &PathBuf::from("C:/tmp/out"));
        let program = command.get_program().to_string_lossy().to_string();
        assert!(program.ends_with("python.exe"));
        let args: Vec<String> = command.get_args().map(|a| a.to_string_lossy().to_string()).collect();
        assert!(args.iter().any(|a| a.ends_with("transcribe.py")));
        assert!(args.contains(&"--language".to_string()));
        assert!(args.contains(&"en".to_string()));
        assert!(args.contains(&"--out-dir".to_string()));
    }

    #[test]
    fn classify_python_failure_maps_known_causes() {
        assert_eq!(classify_python_failure("CUDA out of memory"), SttError::Oom);
        assert_eq!(classify_python_failure("Repo is gated, requires approval"), SttError::ModelMissing);
        assert_eq!(classify_python_failure("soundfile failed to open the file"), SttError::AudioDecode);
        assert_eq!(classify_python_failure("some unexpected traceback"), SttError::SidecarCrash);
    }
}
