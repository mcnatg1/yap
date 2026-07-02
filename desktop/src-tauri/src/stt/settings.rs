use crate::stt::error::SttError;
use crate::stt::gpu::GpuPreference;

pub fn effective_gpu_preference() -> GpuPreference {
    preference_from_env_value(std::env::var("YAP_USE_GPU").ok().as_deref())
}

fn preference_from_env_value(value: Option<&str>) -> GpuPreference {
    let Some(trimmed) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return GpuPreference::Auto;
    };
    crate::stt::gpu::gpu_preference_from(Some(trimmed))
}

pub fn polish_num_gpu_layers() -> u32 {
    match effective_gpu_preference() {
        GpuPreference::Cpu => 0,
        GpuPreference::Auto | GpuPreference::On => 99,
    }
}

pub fn settings_dir_from<F>(env: F) -> std::path::PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(local) = env("LOCALAPPDATA") {
        return std::path::PathBuf::from(local).join("Yap");
    }
    std::path::PathBuf::from(".")
}

fn settings_dir() -> std::path::PathBuf {
    settings_dir_from(|key| std::env::var(key).ok())
}

fn fallback_disabled_path() -> std::path::PathBuf {
    settings_dir().join("local-fallback.disabled")
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
    fn default_settings_use_auto_gpu() {
        assert_eq!(preference_from_env_value(None), GpuPreference::Auto);
        assert_eq!(preference_from_env_value(Some(" ")), GpuPreference::Auto);
    }

    #[test]
    fn env_can_force_cpu() {
        assert_eq!(preference_from_env_value(Some("cpu")), GpuPreference::Cpu);
    }

    #[test]
    fn settings_dir_uses_localappdata() {
        let dir = settings_dir_from(|key| match key {
            "LOCALAPPDATA" => Some("C:/Users/me/AppData/Local".into()),
            _ => None,
        });
        assert_eq!(
            dir,
            std::path::PathBuf::from("C:/Users/me/AppData/Local").join("Yap")
        );
    }
}
