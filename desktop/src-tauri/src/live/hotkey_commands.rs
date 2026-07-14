use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tauri_plugin_global_shortcut::Shortcut;

use crate::{authorization, live};

pub(crate) const DICTATION_UNAVAILABLE_ERROR: &str = "Live shortcut is unavailable.";
pub(crate) const PASTE_UNAVAILABLE_ERROR: &str = "Paste shortcut is unavailable.";
const HOTKEY_ENROLLMENT_WINDOW: Duration = Duration::from_secs(15);
const HOTKEY_POLL_INTERVAL: Duration = Duration::from_millis(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveHotkeyKind {
    Dictation,
    PasteLast,
}

impl LiveHotkeyKind {
    fn current(self, view: &live::state::LiveSessionView) -> &str {
        match self {
            Self::Dictation => &view.hotkey,
            Self::PasteLast => &view.paste_hotkey,
        }
    }

    fn conflicting(self, view: &live::state::LiveSessionView) -> &str {
        match self {
            Self::Dictation => &view.paste_hotkey,
            Self::PasteLast => &view.hotkey,
        }
    }

    fn conflict_message(self) -> &'static str {
        match self {
            Self::Dictation => "Dictation shortcut must differ from paste shortcut.",
            Self::PasteLast => "Paste shortcut must differ from dictation shortcut.",
        }
    }

    fn update(self, view: &mut live::state::LiveSessionView, hotkey: String) {
        match self {
            Self::Dictation => view.hotkey = hotkey,
            Self::PasteLast => view.paste_hotkey = hotkey,
        }
    }

    fn startup_error(self) -> &'static str {
        match self {
            Self::Dictation => DICTATION_UNAVAILABLE_ERROR,
            Self::PasteLast => PASTE_UNAVAILABLE_ERROR,
        }
    }

    fn is_paste(self) -> bool {
        matches!(self, Self::PasteLast)
    }

    fn purpose(self) -> live::hotkeys::HotkeyPurpose {
        match self {
            Self::Dictation => live::hotkeys::HotkeyPurpose::Dictation,
            Self::PasteLast => live::hotkeys::HotkeyPurpose::PasteLast,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Dictation => "dictation",
            Self::PasteLast => "paste-last",
        }
    }

    fn required_modifier_count(self) -> u32 {
        match self {
            Self::Dictation => 2,
            Self::PasteLast => 3,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct HotkeyEnrollmentGate {
    active: Arc<AtomicBool>,
}

impl HotkeyEnrollmentGate {
    fn try_begin(&self) -> Result<HotkeyEnrollmentLease, String> {
        self.active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Shortcut recording is already active.".to_string())?;
        Ok(HotkeyEnrollmentLease {
            active: Arc::clone(&self.active),
        })
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
struct HotkeyEnrollmentLease {
    active: Arc<AtomicBool>,
}

impl Drop for HotkeyEnrollmentLease {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PhysicalChordSnapshot {
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
    keys: Vec<String>,
}

impl PhysicalChordSnapshot {
    fn is_neutral(&self) -> bool {
        !self.ctrl && !self.shift && !self.alt && !self.meta && self.keys.is_empty()
    }

    fn modifier_count(&self) -> u32 {
        [self.ctrl, self.shift, self.alt, self.meta]
            .into_iter()
            .filter(|pressed| *pressed)
            .count() as u32
    }

    fn normalized_chord(&self, purpose: live::hotkeys::HotkeyPurpose) -> Result<String, String> {
        if self.keys.len() != 1 {
            return Err("Press exactly one shortcut key.".into());
        }
        let mut parts = Vec::with_capacity(5);
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.meta {
            parts.push("Meta".to_string());
        }
        parts.push(self.keys[0].clone());
        live::hotkeys::normalize_hotkey_for(&parts.join("+"), purpose)
    }

    fn contains_input_outside(&self, candidate: &Self) -> bool {
        (self.ctrl && !candidate.ctrl)
            || (self.shift && !candidate.shift)
            || (self.alt && !candidate.alt)
            || (self.meta && !candidate.meta)
            || self.keys.iter().any(|key| {
                !candidate
                    .keys
                    .iter()
                    .any(|candidate_key| candidate_key == key)
            })
    }
}

#[derive(Debug, Clone)]
enum HotkeyEnrollmentPhase {
    AwaitingNeutral,
    AwaitingChord,
    AwaitingRelease {
        candidate: PhysicalChordSnapshot,
        normalized: String,
    },
    Finished,
}

#[derive(Debug, PartialEq, Eq)]
enum HotkeyEnrollmentObservation {
    Pending,
    Complete(String),
    Cancelled,
}

struct HotkeyEnrollmentEpoch {
    kind: LiveHotkeyKind,
    expires_at: Instant,
    phase: HotkeyEnrollmentPhase,
}

impl HotkeyEnrollmentEpoch {
    fn arm(confirmed: bool, kind: LiveHotkeyKind, now: Instant) -> Option<Self> {
        confirmed.then_some(Self {
            kind,
            expires_at: now + HOTKEY_ENROLLMENT_WINDOW,
            phase: HotkeyEnrollmentPhase::AwaitingNeutral,
        })
    }

    fn observe(
        &mut self,
        now: Instant,
        snapshot: PhysicalChordSnapshot,
    ) -> Result<HotkeyEnrollmentObservation, String> {
        if matches!(self.phase, HotkeyEnrollmentPhase::Finished) {
            return Err("Shortcut recording epoch was already consumed.".into());
        }
        if now >= self.expires_at {
            self.phase = HotkeyEnrollmentPhase::Finished;
            return Err("Shortcut recording expired before a chord was completed.".into());
        }

        match self.phase.clone() {
            HotkeyEnrollmentPhase::AwaitingNeutral => {
                if snapshot.is_neutral() {
                    self.phase = HotkeyEnrollmentPhase::AwaitingChord;
                }
                Ok(HotkeyEnrollmentObservation::Pending)
            }
            HotkeyEnrollmentPhase::AwaitingChord => {
                if snapshot.keys.as_slice() == ["Escape"] && snapshot.modifier_count() == 0 {
                    self.phase = HotkeyEnrollmentPhase::Finished;
                    return Ok(HotkeyEnrollmentObservation::Cancelled);
                }
                if snapshot.modifier_count() < self.kind.required_modifier_count()
                    || snapshot.keys.is_empty()
                {
                    return Ok(HotkeyEnrollmentObservation::Pending);
                }
                let normalized = snapshot.normalized_chord(self.kind.purpose())?;
                self.phase = HotkeyEnrollmentPhase::AwaitingRelease {
                    candidate: snapshot,
                    normalized,
                };
                Ok(HotkeyEnrollmentObservation::Pending)
            }
            HotkeyEnrollmentPhase::AwaitingRelease {
                candidate,
                normalized,
            } => {
                if snapshot.contains_input_outside(&candidate) {
                    self.phase = HotkeyEnrollmentPhase::Finished;
                    return Err("Shortcut changed before the recorded chord was released.".into());
                }
                if snapshot.is_neutral() {
                    self.phase = HotkeyEnrollmentPhase::Finished;
                    return Ok(HotkeyEnrollmentObservation::Complete(normalized));
                }
                Ok(HotkeyEnrollmentObservation::Pending)
            }
            HotkeyEnrollmentPhase::Finished => unreachable!(),
        }
    }
}

#[cfg(windows)]
fn physical_chord_snapshot() -> PhysicalChordSnapshot {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    fn pressed(virtual_key: i32) -> bool {
        unsafe { GetAsyncKeyState(virtual_key) < 0 }
    }

    let mut keys = Vec::new();
    for virtual_key in b'A'..=b'Z' {
        if pressed(i32::from(virtual_key)) {
            keys.push(char::from(virtual_key).to_string());
        }
    }
    for virtual_key in b'0'..=b'9' {
        if pressed(i32::from(virtual_key)) {
            keys.push(char::from(virtual_key).to_string());
        }
    }
    for offset in 0..12_i32 {
        if pressed(0x70 + offset) {
            keys.push(format!("F{}", offset + 1));
        }
    }
    for (virtual_key, name) in [
        (0x08, "Backspace"),
        (0x09, "Tab"),
        (0x0d, "Enter"),
        (0x1b, "Escape"),
        (0x20, "Space"),
    ] {
        if pressed(virtual_key) {
            keys.push(name.to_string());
        }
    }

    PhysicalChordSnapshot {
        ctrl: pressed(0x11),
        shift: pressed(0x10),
        alt: pressed(0x12),
        meta: pressed(0x5b) || pressed(0x5c),
        keys,
    }
}

#[cfg(windows)]
fn capture_physical_hotkey(mut epoch: HotkeyEnrollmentEpoch) -> Result<Option<String>, String> {
    loop {
        match epoch.observe(Instant::now(), physical_chord_snapshot())? {
            HotkeyEnrollmentObservation::Pending => {
                std::thread::sleep(HOTKEY_POLL_INTERVAL);
            }
            HotkeyEnrollmentObservation::Complete(hotkey) => return Ok(Some(hotkey)),
            HotkeyEnrollmentObservation::Cancelled => return Ok(None),
        }
    }
}

#[cfg(not(windows))]
fn capture_physical_hotkey(_: HotkeyEnrollmentEpoch) -> Result<Option<String>, String> {
    Err("Physical shortcut recording is currently supported only on Windows.".into())
}

async fn confirm_hotkey_enrollment(
    app: tauri::AppHandle,
    kind: LiveHotkeyKind,
) -> Result<bool, String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

    let label = kind.label();
    let modifier_count = kind.required_modifier_count();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(format!(
                "Record a new {label} shortcut?\n\nAfter choosing Record, hold at least {modifier_count} modifier keys plus one key, then release the entire chord. Yap listens for only this one physical chord for 15 seconds. Press Escape to cancel."
            ))
            .title("Record physical shortcut")
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Record".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|error| format!("Could not show shortcut confirmation: {error}"))
}

fn apply_successful_hotkey_change(
    view: &mut live::state::LiveSessionView,
    kind: LiveHotkeyKind,
    hotkey: String,
    recovered_startup_failure: bool,
) {
    kind.update(view, hotkey);
    if !recovered_startup_failure {
        return;
    }

    if view.error.as_deref() == Some(kind.startup_error()) {
        view.error = None;
    }
    if matches!(kind, LiveHotkeyKind::Dictation) {
        view.route = live::state::LiveRoute::None;
        view.status = live::state::LiveSessionStatus::Idle;
    }
}

#[tauri::command]
pub(crate) async fn record_live_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    gate: tauri::State<'_, HotkeyEnrollmentGate>,
) -> Result<live::state::LiveSessionView, String> {
    record_live_hotkey_for(window, app, state, gate, LiveHotkeyKind::Dictation).await
}

#[tauri::command]
pub(crate) fn clear_live_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    change_live_hotkey(window, app, state, LiveHotkeyKind::Dictation, None)
}

#[tauri::command]
pub(crate) fn reset_live_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    change_live_hotkey(
        window,
        app,
        state,
        LiveHotkeyKind::Dictation,
        Some(live::settings::DEFAULT_HOTKEY.into()),
    )
}

#[tauri::command]
pub(crate) async fn record_live_paste_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    gate: tauri::State<'_, HotkeyEnrollmentGate>,
) -> Result<live::state::LiveSessionView, String> {
    record_live_hotkey_for(window, app, state, gate, LiveHotkeyKind::PasteLast).await
}

