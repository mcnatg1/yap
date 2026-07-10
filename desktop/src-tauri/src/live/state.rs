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
pub struct LiveLevelView {
    pub level: Option<f32>,
    pub status: LiveSessionStatus,
}

impl From<&LiveSessionView> for LiveLevelView {
    fn from(view: &LiveSessionView) -> Self {
        Self {
            level: view.level,
            status: view.status,
        }
    }
}

impl LiveSessionView {
    pub fn from_settings(settings: &LiveSettings) -> Self {
        let hotkey = settings.hotkey.clone().unwrap_or_default();
        let paste_hotkey = settings
            .paste_hotkey
            .clone()
            .filter(|paste| !super::hotkeys::configured_hotkeys_match(&hotkey, paste))
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

pub struct LiveSessionState {
    last_completed_transcript: Mutex<Option<String>>,
    startup_shortcut_failures: Mutex<StartupShortcutFailures>,
    view: Mutex<LiveSessionView>,
}

#[derive(Default)]
struct StartupShortcutFailures {
    dictation: bool,
    paste_last: bool,
}

pub fn is_live_capture_active(status: LiveSessionStatus) -> bool {
    matches!(
        status,
        LiveSessionStatus::Listening | LiveSessionStatus::Speaking | LiveSessionStatus::Settling
    )
}

pub fn is_live_session_started(status: LiveSessionStatus) -> bool {
    is_live_capture_active(status)
        || matches!(status, LiveSessionStatus::Armed | LiveSessionStatus::Saving)
}

impl LiveSessionState {
    pub fn new(settings: LiveSettings) -> Self {
        Self {
            last_completed_transcript: Mutex::new(None),
            startup_shortcut_failures: Mutex::new(StartupShortcutFailures::default()),
            view: Mutex::new(LiveSessionView::from_settings(&settings)),
        }
    }

    pub(crate) fn mark_startup_shortcut_failure(&self, is_paste: bool) {
        let mut failures = self
            .startup_shortcut_failures
            .lock()
            .expect("live startup shortcut state poisoned");
        if is_paste {
            failures.paste_last = true;
        } else {
            failures.dictation = true;
        }
    }

    pub(crate) fn take_startup_shortcut_failure(&self, is_paste: bool) -> bool {
        let mut failures = self
            .startup_shortcut_failures
            .lock()
            .expect("live startup shortcut state poisoned");
        let failed = if is_paste {
            &mut failures.paste_last
        } else {
            &mut failures.dictation
        };
        std::mem::take(failed)
    }

    pub(crate) fn clear_startup_shortcut_failure(&self, is_paste: bool) {
        let _ = self.take_startup_shortcut_failure(is_paste);
    }

    pub fn last_completed_transcript(&self) -> Option<String> {
        self.last_completed_transcript
            .lock()
            .expect("live completed transcript state poisoned")
            .clone()
    }

    pub fn remember_completed_transcript(&self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        *self
            .last_completed_transcript
            .lock()
            .expect("live completed transcript state poisoned") = Some(text.to_string());
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
        self.clear_startup_shortcut_failure(false);
        self.update(|view| {
            view.error = None;
            view.level = Some(0.0);
            view.transcription_degraded = false;
            let route = live_route_for(setup, server_ready);
            view.route = route;
            view.active_capture_mode = (route != LiveRoute::Blocked).then_some(view.capture_mode);
            view.status = if route == LiveRoute::Blocked {
                LiveSessionStatus::Blocked
            } else {
                LiveSessionStatus::Armed
            };
            if route == LiveRoute::Blocked {
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
            view.active_capture_mode = None;
        })
    }

    pub fn try_begin_saving(&self, runtime_active: bool) -> Option<LiveSessionView> {
        let mut view = self.view.lock().expect("live state poisoned");
        if view.status == LiveSessionStatus::Saving
            || (!is_live_session_started(view.status) && !runtime_active)
        {
            return None;
        }
        view.error = None;
        view.level = Some(0.0);
        view.status = LiveSessionStatus::Saving;
        view.active_capture_mode = None;
        Some(view.clone())
    }

    pub fn finish_saving(&self) -> LiveSessionView {
        self.update(|view| {
            view.level = Some(0.0);
            view.partial_text = None;
            view.route = LiveRoute::None;
            view.status = LiveSessionStatus::Idle;
            view.active_capture_mode = None;
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
        self.clear_startup_shortcut_failure(false);
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
                view.active_capture_mode = None;
                view.error = Some("Server unavailable and local fallback is not ready.".into());
            }
        })
    }

    pub fn try_begin_listening_from_armed(&self) -> Option<LiveSessionView> {
        let mut view = self.view.lock().expect("live state poisoned");
        if view.status != LiveSessionStatus::Armed {
            return None;
        }
        {
            view.error = None;
            view.final_text = None;
            view.level = Some(0.0);
            view.partial_text = None;
            view.transcription_degraded = false;
            view.route = LiveRoute::LocalFallback;
            view.status = LiveSessionStatus::Listening;
            let capture_mode = view.capture_mode;
            view.active_capture_mode.get_or_insert(capture_mode);
        }
        Some(view.clone())
    }

    pub fn update_if_saving(
        &self,
        update: impl FnOnce(&mut LiveSessionView),
    ) -> Option<LiveSessionView> {
        let mut view = self.view.lock().expect("live state poisoned");
        if view.status != LiveSessionStatus::Saving {
            return None;
        }
        update(&mut view);
        Some(view.clone())
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
            if matches!(
                view.status,
                LiveSessionStatus::Idle | LiveSessionStatus::Blocked
            ) {
                return;
            }
            if view.status != LiveSessionStatus::Saving {
                view.error = None;
            }
            view.partial_text = Some(text.to_string());
            if view.status != LiveSessionStatus::Saving {
                view.status = LiveSessionStatus::Speaking;
            }
        })
    }

