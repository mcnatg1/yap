use std::sync::Mutex;

use crate::runtime;

use super::settings::LiveSettings;

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
    pub hotkey: String,
    pub input_device_id: Option<String>,
    pub input_device_label: Option<String>,
    pub level: Option<f32>,
    pub partial_text: Option<String>,
    pub final_text: Option<String>,
    pub error: Option<String>,
}

impl LiveSessionView {
    pub fn from_settings(settings: &LiveSettings) -> Self {
        Self {
            visibility: if settings.overlay_enabled {
                LiveOverlayVisibility::Enabled
            } else {
                LiveOverlayVisibility::Hidden
            },
            status: LiveSessionStatus::Idle,
            route: LiveRoute::None,
            capture_mode: settings.capture_mode,
            hotkey: settings.hotkey.clone().unwrap_or_default(),
            input_device_id: settings.input_device_id.clone(),
            input_device_label: settings.input_device_id.clone(),
            level: Some(0.0),
            partial_text: None,
            final_text: None,
            error: None,
        }
    }
}

pub struct LiveSessionState {
    view: Mutex<LiveSessionView>,
}

pub fn is_live_capture_active(status: LiveSessionStatus) -> bool {
    matches!(
        status,
        LiveSessionStatus::Listening | LiveSessionStatus::Speaking | LiveSessionStatus::Settling
    )
}

pub fn is_live_session_started(status: LiveSessionStatus) -> bool {
    is_live_capture_active(status) || status == LiveSessionStatus::Armed
}

impl LiveSessionState {
    pub fn new(settings: LiveSettings) -> Self {
        Self {
            view: Mutex::new(LiveSessionView::from_settings(&settings)),
        }
    }

    pub fn snapshot(&self) -> LiveSessionView {
        self.view.lock().expect("live state poisoned").clone()
    }

    pub fn update(&self, update: impl FnOnce(&mut LiveSessionView)) -> LiveSessionView {
        let mut view = self.view.lock().expect("live state poisoned");
        update(&mut view);
        view.clone()
    }

    pub fn start(&self, setup: runtime::state::SetupState, server_ready: bool) -> LiveSessionView {
        self.update(|view| {
            view.error = None;
            view.level = Some(0.0);
            view.route = live_route_for(setup, server_ready);
            view.status = if view.route == LiveRoute::Blocked {
                LiveSessionStatus::Blocked
            } else {
                LiveSessionStatus::Armed
            };
            if view.route == LiveRoute::Blocked {
                view.error = Some(blocked_message(setup).into());
            }
        })
    }

    pub fn stop(&self) -> LiveSessionView {
        self.update(|view| {
            view.error = None;
            view.level = Some(0.0);
            view.partial_text = None;
            view.route = LiveRoute::None;
            view.status = LiveSessionStatus::Idle;
        })
    }

    pub fn toggle(&self, setup: runtime::state::SetupState, server_ready: bool) -> LiveSessionView {
        if is_live_session_started(self.snapshot().status) {
            self.stop()
        } else {
            self.start(setup, server_ready)
        }
    }

    pub fn route_loss(&self, fallback_ready: bool) -> LiveSessionView {
        self.update(|view| {
            if view.route != LiveRoute::ServerLive {
                return;
            }
            if fallback_ready {
                view.route = LiveRoute::LocalFallback;
                view.error = Some("Server unavailable. Using local fallback.".into());
            } else {
                view.route = LiveRoute::Blocked;
                view.status = LiveSessionStatus::Blocked;
                view.error = Some("Server unavailable and local fallback is not ready.".into());
            }
        })
    }

    pub fn clear_for_new_session(&self) -> LiveSessionView {
        self.update(|view| {
            view.error = None;
            view.final_text = None;
            view.level = Some(0.0);
            view.partial_text = None;
            view.route = LiveRoute::LocalFallback;
            view.status = LiveSessionStatus::Listening;
        })
    }

    pub fn update_level(&self, level: f32) -> LiveSessionView {
        self.update(|view| {
            let level = level.clamp(0.0, 1.0);
            view.level = Some(level);
            match view.status {
                LiveSessionStatus::Listening if level >= 0.18 => {
                    view.status = LiveSessionStatus::Speaking;
                }
                LiveSessionStatus::Speaking if level <= 0.08 => {
                    view.status = LiveSessionStatus::Listening;
                }
                _ => {}
            }
        })
    }

    pub fn update_partial(&self, text: &str) -> LiveSessionView {
        self.update(|view| {
            view.error = None;
            view.partial_text = Some(text.to_string());
            view.status = LiveSessionStatus::Speaking;
        })
    }