async fn record_live_hotkey_for(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    gate: tauri::State<'_, HotkeyEnrollmentGate>,
    kind: LiveHotkeyKind,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    ensure_live_hotkey_idle(state.snapshot().status)?;
    let _lease = gate.try_begin()?;
    let confirmed = confirm_hotkey_enrollment(app.clone(), kind).await?;
    let Some(epoch) = HotkeyEnrollmentEpoch::arm(confirmed, kind, Instant::now()) else {
        return Ok(state.snapshot());
    };
    let captured = tauri::async_runtime::spawn_blocking(move || capture_physical_hotkey(epoch))
        .await
        .map_err(|error| format!("Shortcut recording worker failed: {error}"))??;
    let Some(hotkey) = captured else {
        return Ok(state.snapshot());
    };
    change_live_hotkey(window, app, state, kind, Some(hotkey))
}

#[tauri::command]
pub(crate) fn clear_live_paste_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    change_live_hotkey(window, app, state, LiveHotkeyKind::PasteLast, None)
}

#[tauri::command]
pub(crate) fn reset_live_paste_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
) -> Result<live::state::LiveSessionView, String> {
    change_live_hotkey(
        window,
        app,
        state,
        LiveHotkeyKind::PasteLast,
        Some(live::settings::DEFAULT_PASTE_HOTKEY.into()),
    )
}

