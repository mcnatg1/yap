use std::sync::Mutex;

use crate::{live::settings::LiveSettings, runtime};

use super::{
    memory::SessionMemory,
    view::{LiveCaptureMode, LiveRoute, LiveSessionStatus, LiveSessionView},
};

pub struct LiveSessionState {
    memory: SessionMemory,
    view: Mutex<LiveSessionView>,
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
            memory: SessionMemory::default(),
            view: Mutex::new(LiveSessionView::from_settings(&settings)),
        }
    }

    pub(crate) fn mark_startup_shortcut_failure(&self, is_paste: bool) {
        self.memory.mark_startup_shortcut_failure(is_paste);
    }

    pub(crate) fn take_startup_shortcut_failure(&self, is_paste: bool) -> bool {
        self.memory.take_startup_shortcut_failure(is_paste)
    }

    pub(crate) fn clear_startup_shortcut_failure(&self, is_paste: bool) {
        self.memory.clear_startup_shortcut_failure(is_paste);
    }

    pub fn last_completed_transcript(&self) -> Option<String> {
        self.memory.last_completed_transcript()
    }

    pub fn remember_completed_transcript(&self, text: &str) {
        self.memory.remember_completed_transcript(text);
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

    pub(crate) fn try_begin_local_start(
        &self,
        active_capture_mode: LiveCaptureMode,
        input_device_id: Option<String>,
        input_device_label: Option<String>,
    ) -> Option<LiveSessionView> {
        let mut view = self.view.lock().expect("live state poisoned");
        if !matches!(
            view.status,
            LiveSessionStatus::Idle | LiveSessionStatus::Blocked
        ) {
            return None;
        }
        view.error = None;
        view.input_device_id = input_device_id;
        view.input_device_label = input_device_label;
        view.level = Some(0.0);
        view.route = LiveRoute::LocalFallback;
        view.status = LiveSessionStatus::Armed;
        view.active_capture_mode = Some(active_capture_mode);
        Some(view.clone())
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
