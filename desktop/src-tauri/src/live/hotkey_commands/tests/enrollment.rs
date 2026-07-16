use std::time::{Duration, Instant};

use crate::live::hotkey_commands::{
    enrollment::{
        HotkeyEnrollmentEpoch, HotkeyEnrollmentGate, HotkeyEnrollmentObservation,
        PhysicalChordSnapshot, HOTKEY_ENROLLMENT_WINDOW,
    },
    kind::LiveHotkeyKind,
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
