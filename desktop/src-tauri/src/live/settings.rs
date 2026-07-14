use std::path::PathBuf;

use super::state::{LiveCaptureMode, LiveOverlayVisibility, LiveSessionView};

pub const DEFAULT_HOTKEY: &str = "Ctrl+Shift+Space";
pub const DEFAULT_PASTE_HOTKEY: &str = "Ctrl+Shift+Alt+V";

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
            paste_hotkey: Some(DEFAULT_PASTE_HOTKEY.into()),
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
    save_to_path(settings, &path)
}

pub(crate) fn save_view(view: &LiveSessionView) -> Result<(), String> {
    save(&LiveSettings {
        overlay_enabled: view.visibility == LiveOverlayVisibility::Enabled,
        hotkey: (!view.hotkey.is_empty()).then(|| view.hotkey.clone()),
        paste_hotkey: (!view.paste_hotkey.is_empty()).then(|| view.paste_hotkey.clone()),
        capture_mode: view.capture_mode,
        input_device_id: view.input_device_id.clone(),
    })
}

fn save_to_path(settings: &LiveSettings, path: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create settings directory: {err}"))?;
    }
    let text = serde_json::to_string_pretty(settings)
        .map_err(|err| format!("Failed to serialize live settings: {err}"))?;
    std::fs::remove_file(path.with_extension("json.part")).ok();
    crate::stt::model::write_text_atomically(path, &text)
        .map_err(|err| format!("Failed to save live settings: {err}"))
}

pub fn settings_dir_from(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    crate::paths::app_data_dir_from(env)
}

fn settings_path() -> PathBuf {
    crate::paths::app_data_dir().join("live-settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_live_settings_are_push_to_talk() {
        let settings = LiveSettings::default();

        assert_eq!(settings.hotkey.as_deref(), Some(DEFAULT_HOTKEY));
        assert_eq!(settings.paste_hotkey.as_deref(), Some(DEFAULT_PASTE_HOTKEY));
        assert_eq!(settings.capture_mode, LiveCaptureMode::PushToTalk);
        assert!(settings.overlay_enabled);
    }

    #[test]
    fn settings_dir_uses_app_data_override() {
        let local = std::env::temp_dir().join("local-data");
        let dir = settings_dir_from(|key| {
            (key == "YAP_APP_DATA_DIR").then(|| local.display().to_string())
        });

        assert_eq!(dir, local);
    }

    #[test]
    fn save_live_settings_replaces_stale_partial_file() {
        let dir = std::env::temp_dir().join(format!(
            "yap-live-settings-{}-{}",
            std::process::id(),
            crate::live::recordings::unix_millis_now().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("live-settings.json");
        let partial = dir.join("live-settings.json.part");
        std::fs::write(&partial, "stale").unwrap();

        save_to_path(&LiveSettings::default(), &path).unwrap();

        assert!(path.exists());
        assert!(!partial.exists());
        std::fs::remove_dir_all(dir).ok();
    }
}
