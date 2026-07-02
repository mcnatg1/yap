use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::stt::binary;
use crate::stt::error::SttError;
use crate::stt::progress::ProgressReporter;
pub const PORT_RANGE: RangeInclusive<u16> = 8765..=8775;
pub const READY_BUDGET: Duration = Duration::from_secs(300);
pub const IDLE_UNLOAD: Duration = Duration::from_secs(600);
const HOST: &str = "127.0.0.1";
const LOCAL_FALLBACK_BACKEND: &str = "moonshine-streaming";
const LOCAL_FALLBACK_THREADS: &str = "8";
const LOCAL_FALLBACK_PROCESSORS: &str = "2";
const ENV_ALLOWLIST: [&str; 4] = ["PATH", "SYSTEMROOT", "TEMP", "TMP"];

pub fn first_free_port(
    range: RangeInclusive<u16>,
    mut is_free: impl FnMut(u16) -> bool,
) -> Option<u16> {
    range.into_iter().find(|port| is_free(*port))
}

pub fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind((HOST, port)).is_ok()
}

pub fn probe_port() -> Option<u16> {
    first_free_port(PORT_RANGE, port_is_free)
}

pub fn sidecar_env<I>(source: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (String, String)>,
{
    source
        .into_iter()
        .filter(|(key, _)| {
            ENV_ALLOWLIST
                .iter()
                .any(|allowed| key.eq_ignore_ascii_case(allowed))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarEndpoint {
    pub url: String,
    pub api_key: String,
}

pub fn build_launch_args(
    gguf: &Path,
    punc_model: &Path,
    port: u16,
    gpu_layers: u32,
    api_key: &str,
) -> Vec<String> {
    let cache_dir = gguf.parent().unwrap_or_else(|| Path::new("."));
    let mut args = vec![
        "--server".to_string(),
        "--backend".to_string(),
        LOCAL_FALLBACK_BACKEND.to_string(),
        "-m".to_string(),
        gguf.to_string_lossy().to_string(),
        "--host".to_string(),
        HOST.to_string(),
        "--port".to_string(),
        port.to_string(),
        "--api-keys".to_string(),
        api_key.to_string(),
        "--cache-dir".to_string(),
        cache_dir.to_string_lossy().to_string(),
        "--punc-model".to_string(),
        punc_model.to_string_lossy().to_string(),
        "-t".to_string(),
        LOCAL_FALLBACK_THREADS.to_string(),
        "-p".to_string(),
        LOCAL_FALLBACK_PROCESSORS.to_string(),
    ];
    if gpu_layers > 0 {
        args.push("--gpu-backend".to_string());
        args.push("auto".to_string());
    } else {
        args.push("-ng".to_string());
    }
    args
}

pub fn redact_launch_args(args: &[String]) -> Vec<String> {
    let mut redacted = args.to_vec();
    if let Some(index) = redacted.iter().position(|arg| arg == "--api-keys") {
        if let Some(value) = redacted.get_mut(index + 1) {
            *value = "<redacted>".to_string();
        }
    }
    redacted
}

fn random_api_key() -> Result<String, SttError> {
    let mut bytes = [0u8; 32];
    fill_random(&mut bytes)?;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    Ok(out)
}

#[cfg(windows)]
fn fill_random(bytes: &mut [u8]) -> Result<(), SttError> {
    #[link(name = "advapi32")]
    extern "system" {
        fn SystemFunction036(buffer: *mut std::ffi::c_void, length: u32) -> u8;
    }
    let ok = unsafe { SystemFunction036(bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    if ok == 0 {
        Err(SttError::SidecarUnreachable)
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn fill_random(bytes: &mut [u8]) -> Result<(), SttError> {
    use std::io::Read as _;

    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(bytes))
        .map_err(|_| SttError::SidecarUnreachable)
}

#[cfg(not(any(windows, unix)))]
fn fill_random(_bytes: &mut [u8]) -> Result<(), SttError> {
    Err(SttError::SidecarUnreachable)
}

pub fn health_is_ready(json: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(value) => {
            let backend = value.get("backend").and_then(serde_json::Value::as_str);
            value.get("status").and_then(serde_json::Value::as_str) == Some("ok")
                && matches!(backend, Some("moonshine-streaming" | "moonshine_streaming"))
        }
        Err(_) => false,
    }
}

pub fn should_unload(idle: Duration, threshold: Duration) -> bool {
    idle >= threshold
}

pub fn sidecar_binary_path(exe_dir: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "crispasr.exe"
    } else {
        "crispasr"
    };
    exe_dir.join(name)
}

pub fn resolve_binary<F>(env: F, exe_dir: &Path) -> Result<PathBuf, SttError>
where
    F: Fn(&str) -> Option<String>,
{
    let _ = env;
    binary::resolve_for_spawn(exe_dir)
}
pub struct CrispasrSidecar {
    child: Option<Child>,
    pub(crate) port: Option<u16>,
    api_key: Option<String>,
    last_used: Instant,
}

impl CrispasrSidecar {
    pub fn new() -> Self {
        Self {
            child: None,
            port: None,
            api_key: None,
            last_used: Instant::now(),
        }
    }

    fn endpoint(&self) -> Option<SidecarEndpoint> {
        Some(SidecarEndpoint {
            url: self.base_url()?,
            api_key: self.api_key.clone()?,
        })
    }

    pub fn base_url(&self) -> Option<String> {
        self.port.map(|port| format!("http://{HOST}:{port}"))
    }

    pub fn is_running(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(None) => true,
                Ok(Some(_)) => false,
                Err(_) => false,
            },
            None => false,
        }
    }

    pub fn mark_used(&mut self) {
        self.last_used = Instant::now();
    }

    pub fn ensure_ready(&mut self) -> Result<SidecarEndpoint, SttError> {
        self.ensure_ready_with_progress(None)
    }

    pub fn ensure_ready_with_progress(
        &mut self,
        reporter: Option<&ProgressReporter>,
    ) -> Result<SidecarEndpoint, SttError> {
        let started = Instant::now();
        if self.is_running() {
            if let Some(endpoint) = self.endpoint() {
                self.mark_used();
                crate::stt::log_stt_timed(
                    "ensure_ready",
                    started.elapsed(),
                    "sidecar already running",
                );
                return Ok(endpoint);
            }
        }
        self.shutdown();

        if let Some(report) = reporter {
            report.emit("loading_model", Some(3), "Preparing transcription engine…");
        }
        crate::stt::log_stt("ensure_ready: resolving binary");

        let binary = match binary::binary_install_status(&current_exe_dir())? {
            binary::BinaryInstallStatus::Installed => {
                binary::resolve_for_spawn(&current_exe_dir())?
            }
            binary::BinaryInstallStatus::Downloadable | binary::BinaryInstallStatus::Invalid => {
                if let Some(report) = reporter {
                    report.emit(
                        "loading_model",
                        Some(5),
                        "Downloading transcription engine…",
                    );
                }
                crate::stt::log_stt("ensure_ready: downloading binary (fallback)");
                binary::ensure_binary()?
            }
            binary::BinaryInstallStatus::Unsupported => {
                crate::stt::log_stt("crispasr auto-install unsupported on this platform");
                return Err(SttError::SidecarUnreachable);
            }
        };
        crate::stt::log_stt_timed(
            "ensure_ready",
            started.elapsed(),
            &format!("binary ready at {}", binary.display()),
        );

        if let Some(report) = reporter {
            report.emit("loading_model", Some(8), "Loading transcription model…");
        }

        let model_started = Instant::now();
        let pin = crate::stt::pin::load_pin().map_err(|_| SttError::ModelCorrupt)?;
        let model = if crate::stt::model::is_installed(&pin) {
            let path = crate::stt::model::models_dir().join(&pin.gguf_file);
            let size_gb = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0) as f64 / 1e9;
            crate::stt::log_stt_timed(
                "ensure_ready",
                model_started.elapsed(),
                &format!("using cached model {:.2} GB at {}", size_gb, path.display()),
            );
            path
        } else {
            if let Some(report) = reporter {
                report.emit(
                    "loading_model",
                    Some(6),
                    "Downloading transcription model (fallback)…",
                );
            }
            crate::stt::log_stt("ensure_ready: downloading model (fallback)");
            crate::stt::model::ensure_model_at(
                &crate::stt::model::models_dir(),
                &pin,
                crate::stt::model::download_file,
            )?
        };
        let punc_model = crate::stt::model::models_dir().join(&pin.punc_file);
        let port = probe_port().ok_or(SttError::SidecarUnreachable)?;
        let api_key = random_api_key()?;

        let gpu = crate::stt::gpu::GpuStatus::resolve();
        crate::stt::log_stt(&format!(
            "crispasr spawn binary={} model={} port={} gpu_available={} layers={} adapter={}",
            binary.display(),
            model.display(),
            port,
            gpu.available,
            gpu.layers,
            gpu.adapter_name.as_deref().unwrap_or("none")
        ));
        let spawn_started = Instant::now();
        let child = spawn_child(&binary, &model, &punc_model, port, gpu.layers, &api_key)?;
        self.child = Some(child);
        self.port = Some(port);
        self.api_key = Some(api_key);
        crate::stt::log_stt_timed(
            "ensure_ready",
            spawn_started.elapsed(),
            "sidecar process spawned",
        );

        let url = self.base_url().ok_or(SttError::SidecarUnreachable)?;
        if wait_ready_with_progress(&url, reporter, started) {
            self.mark_used();
            crate::stt::log_stt_timed(
                "ensure_ready",
                started.elapsed(),
                &format!("sidecar ready on {url}"),
            );
            self.endpoint().ok_or(SttError::SidecarUnreachable)
        } else {
            crate::stt::log_stt_timed(
                "ensure_ready",
                started.elapsed(),
                &format!(
                    "sidecar failed ready-gate after {}s",
                    READY_BUDGET.as_secs()
                ),
            );
            self.shutdown();
            Err(SttError::SidecarUnreachable)
        }
    }

    pub fn restart(&mut self) -> Result<SidecarEndpoint, SttError> {
        crate::stt::log_stt("crispasr sidecar restart-once");
        self.shutdown();
        self.ensure_ready()
    }

    pub fn unload_if_idle(&mut self) {
        if self.is_running() && should_unload(self.last_used.elapsed(), IDLE_UNLOAD) {
            crate::stt::log_stt("crispasr sidecar idle-unload after 10min");
            self.shutdown();
        }
    }

    pub fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.port = None;
        self.api_key = None;
    }
}

impl Default for CrispasrSidecar {
    fn default() -> Self {
        Self::new()
    }
}

fn current_exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn spawn_child(
    binary: &Path,
    model: &Path,
    punc_model: &Path,
    port: u16,
    gpu_layers: u32,
    api_key: &str,
) -> Result<Child, SttError> {
    let stderr_path = crate::stt::sidecar_stderr_log_path();
    if let Some(parent) = stderr_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let stderr_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_path)
        .map_err(|_| SttError::SidecarUnreachable)?;
    let args = build_launch_args(model, punc_model, port, gpu_layers, api_key);
    crate::stt::log_stt(&format!(
        "spawning sidecar stderr_log={} args={:?}",
        stderr_path.display(),
        redact_launch_args(&args)
    ));

    let mut command = Command::new(binary);
    command.args(args);
    command.env_clear();
    command.envs(sidecar_env(std::env::vars()));
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::from(stderr_file));
    crate::stt::hide_child_console(&mut command);
    command.spawn().map_err(|err| {
        crate::stt::log_stt(&format!("sidecar spawn failed: {err}"));
        SttError::SidecarUnreachable
    })
}

