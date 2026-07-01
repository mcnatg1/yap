use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::stt::error::SttError;

pub const PORT_RANGE: RangeInclusive<u16> = 8765..=8775;
pub const READY_BUDGET: Duration = Duration::from_secs(10);
pub const IDLE_UNLOAD: Duration = Duration::from_secs(600);
const HOST: &str = "127.0.0.1";
const ENV_ALLOWLIST: [&str; 4] = ["PATH", "SYSTEMROOT", "TEMP", "TMP"];

pub fn first_free_port(range: RangeInclusive<u16>, mut is_free: impl FnMut(u16) -> bool) -> Option<u16> {
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
        .filter(|(key, _)| ENV_ALLOWLIST.iter().any(|allowed| key.eq_ignore_ascii_case(allowed)))
        .collect()
}

pub fn build_launch_args(gguf: &Path, port: u16) -> Vec<String> {
    vec![
        "--server".to_string(),
        "-m".to_string(),
        gguf.to_string_lossy().to_string(),
        "--host".to_string(),
        HOST.to_string(),
        "--port".to_string(),
        port.to_string(),
    ]
}

pub fn health_is_ready(json: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(value) => {
            value.get("status").and_then(serde_json::Value::as_str) == Some("ok")
                && value.get("backend").and_then(serde_json::Value::as_str) == Some("cohere")
        }
        Err(_) => false,
    }
}

pub fn should_unload(idle: Duration, threshold: Duration) -> bool {
    idle >= threshold
}

pub fn sidecar_binary_path(exe_dir: &Path) -> PathBuf {
    let name = if cfg!(windows) { "crispasr.exe" } else { "crispasr" };
    exe_dir.join(name)
}

pub fn resolve_binary<F>(env: F, exe_dir: &Path) -> Result<PathBuf, SttError>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dev) = env("YAP_CRISPASR_BIN") {
        let path = PathBuf::from(dev);
        return if path.exists() { Ok(path) } else { Err(SttError::SidecarUnreachable) };
    }
    let bundled = sidecar_binary_path(exe_dir);
    if bundled.exists() {
        Ok(bundled)
    } else {
        Err(SttError::SidecarUnreachable)
    }
}

pub struct CrispasrSidecar {
    child: Option<Child>,
    pub(crate) port: Option<u16>,
    last_used: Instant,
}

impl CrispasrSidecar {
    pub fn new() -> Self {
        Self {
            child: None,
            port: None,
            last_used: Instant::now(),
        }
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

    pub fn ensure_ready(&mut self) -> Result<String, SttError> {
        if self.is_running() {
            if let Some(url) = self.base_url() {
                self.mark_used();
                return Ok(url);
            }
        }
        self.shutdown();

        let binary = resolve_binary(|key| std::env::var(key).ok(), &current_exe_dir())?;
        let pin = crate::stt::pin::load_pin().map_err(|_| SttError::ModelCorrupt)?;
        if crate::stt::model::verify_sha256(&binary, &pin.binary_sha256).is_err() {
            crate::stt::log_stt("crispasr binary failed SHA-256 verification; refusing to spawn");
            return Err(SttError::SidecarUnreachable);
        }
        let model = crate::stt::model::ensure_model()?;
        let port = probe_port().ok_or(SttError::SidecarUnreachable)?;

        let child = spawn_child(&binary, &model, port)?;
        self.child = Some(child);
        self.port = Some(port);

        let url = self.base_url().ok_or(SttError::SidecarUnreachable)?;
        if wait_ready(&url) {
            self.mark_used();
            crate::stt::log_stt(&format!("crispasr sidecar ready on {url}"));
            Ok(url)
        } else {
            crate::stt::log_stt("crispasr sidecar failed the 10s ready-gate");
            self.shutdown();
            Err(SttError::SidecarUnreachable)
        }
    }

    pub fn restart(&mut self) -> Result<String, SttError> {
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

fn spawn_child(binary: &Path, model: &Path, port: u16) -> Result<Child, SttError> {
    let mut command = Command::new(binary);
    command.args(build_launch_args(model, port));
    command.env_clear();
    command.envs(sidecar_env(std::env::vars()));
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    crate::stt::hide_child_console(&mut command);
    command.spawn().map_err(|_| SttError::SidecarUnreachable)
}

fn wait_ready(base_url: &str) -> bool {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    let deadline = Instant::now() + READY_BUDGET;
    while Instant::now() < deadline {
        if let Ok(response) = client.get(format!("{base_url}/health")).send() {
            if response.status().is_success() {
                if let Ok(body) = response.text() {
                    if health_is_ready(&body) {
                        return true;
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
        assert!(!scrubbed.iter().any(|(k, _)| k.eq_ignore_ascii_case("HF_TOKEN")));
        assert!(!scrubbed.iter().any(|(k, _)| k.eq_ignore_ascii_case("GITHUB_TOKEN")));
    }

    #[test]
    fn launch_args_bind_loopback_only() {
        let args = build_launch_args(std::path::Path::new("C:/models/m.gguf"), 8765);
        assert_eq!(args[0], "--server");
        let host = args.iter().position(|a| a == "--host").unwrap();
        assert_eq!(args[host + 1], "127.0.0.1");
        let port = args.iter().position(|a| a == "--port").unwrap();
        assert_eq!(args[port + 1], "8765");
    }

    #[test]
    fn health_ready_requires_ok_and_cohere() {
        assert!(health_is_ready(r#"{"status":"ok","backend":"cohere"}"#));
        assert!(!health_is_ready(r#"{"status":"ok","backend":"whisper"}"#));
        assert!(!health_is_ready(r#"{"status":"loading","backend":"cohere"}"#));
        assert!(!health_is_ready("not json"));
    }

    #[test]
    fn should_unload_after_threshold() {
        assert!(should_unload(std::time::Duration::from_secs(601), IDLE_UNLOAD));
        assert!(!should_unload(std::time::Duration::from_secs(10), IDLE_UNLOAD));
    }

    #[test]
    fn resolve_binary_missing_dev_override_is_unreachable() {
        let err = resolve_binary(|_| Some("C:/definitely/not/here.exe".into()), std::path::Path::new("C:/app"));
        assert_eq!(err.unwrap_err(), SttError::SidecarUnreachable);
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