fn change_live_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    kind: LiveHotkeyKind,
    hotkey: Option<String>,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    ensure_live_hotkey_idle(state.snapshot().status)?;

    let snapshot = state.snapshot();
    let requested_value = hotkey
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let (next_value, next) = if requested_value.is_empty() {
        (String::new(), None)
    } else {
        let normalized = live::hotkeys::normalize_hotkey_for(&requested_value, kind.purpose())?;
        let shortcut = live::hotkeys::parse_hotkey_for(&normalized, kind.purpose())?;
        (normalized, Some(shortcut))
    };
    if live::hotkeys::configured_hotkeys_match(kind.conflicting(&snapshot), &next_value) {
        return Err(kind.conflict_message().into());
    }
    if kind.current(&snapshot) == next_value {
        return Ok(snapshot);
    }

    let previous = live::hotkeys::parse_hotkey_for(kind.current(&snapshot), kind.purpose()).ok();
    let mut prospective = snapshot.clone();
    kind.update(&mut prospective, next_value.clone());
    replace_hotkey_registration(
        previous,
        next,
        |shortcut| {
            app.global_shortcut()
                .unregister(shortcut)
                .map_err(|error| format!("Failed to unregister previous shortcut: {error}"))
        },
        |shortcut| {
            app.global_shortcut()
                .register(shortcut)
                .map_err(|error| error.to_string())
        },
        || live::settings::save_view(&prospective),
    )?;
    live::shortcut_runtime::reset(&app);

    let recovered_startup_failure = state.take_startup_shortcut_failure(kind.is_paste());
    let view = state.update(|view| {
        apply_successful_hotkey_change(view, kind, next_value, recovered_startup_failure);
    });
    live::events::emit_session(&app, &view);
    Ok(view)
}