    pub fn update_final(&self, text: &str) -> LiveSessionView {
        self.update(|view| {
            if matches!(
                view.status,
                LiveSessionStatus::Idle | LiveSessionStatus::Blocked
            ) {
                return;
            }
            if view.status != LiveSessionStatus::Saving {
                view.error = None;
            }
            view.partial_text = None;
            view.final_text = Some(append_final_text(view.final_text.as_deref(), text));
            if view.status != LiveSessionStatus::Saving {
                view.status = LiveSessionStatus::Settling;
            }
        })
    }

    pub fn mark_transcription_backpressure(&self) -> LiveSessionView {
        self.update(|view| {
            if !is_live_session_started(view.status) {
                return;
            }
            view.transcription_degraded = true;
            view.error =
                Some("Live transcription is catching up. Audio will still be saved.".into());
        })
    }

    pub fn mark_transcription_degraded(&self) -> LiveSessionView {
        self.update(|view| {
            if is_live_session_started(view.status) {
                view.transcription_degraded = true;
            }
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
        self.clear_startup_shortcut_failure(false);
        self.update(|view| {
            let was_saving = view.status == LiveSessionStatus::Saving;
            view.error = Some(message.to_string());
            view.level = Some(0.0);
            if was_saving {
                view.transcription_degraded = true;
                return;
            }
            view.partial_text = None;
            view.route = LiveRoute::Blocked;
            view.status = LiveSessionStatus::Blocked;
            view.active_capture_mode = None;
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
    fn live_state_restores_paste_hotkey_settings() {
        let view = LiveSessionView::from_settings(&LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: Some("Ctrl+Shift+V".into()),
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });

        assert_eq!(view.paste_hotkey, "Ctrl+Shift+V");
    }

    #[test]
    fn live_state_rejects_a_paste_hotkey_that_conflicts_with_dictation() {
        let view = LiveSessionView::from_settings(&LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: Some("Ctrl+Shift+Space".into()),
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });

        assert_eq!(view.hotkey, "Ctrl+Shift+Space");
        assert_eq!(view.paste_hotkey, "");
    }

    #[test]
    fn last_completed_transcript_survives_a_new_active_session() {
        let state = LiveSessionState::new(LiveSettings::default());
        state.update_partial("unfinished words");
        assert_eq!(state.last_completed_transcript(), None);

        state.remember_completed_transcript("finished words");
        state.update(|view| view.status = LiveSessionStatus::Armed);
        state.try_begin_listening_from_armed().unwrap();
        state.update_partial("new unfinished words");

        assert_eq!(
            state.last_completed_transcript().as_deref(),
            Some("finished words")
        );
    }

    #[test]
    fn capture_start_cannot_overwrite_a_saving_lease() {
        let state = LiveSessionState::new(LiveSettings::default());
        state.update(|view| view.status = LiveSessionStatus::Armed);
        state.try_begin_saving(false).unwrap();

        assert!(state.try_begin_listening_from_armed().is_none());
        assert_eq!(state.snapshot().status, LiveSessionStatus::Saving);
    }

    #[test]
    fn startup_shortcut_failures_are_tracked_independently() {
        let state = LiveSessionState::new(LiveSettings::default());
        state.mark_startup_shortcut_failure(false);
        state.mark_startup_shortcut_failure(true);

        assert!(state.take_startup_shortcut_failure(true));
        assert!(!state.take_startup_shortcut_failure(true));
        assert!(state.take_startup_shortcut_failure(false));
        assert!(!state.take_startup_shortcut_failure(false));
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
            paste_hotkey: None,
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
            paste_hotkey: None,
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
            paste_hotkey: None,
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
            paste_hotkey: None,
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });
        state.update(|view| view.status = LiveSessionStatus::Speaking);
        state.update_final("hello.");

        let view = state.stop();

        assert_eq!(view.final_text.as_deref(), Some("hello."));
    }

    #[test]
    fn saving_counts_as_busy_while_live_stop_drains() {
        assert!(is_live_session_started(LiveSessionStatus::Saving));
    }

    #[test]
    fn saving_keeps_drain_text_without_returning_to_recording() {
        let state = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: None,
            capture_mode: LiveCaptureMode::Toggle,
            input_device_id: None,
        });
        state.update(|view| {
            view.active_capture_mode = Some(LiveCaptureMode::Toggle);
            view.route = LiveRoute::LocalFallback;
            view.status = LiveSessionStatus::Listening;
        });

