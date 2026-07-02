use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::stt::gpu::GpuPreference;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GpuSetting {
    #[default]
    Cpu,
    Auto,
}

impl GpuSetting {
    pub fn to_preference(self) -> GpuPreference {
        match self {
            GpuSetting::Cpu => GpuPreference::Cpu,
            GpuSetting::Auto => GpuPreference::Auto,
        }
    }

    pub fn from_preference(preference: GpuPreference) -> Self {
        match preference {
            GpuPreference::Cpu => GpuSetting::Cpu,
            GpuPreference::Auto | GpuPreference::On => GpuSetting::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub use_gpu: GpuSetting,
}

pub fn settings_path() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("Yap").join("settings.json");
    }
    PathBuf::from("settings.json")
}

pub fn load_settings() -> AppSettings {
    let path = settings_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return AppSettings::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("Failed to create settings dir: {err}"))?;
    }
    let body = serde_json::to_string_pretty(settings).map_err(|err| format!("Failed to encode settings: {err}"))?;
    std::fs::write(&path, body).map_err(|err| format!("Failed to write settings: {err}"))
}

pub fn effective_gpu_preference() -> GpuPreference {
    if let Ok(env) = std::env::var("YAP_USE_GPU") {
        let trimmed = env.trim();
        if !trimmed.is_empty() {
            return crate::stt::gpu::gpu_preference_from(Some(trimmed));
        }
    }
    load_settings().use_gpu.to_preference()
}

pub fn polish_num_gpu_layers() -> u32 {
    match effective_gpu_preference() {
        GpuPreference::Cpu => 0,
        GpuPreference::Auto | GpuPreference::On => 99,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_prefer_cpu() {
        let settings = AppSettings::default();
        assert_eq!(settings.use_gpu, GpuSetting::Cpu);
        assert_eq!(settings.use_gpu.to_preference(), GpuPreference::Cpu);
    }

    #[test]
    fn round_trip_json() {
        let settings = AppSettings { use_gpu: GpuSetting::Auto };
        let json = serde_json::to_string(&settings).unwrap();
        let parsed: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, settings);
    }
}