fn ensure_live_hotkey_idle(status: live::state::LiveSessionStatus) -> Result<(), String> {
    if live::state::is_live_session_started(status) {
        return Err("Stop live before changing the shortcut.".into());
    }
    Ok(())
}

pub(crate) fn replace_hotkey_registration(
    previous: Option<Shortcut>,
    next: Option<Shortcut>,
    mut unregister: impl FnMut(Shortcut) -> Result<(), String>,
    mut register: impl FnMut(Shortcut) -> Result<(), String>,
    persist: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if let Some(shortcut) = previous.as_ref() {
        unregister(*shortcut)?;
    }

    if let Some(shortcut) = next.as_ref() {
        if let Err(error) = register(*shortcut) {
            return match previous.as_ref() {
                Some(previous) => register(*previous).map_or_else(
                    |restore_error| {
                        Err(format!(
                            "Shortcut is unavailable: {error}; failed to restore previous shortcut: {restore_error}"
                        ))
                    },
                    |_| Err(format!("Shortcut is unavailable: {error}")),
                ),
                None => Err(format!("Shortcut is unavailable: {error}")),
            };
        }
    }

    if let Err(error) = persist() {
        let mut rollback_errors = Vec::new();
        if let Some(shortcut) = next.as_ref() {
            if let Err(rollback_error) = unregister(*shortcut) {
                rollback_errors.push(format!(
                    "failed to unregister new shortcut: {rollback_error}"
                ));
            }
        }
        if let Some(shortcut) = previous.as_ref() {
            if let Err(rollback_error) = register(*shortcut) {
                rollback_errors.push(format!(
                    "failed to restore previous shortcut: {rollback_error}"
                ));
            }
        }
        if rollback_errors.is_empty() {
            return Err(error);
        }
        return Err(format!("{error}; {}", rollback_errors.join("; ")));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        time::{Duration, Instant},
    };

    use super::{
        apply_successful_hotkey_change, ensure_live_hotkey_idle, replace_hotkey_registration,
        HotkeyEnrollmentEpoch, HotkeyEnrollmentGate, HotkeyEnrollmentObservation, LiveHotkeyKind,
        PhysicalChordSnapshot, DICTATION_UNAVAILABLE_ERROR, HOTKEY_ENROLLMENT_WINDOW,
    };
    use crate::live::{
        hotkeys::parse_hotkey,
        settings::LiveSettings,
        state::{LiveRoute, LiveSessionStatus, LiveSessionView},
    };

    fn physical(ctrl: bool, shift: bool, alt: bool, keys: &[&str]) -> PhysicalChordSnapshot {
        PhysicalChordSnapshot {
            ctrl,
            shift,
            alt,
            meta: false,
            keys: keys.iter().map(|key| (*key).to_string()).collect(),
        }
    }

    #[test]
    fn physical_enrollment_cannot_arm_without_native_confirmation() {
        let now = Instant::now();
        assert!(HotkeyEnrollmentEpoch::arm(false, LiveHotkeyKind::Dictation, now).is_none());
        assert!(HotkeyEnrollmentEpoch::arm(true, LiveHotkeyKind::Dictation, now).is_some());
    }

    #[test]
    fn expired_physical_enrollment_cannot_commit() {
        let now = Instant::now();
        let mut epoch = HotkeyEnrollmentEpoch::arm(true, LiveHotkeyKind::Dictation, now).unwrap();
        assert_eq!(
            epoch.observe(now, PhysicalChordSnapshot::default()),
            Ok(HotkeyEnrollmentObservation::Pending)
        );
        let error = epoch
            .observe(
                now + HOTKEY_ENROLLMENT_WINDOW,
                physical(true, true, false, &["D"]),
            )
            .unwrap_err();
        assert!(error.contains("expired"));
    }

    #[test]
    fn substituted_physical_chord_invalidates_the_epoch() {
        let now = Instant::now();
        let mut epoch = HotkeyEnrollmentEpoch::arm(true, LiveHotkeyKind::Dictation, now).unwrap();
        epoch
            .observe(now, PhysicalChordSnapshot::default())
            .unwrap();
        assert_eq!(
            epoch.observe(
                now + Duration::from_millis(1),
                physical(true, true, false, &["D"]),
            ),
            Ok(HotkeyEnrollmentObservation::Pending)
        );
        let error = epoch
            .observe(
                now + Duration::from_millis(2),
                physical(true, true, false, &["E"]),
            )
            .unwrap_err();
        assert!(error.contains("changed"));
    }

    #[test]
    fn completed_physical_enrollment_requires_release_and_cannot_be_replayed() {
        let now = Instant::now();
        let mut epoch = HotkeyEnrollmentEpoch::arm(true, LiveHotkeyKind::PasteLast, now).unwrap();
        epoch
            .observe(now, PhysicalChordSnapshot::default())
            .unwrap();
        epoch
            .observe(
                now + Duration::from_millis(1),
                physical(true, true, true, &["P"]),
            )
            .unwrap();
        assert_eq!(
            epoch.observe(
                now + Duration::from_millis(2),
                PhysicalChordSnapshot::default(),
            ),
            Ok(HotkeyEnrollmentObservation::Complete(
                "Ctrl+Shift+Alt+P".into()
            ))
        );
        let error = epoch
            .observe(
                now + Duration::from_millis(3),
                PhysicalChordSnapshot::default(),
            )
            .unwrap_err();
        assert!(error.contains("already consumed"));
    }

    #[test]
    fn ordinary_typing_is_ignored_during_the_bounded_physical_epoch() {
        let now = Instant::now();
        let mut epoch = HotkeyEnrollmentEpoch::arm(true, LiveHotkeyKind::Dictation, now).unwrap();
        epoch
            .observe(now, PhysicalChordSnapshot::default())
            .unwrap();
        assert_eq!(
            epoch.observe(
                now + Duration::from_millis(1),
                physical(false, false, false, &["D"]),
            ),
            Ok(HotkeyEnrollmentObservation::Pending)
        );
        assert_eq!(
            epoch.observe(
                now + Duration::from_millis(2),
                PhysicalChordSnapshot::default(),
            ),
            Ok(HotkeyEnrollmentObservation::Pending)
        );
    }

    #[test]
    fn only_one_native_enrollment_gate_can_be_active() {
        let gate = HotkeyEnrollmentGate::default();
        let lease = gate.try_begin().unwrap();
        assert!(gate.is_active());
        assert!(gate.try_begin().unwrap_err().contains("already active"));
        drop(lease);
        assert!(!gate.is_active());
        assert!(gate.try_begin().is_ok());
    }

    #[test]
    fn successful_replacement_clears_matching_startup_failure() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.hotkey.clear();
        view.error = Some("Live shortcut is unavailable.".into());
        view.route = LiveRoute::Blocked;
        view.status = LiveSessionStatus::Blocked;

        apply_successful_hotkey_change(
            &mut view,
            LiveHotkeyKind::Dictation,
            "Ctrl+Shift+D".into(),
            true,
        );

        assert_eq!(view.hotkey, "Ctrl+Shift+D");
        assert_eq!(view.error, None);
        assert_eq!(view.route, LiveRoute::None);
        assert_eq!(view.status, LiveSessionStatus::Idle);
    }

    #[test]
    fn successful_replacement_preserves_unrelated_block() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.error = Some("Local model is unavailable.".into());
        view.route = LiveRoute::Blocked;
        view.status = LiveSessionStatus::Blocked;

        apply_successful_hotkey_change(
            &mut view,
            LiveHotkeyKind::Dictation,
            "Ctrl+Shift+D".into(),
            false,
        );

        assert_eq!(view.error.as_deref(), Some("Local model is unavailable."));
        assert_eq!(view.route, LiveRoute::Blocked);
        assert_eq!(view.status, LiveSessionStatus::Blocked);
    }

    #[test]
    fn startup_recovery_does_not_depend_on_mutable_error_copy() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.hotkey.clear();
        view.error = Some("Selected microphone unavailable. Using default.".into());
        view.route = LiveRoute::Blocked;
        view.status = LiveSessionStatus::Blocked;

        apply_successful_hotkey_change(
            &mut view,
            LiveHotkeyKind::Dictation,
            "Ctrl+Shift+D".into(),
            true,
        );

        assert_eq!(
            view.error.as_deref(),
            Some("Selected microphone unavailable. Using default.")
        );
        assert_eq!(view.route, LiveRoute::None);
        assert_eq!(view.status, LiveSessionStatus::Idle);
    }

    #[test]
    fn replacing_both_failed_shortcuts_clears_dictation_block_last() {
        let mut view = LiveSessionView::from_settings(&LiveSettings::default());
        view.hotkey.clear();
        view.paste_hotkey.clear();
        view.error = Some(DICTATION_UNAVAILABLE_ERROR.into());
        view.route = LiveRoute::Blocked;
        view.status = LiveSessionStatus::Blocked;

        apply_successful_hotkey_change(
            &mut view,
            LiveHotkeyKind::PasteLast,
            "Ctrl+Shift+V".into(),
            true,
        );
        assert_eq!(view.error.as_deref(), Some(DICTATION_UNAVAILABLE_ERROR));
        assert_eq!(view.status, LiveSessionStatus::Blocked);

        apply_successful_hotkey_change(
            &mut view,
            LiveHotkeyKind::Dictation,
            "Ctrl+Shift+D".into(),
            true,
        );
        assert_eq!(view.error, None);
        assert_eq!(view.route, LiveRoute::None);
        assert_eq!(view.status, LiveSessionStatus::Idle);
    }

    #[test]
    fn persistence_failure_restores_previous_registration() {
        let events = RefCell::new(Vec::new());
        let previous = parse_hotkey("Ctrl+Shift+Space").unwrap();
        let next = parse_hotkey("Ctrl+Shift+V").unwrap();

        let result = replace_hotkey_registration(
            Some(previous),
            Some(next),
            |shortcut| {
                events.borrow_mut().push(format!("unregister:{shortcut:?}"));
                Ok(())
            },
            |shortcut| {
                events.borrow_mut().push(format!("register:{shortcut:?}"));
                Ok(())
            },
            || {
                events.borrow_mut().push("persist".into());
                Err("disk full".into())
            },
        );

        assert!(result.unwrap_err().contains("disk full"));
        assert_eq!(
            events
                .borrow()
                .iter()
                .map(|event| event.split(':').next().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "unregister",
                "register",
                "persist",
                "unregister",
                "register"
            ]
        );
    }

    #[test]
    fn failed_new_registration_restores_previous_registration_without_persisting() {
        let events = RefCell::new(Vec::new());
        let previous = parse_hotkey("Ctrl+Shift+Space").unwrap();
        let next = parse_hotkey("Ctrl+Shift+V").unwrap();
        let register_attempt = RefCell::new(0usize);

        let result = replace_hotkey_registration(
            Some(previous),
            Some(next),
            |shortcut| {
                events.borrow_mut().push(format!("unregister:{shortcut:?}"));
                Ok(())
            },
            |shortcut| {
                events.borrow_mut().push(format!("register:{shortcut:?}"));
                let mut attempt = register_attempt.borrow_mut();
                *attempt += 1;
                if *attempt == 1 {
                    Err("reserved".into())
                } else {
                    Ok(())
                }
            },
            || panic!("persistence must not run after registration failure"),
        );

        assert!(result.unwrap_err().contains("reserved"));
        assert_eq!(
            events
                .borrow()
                .iter()
                .map(|event| event.split(':').next().unwrap())
                .collect::<Vec<_>>(),
            vec!["unregister", "register", "register"]
        );
    }

    #[test]
    fn hotkey_mutation_is_idle_only() {
        use crate::live::state::LiveSessionStatus;

        for status in [
            LiveSessionStatus::Armed,
            LiveSessionStatus::Listening,
            LiveSessionStatus::Speaking,
            LiveSessionStatus::Settling,
            LiveSessionStatus::Saving,
        ] {
            assert_eq!(
                ensure_live_hotkey_idle(status),
                Err("Stop live before changing the shortcut.".into())
            );
        }
        for status in [LiveSessionStatus::Idle, LiveSessionStatus::Blocked] {
            assert_eq!(ensure_live_hotkey_idle(status), Ok(()));
        }
    }
}
