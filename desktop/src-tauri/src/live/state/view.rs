use crate::live::settings::LiveSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LiveOverlayVisibility {
    Enabled,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LiveCaptureMode {
    PushToTalk,
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LiveSessionStatus {
    Idle,
    Armed,
    Listening,
    Speaking,
    Settling,
    Blocked,
    Saving,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LiveRoute {
    ServerLive,
    LocalFallback,
    Blocked,
    None,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveInputDeviceView {
    pub id: String,
    pub label: String,
    pub is_default: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSessionView {
    pub visibility: LiveOverlayVisibility,
    pub status: LiveSessionStatus,
    pub route: LiveRoute,
    pub capture_mode: LiveCaptureMode,
    pub active_capture_mode: Option<LiveCaptureMode>,
    pub hotkey: String,
    pub paste_hotkey: String,
    pub input_device_id: Option<String>,
    pub input_device_label: Option<String>,
    pub level: Option<f32>,
    pub partial_text: Option<String>,
    pub final_text: Option<String>,
    pub transcription_degraded: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveOverlayView {
    pub visibility: LiveOverlayVisibility,
    pub status: LiveSessionStatus,
    pub capture_mode: LiveCaptureMode,
    pub active_capture_mode: Option<LiveCaptureMode>,
    pub level: Option<f32>,
    pub has_final_text: bool,
    pub error: Option<String>,
}

impl From<&LiveSessionView> for LiveOverlayView {
    fn from(view: &LiveSessionView) -> Self {
        Self {
            visibility: view.visibility,
            status: view.status,
            capture_mode: view.capture_mode,
            active_capture_mode: view.active_capture_mode,
            level: view.level,
            has_final_text: view
                .final_text
                .as_deref()
                .is_some_and(|text| !text.trim().is_empty()),
            error: view.error.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveLevelView {
    pub level: Option<f32>,
}

impl From<&LiveSessionView> for LiveLevelView {
    fn from(view: &LiveSessionView) -> Self {
        Self { level: view.level }
    }
}

impl LiveSessionView {
    pub fn from_settings(settings: &LiveSettings) -> Self {
        let hotkey = settings.hotkey.clone().unwrap_or_default();
        let paste_hotkey = settings
            .paste_hotkey
            .clone()
            .filter(|paste| !crate::live::hotkeys::configured_hotkeys_match(&hotkey, paste))
            .unwrap_or_default();
        Self {
            visibility: if settings.overlay_enabled {
                LiveOverlayVisibility::Enabled
            } else {
                LiveOverlayVisibility::Hidden
            },
            status: LiveSessionStatus::Idle,
            route: LiveRoute::None,
            capture_mode: settings.capture_mode,
            active_capture_mode: None,
            hotkey,
            paste_hotkey,
            input_device_id: settings.input_device_id.clone(),
            input_device_label: settings.input_device_id.clone(),
            level: Some(0.0),
            partial_text: None,
            final_text: None,
            transcription_degraded: false,
            error: None,
        }
    }
}