    pub fn update_final(&self, text: &str) -> LiveSessionView {
        self.update(|view| {
            view.error = None;
            view.partial_text = None;
            view.final_text = Some(append_final_text(view.final_text.as_deref(), text));
            view.status = LiveSessionStatus::Settling;
        })
    }

    pub fn return_to_listening(&self) -> LiveSessionView {
        self.update(|view| {
            if matches!(
                view.status,
                LiveSessionStatus::Listening
                    | LiveSessionStatus::Speaking
                    | LiveSessionStatus::Settling
            ) {
                view.status = LiveSessionStatus::Listening;
            }
            view.level = Some(0.0);
        })
    }

    pub fn block_with_error(&self, message: &str) -> LiveSessionView {
        self.update(|view| {
            view.error = Some(message.to_string());
            view.level = Some(0.0);
            view.partial_text = None;
            view.route = LiveRoute::Blocked;
            view.status = LiveSessionStatus::Blocked;
        })
    }
}

pub fn live_route_for(setup: runtime::state::SetupState, server_ready: bool) -> LiveRoute {
    if server_ready {
        return LiveRoute::ServerLive;
    }
    match setup {
        runtime::state::SetupState::FallbackReady => LiveRoute::LocalFallback,
        _ => LiveRoute::Blocked,
    }
}

fn blocked_message(setup: runtime::state::SetupState) -> &'static str {
    match setup {
        runtime::state::SetupState::FallbackDisabled => "Local fallback is disabled.",
        runtime::state::SetupState::SetupError => "Local fallback needs attention.",
        _ => "Local fallback is not ready.",
    }
}

fn append_final_text(existing: Option<&str>, next: &str) -> String {
    let next = next.trim();
    match existing.map(str::trim).filter(|text| !text.is_empty()) {
        Some(existing) if !next.is_empty() => format!("{existing} {next}"),
        Some(existing) => existing.to_string(),
        None => next.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_start_uses_local_fallback_without_server() {
        assert_eq!(
            live_route_for(runtime::state::SetupState::FallbackReady, false),
            LiveRoute::LocalFallback
        );
    }

    #[test]
    fn live_start_blocks_without_any_route() {
        assert_eq!(
            live_route_for(runtime::state::SetupState::FallbackMissing, false),
            LiveRoute::Blocked
        );
    }

    #[test]
    fn route_loss_downgrades_server_to_fallback() {
        let state = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });
        state.update(|view| {
            view.route = LiveRoute::ServerLive;
            view.status = LiveSessionStatus::Listening;
        });

        let view = state.route_loss(true);

        assert_eq!(view.route, LiveRoute::LocalFallback);
        assert_eq!(
            view.error.as_deref(),
            Some("Server unavailable. Using local fallback.")
        );
    }

    #[test]
    fn toggle_stops_active_capture() {
        let state = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            capture_mode: LiveCaptureMode::Toggle,
            input_device_id: None,
        });
        state.start(runtime::state::SetupState::FallbackReady, false);

        let view = state.toggle(runtime::state::SetupState::FallbackReady, false);

        assert_eq!(view.status, LiveSessionStatus::Idle);
        assert_eq!(view.route, LiveRoute::None);
    }

    #[test]
    fn start_arms_without_claiming_mic_capture() {
        let state = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });

        let view = state.start(runtime::state::SetupState::FallbackReady, false);

        assert_eq!(view.status, LiveSessionStatus::Armed);
        assert!(!is_live_capture_active(view.status));
    }

    #[test]
    fn stop_preserves_final_text() {
        let state = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });
        state.update_final("hello.");

        let view = state.stop();

        assert_eq!(view.final_text.as_deref(), Some("hello."));
    }

    #[test]
    fn final_event_settles_then_listens() {
        let state = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });
        state.update(|view| view.status = LiveSessionStatus::Speaking);

        let view = state.update_final("hello.");

        assert_eq!(view.status, LiveSessionStatus::Settling);
        let view = state.return_to_listening();
        assert_eq!(view.status, LiveSessionStatus::Listening);
        assert_eq!(view.final_text.as_deref(), Some("hello."));
    }

    #[test]
    fn stream_crash_blocks_without_losing_final_text() {
        let state = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });
        state.update_final("kept.");

        let view = state.block_with_error("Live stream stopped.");

        assert_eq!(view.status, LiveSessionStatus::Blocked);
        assert_eq!(view.final_text.as_deref(), Some("kept."));
    }

    #[test]
    fn level_updates_can_mark_speaking() {
        let state = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });
        state.update(|view| view.status = LiveSessionStatus::Listening);

        let view = state.update_level(0.35);

        assert_eq!(view.status, LiveSessionStatus::Speaking);
        assert_eq!(view.level, Some(0.35));
    }
}
