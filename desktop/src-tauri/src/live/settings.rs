use std::path::PathBuf;

use super::state::LiveCaptureMode;

pub const DEFAULT_HOTKEY: &str = "Ctrl+Shift+Space";

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSettings {
    pub overlay_enabled: bool,
    pub hotkey: Option<String>,
    pub paste_hotkey: Option<String>,
    pub capture_mode: LiveCaptureMode,
    pub input_device_id: Option<String>,
}

impl Default for LiveSettings {
    fn default() -> Self {
        Self {
            overlay_enabled: true,
            hotkey: Some(DEFAULT_HOTKEY.into()),
            paste_hotkey: None,
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        }
    }
}

pub fn load() -> LiveSettings {
    let path = settings_path();
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(settings: &LiveSettings) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create settings directory: {err}"))?;
    }
    let text = serde_json::to_string_pretty(settings)
        .map_err(|err| format!("Failed to serialize live settings: {err}"))?;
    std::fs::write(path, text).map_err(|err| format!("Failed to save live settings: {err}"))
}

pub fn settings_dir_from(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(local_app_data) = env("LOCALAPPDATA") {
        return PathBuf::from(local_app_data).join("Yap");
    }
    env("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".yap")
}

fn settings_path() -> PathBuf {
    settings_dir_from(|key| std::env::var(key).ok()).join("live-settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_live_settings_are_push_to_talk() {
        let settings = LiveSettings::default();

        assert_eq!(settings.hotkey.as_deref(), Some(DEFAULT_HOTKEY));
        assert_eq!(settings.paste_hotkey, None);
        assert_eq!(settings.capture_mode, LiveCaptureMode::PushToTalk);
        assert!(settings.overlay_enabled);
    }

    #[test]
    fn settings_dir_uses_local_app_data() {
        let dir = settings_dir_from(|key| {
            (key == "LOCALAPPDATA").then(|| "C:/Users/Test/AppData/Local".into())
        });

        assert_eq!(
            dir,
            PathBuf::from("C:/Users/Test/AppData/Local").join("Yap")
        );
    }
}
