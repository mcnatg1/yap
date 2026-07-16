use crate::stt::error::SttError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalComputeTarget {
    Auto,
    Cpu,
}

impl LocalComputeTarget {
    pub fn id(self) -> String {
        match self {
            Self::Auto => "auto".into(),
            Self::Cpu => "cpu".into(),
        }
    }
}

pub fn effective_compute_target() -> LocalComputeTarget {
    if let Some(target) =
        compute_target_from_env_value(std::env::var("YAP_USE_GPU").ok().as_deref())
    {
        return target;
    }
    saved_compute_target()
}

#[cfg(test)]
fn compute_target_from_env_for_test(value: Option<&str>) -> LocalComputeTarget {
    compute_target_from_env_value(value).unwrap_or(LocalComputeTarget::Auto)
}

pub fn polish_num_gpu_layers() -> u32 {
    match effective_compute_target() {
        LocalComputeTarget::Cpu => 0,
        LocalComputeTarget::Auto => 99,
    }
}

pub fn settings_dir_from<F>(env: F) -> std::path::PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    crate::paths::app_data_dir_from(env)
}

fn settings_dir() -> std::path::PathBuf {
    crate::paths::app_data_dir()
}

fn fallback_disabled_path() -> std::path::PathBuf {
    settings_dir().join("local-fallback.disabled")
}

fn compute_target_path() -> std::path::PathBuf {
    settings_dir().join("compute-target.txt")
}

pub fn saved_compute_target() -> LocalComputeTarget {
    crate::bounded_file::read_text(&compute_target_path(), 64)
        .ok()
        .and_then(|value| parse_compute_target(&value))
        .unwrap_or(LocalComputeTarget::Auto)
}

pub fn set_local_compute_target(target: &str) -> Result<(), SttError> {
    let target = parse_compute_target(target).ok_or(SttError::SidecarUnreachable)?;
    let path = compute_target_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| SttError::SidecarUnreachable)?;
    }
    std::fs::write(path, format!("{}\n", target.id())).map_err(|_| SttError::SidecarUnreachable)
}

fn compute_target_from_env_value(value: Option<&str>) -> Option<LocalComputeTarget> {
    let trimmed = value.map(str::trim).filter(|value| !value.is_empty())?;
    parse_compute_target(trimmed)
}

fn parse_compute_target(value: &str) -> Option<LocalComputeTarget> {
    let value = value.trim().to_lowercase();
    match value.as_str() {
        "auto" => Some(LocalComputeTarget::Auto),
        "cpu" | "0" | "false" | "off" | "no" => Some(LocalComputeTarget::Cpu),
        "1" | "true" | "on" | "yes" => Some(LocalComputeTarget::Auto),
        _ => None,
    }
}

pub fn local_fallback_enabled() -> bool {
    !fallback_disabled_path().exists()
}

pub fn set_local_fallback_enabled(enabled: bool) -> Result<(), SttError> {
    let path = fallback_disabled_path();
    if enabled {
        if path.exists() {
            std::fs::remove_file(path).map_err(|_| SttError::ModelMissing)?;
        }
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| SttError::ModelMissing)?;
    }
    std::fs::write(path, b"disabled\n").map_err(|_| SttError::ModelMissing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_use_auto_compute() {
        assert_eq!(
            compute_target_from_env_for_test(None),
            LocalComputeTarget::Auto
        );
        assert_eq!(
            compute_target_from_env_for_test(Some(" ")),
            LocalComputeTarget::Auto
        );
    }

    #[test]
    fn env_can_force_cpu() {
        assert_eq!(
            compute_target_from_env_for_test(Some("cpu")),
            LocalComputeTarget::Cpu
        );
    }

    #[test]
    fn rejects_specific_gpu_target_for_local_asr() {
        assert_eq!(parse_compute_target("gpu:1"), None);
        assert_eq!(parse_compute_target("auto"), Some(LocalComputeTarget::Auto));
        assert_eq!(parse_compute_target("cpu"), Some(LocalComputeTarget::Cpu));
        assert_eq!(parse_compute_target("gpu:nope"), None);
    }

    #[test]
    fn settings_dir_uses_app_data_override() {
        let local = std::env::temp_dir().join("local-data");
        let dir = settings_dir_from(|key| match key {
            "YAP_APP_DATA_DIR" => Some(local.display().to_string()),
            _ => None,
        });
        assert_eq!(dir, local);
    }
}