        let saving = state.try_begin_saving(false).unwrap();
        assert_eq!(saving.status, LiveSessionStatus::Saving);
        assert_eq!(saving.active_capture_mode, None);

        let final_view = state.update_final("tail text");
        assert_eq!(final_view.status, LiveSessionStatus::Saving);
        assert_eq!(final_view.final_text.as_deref(), Some("tail text"));

        let listening = state.return_to_listening();
        assert_eq!(listening.status, LiveSessionStatus::Saving);
    }

    #[test]
    fn saving_block_keeps_the_finalization_lease_and_partial_text() {
        let state = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: None,
            capture_mode: LiveCaptureMode::Toggle,
            input_device_id: None,
        });
        state.update(|view| view.status = LiveSessionStatus::Speaking);
        state.update_partial("draft");
        state.try_begin_saving(false).unwrap();

        let view = state.block_with_error("Live stream stopped.");

        assert_eq!(view.status, LiveSessionStatus::Saving);
        assert_eq!(view.partial_text.as_deref(), Some("draft"));
        assert!(view.transcription_degraded);
    }

    #[test]
    fn saving_transcript_updates_preserve_completion_errors() {
        let state = LiveSessionState::new(LiveSettings::default());
        state.update(|view| view.status = LiveSessionStatus::Speaking);
        state.try_begin_saving(false).unwrap();
        state.update(|view| view.error = Some("Live transcription stopped unexpectedly.".into()));

        let partial = state.update_partial("tail");
        assert_eq!(
            partial.error.as_deref(),
            Some("Live transcription stopped unexpectedly.")
        );
        let final_view = state.update_final("tail text");
        assert_eq!(
            final_view.error.as_deref(),
            Some("Live transcription stopped unexpectedly.")
        );
    }

    #[test]
    fn a_new_block_supersedes_startup_shortcut_block_ownership() {
        let state = LiveSessionState::new(LiveSettings::default());
        state.mark_startup_shortcut_failure(false);

        state.block_with_error("Local model unavailable.");

        assert!(!state.take_startup_shortcut_failure(false));
    }

    #[test]
    fn saving_claim_allows_only_one_stop_finalizer() {
        let state = LiveSessionState::new(LiveSettings::default());
        state.update(|view| view.status = LiveSessionStatus::Speaking);

        assert!(state.try_begin_saving(false).is_some());
        assert!(state.try_begin_saving(true).is_none());
        assert_eq!(state.snapshot().status, LiveSessionStatus::Saving);
    }

    #[test]
    fn conditional_saving_updates_cannot_touch_idle_or_new_sessions() {
        let state = LiveSessionState::new(LiveSettings::default());
        assert!(state
            .update_if_saving(|view| view.error = Some("stale".into()))
            .is_none());

        state.update(|view| view.status = LiveSessionStatus::Speaking);
        state.try_begin_saving(false).unwrap();
        assert!(state
            .update_if_saving(|view| view.error = Some("current".into()))
            .is_some());
        state.finish_saving();

        assert!(state
            .update_if_saving(|view| view.error = Some("stale".into()))
            .is_none());
        assert_eq!(state.snapshot().error.as_deref(), Some("current"));
    }

    #[test]
    fn finish_saving_returns_idle_without_erasing_completion_error() {
        let state = LiveSessionState::new(LiveSettings::default());
        state.update(|view| view.status = LiveSessionStatus::Speaking);
        state.try_begin_saving(false).unwrap();
        state.update(|view| view.error = Some("Couldn't insert text.".into()));

        let view = state.finish_saving();

        assert_eq!(view.status, LiveSessionStatus::Idle);
        assert_eq!(view.error.as_deref(), Some("Couldn't insert text."));
    }

    #[test]
    fn final_event_settles_then_listens() {
        let state = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: None,
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
            paste_hotkey: None,
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });
        state.update(|view| view.status = LiveSessionStatus::Speaking);
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
            paste_hotkey: None,
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });
        state.update(|view| view.status = LiveSessionStatus::Listening);

        let view = state.update_level(0.35);

        assert_eq!(view.status, LiveSessionStatus::Speaking);
        assert_eq!(view.level, Some(0.35));
    }

    #[test]
    fn level_view_does_not_serialize_transcript_text() {
        let mut view = LiveSessionView::from_settings(&LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: None,
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });
        view.partial_text = Some("large partial".into());
        view.final_text = Some("large final".into());
        view.level = Some(0.5);
        view.status = LiveSessionStatus::Speaking;

        let payload = serde_json::to_value(LiveLevelView::from(&view)).unwrap();

        assert_eq!(
            payload.get("level").and_then(serde_json::Value::as_f64),
            Some(0.5)
        );
        assert_eq!(
            payload.get("status").and_then(serde_json::Value::as_str),
            Some("speaking")
        );
        assert!(payload.get("partialText").is_none());
        assert!(payload.get("finalText").is_none());
    }

    #[test]
    fn backpressure_warning_does_not_reopen_idle_sessions() {
        let state = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: None,
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });

        assert_eq!(state.mark_transcription_backpressure().error, None);
        state.update(|view| view.status = LiveSessionStatus::Speaking);

        let warning = state.mark_transcription_backpressure();
        assert_eq!(
            warning.error.as_deref(),
            Some("Live transcription is catching up. Audio will still be saved.")
        );
        assert!(warning.transcription_degraded);

        let partial = state.update_partial("caught up");
        assert_eq!(partial.error, None);
        assert!(partial.transcription_degraded);
    }

    #[test]
    fn stale_stream_text_does_not_reopen_idle_session() {
        let state = LiveSessionState::new(LiveSettings {
            overlay_enabled: true,
            hotkey: Some("Ctrl+Shift+Space".into()),
            paste_hotkey: None,
            capture_mode: LiveCaptureMode::PushToTalk,
            input_device_id: None,
        });

        state.update_partial("late partial");
        state.update_final("late final");
        let view = state.snapshot();

        assert_eq!(view.status, LiveSessionStatus::Idle);
        assert_eq!(view.partial_text, None);
        assert_eq!(view.final_text, None);
    }
}
