use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
}
