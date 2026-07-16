use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use crate::live;

use super::kind::LiveHotkeyKind;

pub(super) const HOTKEY_ENROLLMENT_WINDOW: Duration = Duration::from_secs(15);
const HOTKEY_POLL_INTERVAL: Duration = Duration::from_millis(8);

#[derive(Clone, Default)]
pub(crate) struct HotkeyEnrollmentGate {
    active: Arc<AtomicBool>,
}

impl HotkeyEnrollmentGate {
    pub(super) fn try_begin(&self) -> Result<HotkeyEnrollmentLease, String> {
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
pub(super) struct HotkeyEnrollmentLease {
    active: Arc<AtomicBool>,
}

impl Drop for HotkeyEnrollmentLease {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PhysicalChordSnapshot {
    pub(super) ctrl: bool,
    pub(super) shift: bool,
    pub(super) alt: bool,
    pub(super) meta: bool,
    pub(super) keys: Vec<String>,
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
pub(super) enum HotkeyEnrollmentObservation {
    Pending,
    Complete(String),
    Cancelled,
}

pub(super) struct HotkeyEnrollmentEpoch {
    kind: LiveHotkeyKind,
    expires_at: Instant,
    phase: HotkeyEnrollmentPhase,
}

impl HotkeyEnrollmentEpoch {
    pub(super) fn arm(confirmed: bool, kind: LiveHotkeyKind, now: Instant) -> Option<Self> {
        confirmed.then_some(Self {
            kind,
            expires_at: now + HOTKEY_ENROLLMENT_WINDOW,
            phase: HotkeyEnrollmentPhase::AwaitingNeutral,
        })
    }

    pub(super) fn observe(
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
pub(super) fn capture_physical_hotkey(
    mut epoch: HotkeyEnrollmentEpoch,
) -> Result<Option<String>, String> {
    loop {
        match epoch.observe(Instant::now(), physical_chord_snapshot())? {
            HotkeyEnrollmentObservation::Pending => std::thread::sleep(HOTKEY_POLL_INTERVAL),
            HotkeyEnrollmentObservation::Complete(hotkey) => return Ok(Some(hotkey)),
            HotkeyEnrollmentObservation::Cancelled => return Ok(None),
        }
    }
}

#[cfg(not(windows))]
pub(super) fn capture_physical_hotkey(_: HotkeyEnrollmentEpoch) -> Result<Option<String>, String> {
    Err("Physical shortcut recording is currently supported only on Windows.".into())
}
