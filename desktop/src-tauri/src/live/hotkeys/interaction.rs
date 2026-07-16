use std::time::{Duration, Instant};

use crate::live::state::LiveCaptureMode;

pub const SHORTCUT_DOUBLE_TAP_MS: u64 = 320;
pub const SHORTCUT_HOLD_MS: u64 = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveShortcutAction {
    None,
    ScheduleHold(u64),
    Start(LiveCaptureMode),
    Stop,
}

#[derive(Debug, Default)]
pub struct LiveShortcutInteraction {
    active_mode: Option<LiveCaptureMode>,
    key_down: bool,
    last_tap_at: Option<Instant>,
    pending_press_at: Option<Instant>,
    pending_press_id: u64,
    pending_start_mode: Option<LiveCaptureMode>,
    suppress_release: bool,
    toggle_stop_armed: bool,
}

impl LiveShortcutInteraction {
    pub fn reset(&mut self) {
        self.active_mode = None;
        self.key_down = false;
        self.last_tap_at = None;
        self.pending_press_at = None;
        self.pending_start_mode = None;
        self.suppress_release = false;
        self.toggle_stop_armed = false;
    }

    pub fn finish_start(&mut self, active_mode: Option<LiveCaptureMode>) {
        let Some(started_mode) = self.pending_start_mode.take() else {
            return;
        };
        if self.active_mode == Some(started_mode) && active_mode != Some(started_mode) {
            self.clear_active_session();
            self.suppress_release = self.key_down;
        }
    }

    pub fn pressed(
        &mut self,
        now: Instant,
        projected_mode: Option<LiveCaptureMode>,
    ) -> LiveShortcutAction {
        self.reconcile(projected_mode);
        if self.key_down {
            return LiveShortcutAction::None;
        }
        self.key_down = true;
        match self.active_mode {
            Some(LiveCaptureMode::Toggle) if self.toggle_stop_armed => {
                self.clear_active_session();
                self.suppress_release = true;
                self.pending_press_at = None;
                self.last_tap_at = None;
                return LiveShortcutAction::Stop;
            }
            Some(_) => return LiveShortcutAction::None,
            None => {}
        }
        if self.last_tap_at.is_some_and(|then| {
            now.duration_since(then) <= Duration::from_millis(SHORTCUT_DOUBLE_TAP_MS)
        }) {
            self.pending_press_at = None;
            self.last_tap_at = None;
            self.begin_session(LiveCaptureMode::Toggle);
            return LiveShortcutAction::Start(LiveCaptureMode::Toggle);
        }

        self.pending_press_id = self.pending_press_id.wrapping_add(1);
        self.pending_press_at = Some(now);
        self.last_tap_at = None;
        LiveShortcutAction::ScheduleHold(self.pending_press_id)
    }

    pub fn hold_elapsed(
        &mut self,
        press_id: u64,
        now: Instant,
        projected_mode: Option<LiveCaptureMode>,
    ) -> LiveShortcutAction {
        self.reconcile(projected_mode);
        let Some(pressed_at) = self.pending_press_at else {
            return LiveShortcutAction::None;
        };
        if press_id != self.pending_press_id
            || self.active_mode.is_some()
            || now.duration_since(pressed_at) < Duration::from_millis(SHORTCUT_HOLD_MS)
        {
            return LiveShortcutAction::None;
        }

        self.pending_press_at = None;
        self.last_tap_at = None;
        self.begin_session(LiveCaptureMode::PushToTalk);
        LiveShortcutAction::Start(LiveCaptureMode::PushToTalk)
    }

    pub fn released(
        &mut self,
        now: Instant,
        projected_mode: Option<LiveCaptureMode>,
    ) -> LiveShortcutAction {
        self.reconcile(projected_mode);
        self.key_down = false;
        if self.suppress_release {
            self.suppress_release = false;
            return LiveShortcutAction::None;
        }
        match self.active_mode {
            Some(LiveCaptureMode::PushToTalk) => {
                self.clear_active_session();
                return LiveShortcutAction::Stop;
            }
            Some(LiveCaptureMode::Toggle) => {
                self.toggle_stop_armed = true;
                return LiveShortcutAction::None;
            }
            None => {}
        }
        if self.pending_press_at.take().is_some() {
            self.last_tap_at = Some(now);
        }
        LiveShortcutAction::None
    }

    fn begin_session(&mut self, mode: LiveCaptureMode) {
        self.active_mode = Some(mode);
        self.pending_start_mode = Some(mode);
        self.toggle_stop_armed = false;
    }

    fn clear_active_session(&mut self) {
        self.active_mode = None;
        self.toggle_stop_armed = false;
    }

    fn reconcile(&mut self, projected_mode: Option<LiveCaptureMode>) {
        if self.pending_start_mode.is_some() || self.suppress_release {
            return;
        }
        match (self.active_mode, projected_mode) {
            (None, Some(mode)) => {
                self.active_mode = Some(mode);
                self.toggle_stop_armed = mode == LiveCaptureMode::Toggle;
            }
            (Some(active), Some(projected)) if active != projected => {
                self.active_mode = Some(projected);
                self.toggle_stop_armed = projected == LiveCaptureMode::Toggle;
            }
            (Some(_), None) => self.clear_active_session(),
            _ => {}
        }
    }
}