fn wait_ready_with_progress(
    base_url: &str,
    reporter: Option<&ProgressReporter>,
    started: Instant,
) -> bool {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            crate::stt::log_stt(&format!("health client build failed: {err}"));
            return false;
        }
    };
    let deadline = Instant::now() + READY_BUDGET;
    let mut last_logged_secs = 0u64;
    while Instant::now() < deadline {
        let elapsed = started.elapsed().as_secs();
        if elapsed >= last_logged_secs + 5 {
            last_logged_secs = elapsed;
            crate::stt::log_stt(&format!(
                "health wait {elapsed}s / {}s (loading model into memory…)",
                READY_BUDGET.as_secs()
            ));
            if let Some(report) = reporter {
                let pct = (8 + elapsed.min(READY_BUDGET.as_secs()).saturating_mul(82)
                    / READY_BUDGET.as_secs()) as u8;
                report.emit(
                    "loading_model",
                    Some(pct.min(90)),
                    &format!("Loading model into memory ({elapsed}s)…"),
                );
            }
        }

        if let Ok(response) = client.get(format!("{base_url}/health")).send() {
            if response.status().is_success() {
                if let Ok(body) = response.text() {
                    if health_is_ready(&body) {
                        return true;
                    }
                    if elapsed >= last_logged_secs.saturating_sub(4) {
                        crate::stt::log_stt(&format!("health not ready yet: {body}"));
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_free_port_returns_first_matching() {
        assert_eq!(first_free_port(8765..=8775, |p| p == 8770), Some(8770));
        assert_eq!(first_free_port(8765..=8767, |_| false), None);
    }

    #[test]
    fn sidecar_env_drops_secrets_keeps_allowlist() {
        let source = vec![
            ("PATH".to_string(), "C:/bin".to_string()),
            ("HF_TOKEN".to_string(), "secret".to_string()),
            ("GITHUB_TOKEN".to_string(), "secret".to_string()),
            ("SystemRoot".to_string(), "C:/Windows".to_string()),
        ];
        let scrubbed = sidecar_env(source);
        assert!(scrubbed.iter().any(|(k, _)| k == "PATH"));
        assert!(scrubbed.iter().any(|(k, _)| k == "SystemRoot"));
        assert!(!scrubbed
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("HF_TOKEN")));
        assert!(!scrubbed
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("GITHUB_TOKEN")));
    }

    #[test]
    fn launch_args_bind_loopback_only() {
        let args = build_launch_args(
            std::path::Path::new("C:/models/m.gguf"),
            std::path::Path::new("C:/models/punc.gguf"),
            8765,
            0,
            "secret",
        );
        assert_eq!(args[0], "--server");
        assert_eq!(args[1], "--backend");
        assert_eq!(args[2], "moonshine-streaming");
        let host = args.iter().position(|a| a == "--host").unwrap();
        assert_eq!(args[host + 1], "127.0.0.1");
        let port = args.iter().position(|a| a == "--port").unwrap();
        assert_eq!(args[port + 1], "8765");
        let api_key = args.iter().position(|a| a == "--api-keys").unwrap();
        assert_eq!(args[api_key + 1], "secret");
        let cache_dir = args.iter().position(|a| a == "--cache-dir").unwrap();
        assert_eq!(args[cache_dir + 1], "C:/models");
        let punc = args.iter().position(|a| a == "--punc-model").unwrap();
        assert_eq!(args[punc + 1], "C:/models/punc.gguf");
        assert!(!args.contains(&"--no-punctuation".to_string()));
        let threads = args.iter().position(|a| a == "-t").unwrap();
        assert_eq!(args[threads + 1], "8");
        let processors = args.iter().position(|a| a == "-p").unwrap();
        assert_eq!(args[processors + 1], "2");
        assert!(args.contains(&"-ng".to_string()));
        assert!(!args.contains(&"--gpu-backend".to_string()));
    }

    #[test]
    fn launch_args_use_gpu_backend_auto_when_gpu_available() {
        let args = build_launch_args(
            std::path::Path::new("C:/models/m.gguf"),
            std::path::Path::new("C:/models/punc.gguf"),
            8765,
            99,
            "secret",
        );
        let gpu = args.iter().position(|a| a == "--gpu-backend").unwrap();
        assert_eq!(args[gpu + 1], "auto");
        assert!(!args.contains(&"-ng".to_string()));
    }

    #[test]
    fn redacts_api_key_from_logged_args() {
        let args = vec![
            "--api-keys".to_string(),
            "secret".to_string(),
            "--port".to_string(),
            "8765".to_string(),
        ];
        let redacted = redact_launch_args(&args);
        assert_eq!(redacted[1], "<redacted>");
        assert_eq!(args[1], "secret");
    }

    #[test]
    fn health_ready_requires_ok_and_moonshine_streaming() {
        assert!(health_is_ready(
            r#"{"status":"ok","backend":"moonshine-streaming"}"#
        ));
        assert!(health_is_ready(
            r#"{"status":"ok","backend":"moonshine_streaming"}"#
        ));
        assert!(!health_is_ready(r#"{"status":"ok","backend":"whisper"}"#));
        assert!(!health_is_ready(
            r#"{"status":"loading","backend":"moonshine-streaming"}"#
        ));
        assert!(!health_is_ready("not json"));
    }

    #[test]
    fn should_unload_after_threshold() {
        assert!(should_unload(
            std::time::Duration::from_secs(601),
            IDLE_UNLOAD
        ));
        assert!(!should_unload(
            std::time::Duration::from_secs(10),
            IDLE_UNLOAD
        ));
    }

    #[test]
    fn resolve_binary_rejects_invalid_dev_override() {
        let dir = std::env::temp_dir().join(format!("yap-bin-dev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stub = dir.join("bad.exe");
        std::fs::write(&stub, vec![0u8; 32]).unwrap();
        std::env::set_var("YAP_CRISPASR_BIN", &stub);
        let err = resolve_binary(|_| None, std::path::Path::new("C:/app")).unwrap_err();
        std::env::remove_var("YAP_CRISPASR_BIN");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(err, SttError::SidecarUnreachable);
    }

    #[test]
    fn new_sidecar_is_not_running_and_has_no_url() {
        let mut sidecar = CrispasrSidecar::new();
        assert!(!sidecar.is_running());
        assert!(sidecar.base_url().is_none());
        sidecar.shutdown(); // no panic when there is no child
    }

    #[test]
    fn base_url_uses_loopback_and_port() {
        let mut sidecar = CrispasrSidecar::new();
        sidecar.port = Some(8770);
        assert_eq!(sidecar.base_url().unwrap(), "http://127.0.0.1:8770");
    }
}
