use std::time::{Duration, Instant};

use super::super::*;
use crate::live::state::LiveCaptureMode;

#[test]
fn shortcut_double_tap_starts_hands_free_and_release_is_ignored() {
    let mut shortcut = LiveShortcutInteraction::default();
    let now = Instant::now();

    assert_eq!(
        shortcut.pressed(now, None),
        LiveShortcutAction::ScheduleHold(1)
    );
    assert_eq!(
        shortcut.released(now + Duration::from_millis(40), None),
        LiveShortcutAction::None
    );
    assert_eq!(
        shortcut.pressed(now + Duration::from_millis(120), None),
        LiveShortcutAction::Start(LiveCaptureMode::Toggle)
    );
    assert_eq!(
        shortcut.released(now + Duration::from_millis(150), None),
        LiveShortcutAction::None
    );
    assert_eq!(
        shortcut.pressed(now + Duration::from_millis(240), None),
        LiveShortcutAction::Stop
    );
    assert_eq!(
        shortcut.released(now + Duration::from_millis(260), None),
        LiveShortcutAction::None
    );
}

#[test]
fn shortcut_reset_clears_stale_tap_state() {
    let mut shortcut = LiveShortcutInteraction::default();
    let now = Instant::now();

    assert_eq!(
        shortcut.pressed(now, None),
        LiveShortcutAction::ScheduleHold(1)
    );
    assert_eq!(
        shortcut.released(now + Duration::from_millis(40), None),
        LiveShortcutAction::None
    );
    shortcut.reset();

    assert_eq!(
        shortcut.pressed(now + Duration::from_millis(120), None),
        LiveShortcutAction::ScheduleHold(2)
    );
}

#[test]
fn shortcut_ignores_repeated_pressed_events_until_release() {
    let mut shortcut = LiveShortcutInteraction::default();
    let now = Instant::now();

    assert_eq!(
        shortcut.pressed(now, None),
        LiveShortcutAction::ScheduleHold(1)
    );
    assert_eq!(
        shortcut.pressed(now + Duration::from_millis(20), None),
        LiveShortcutAction::None
    );
    assert_eq!(
        shortcut.hold_elapsed(1, now + Duration::from_millis(SHORTCUT_HOLD_MS + 1), None,),
        LiveShortcutAction::Start(LiveCaptureMode::PushToTalk)
    );
}

#[test]
fn shortcut_release_during_push_to_talk_start_stops_without_waiting_for_projection() {
    let mut shortcut = LiveShortcutInteraction::default();
    let now = Instant::now();

    assert_eq!(
        shortcut.pressed(now, None),
        LiveShortcutAction::ScheduleHold(1)
    );
    assert_eq!(
        shortcut.hold_elapsed(1, now + Duration::from_millis(SHORTCUT_HOLD_MS + 1), None,),
        LiveShortcutAction::Start(LiveCaptureMode::PushToTalk)
    );
    assert_eq!(
        shortcut.released(now + Duration::from_millis(180), None),
        LiveShortcutAction::Stop
    );
}

#[test]
fn shortcut_hold_starts_push_to_talk_and_release_stops() {
    let mut shortcut = LiveShortcutInteraction::default();
    let now = Instant::now();

    assert_eq!(
        shortcut.pressed(now, None),
        LiveShortcutAction::ScheduleHold(1)
    );
    assert_eq!(
        shortcut.hold_elapsed(1, now + Duration::from_millis(SHORTCUT_HOLD_MS + 1), None,),
        LiveShortcutAction::Start(LiveCaptureMode::PushToTalk)
    );
    assert_eq!(
        shortcut.released(
            now + Duration::from_millis(260),
            Some(LiveCaptureMode::PushToTalk),
        ),
        LiveShortcutAction::Stop
    );
}

#[test]
fn shortcut_single_tap_does_not_start_recording() {
    let mut shortcut = LiveShortcutInteraction::default();
    let now = Instant::now();

    assert_eq!(
        shortcut.pressed(now, None),
        LiveShortcutAction::ScheduleHold(1)
    );
    assert_eq!(
        shortcut.released(now + Duration::from_millis(40), None),
        LiveShortcutAction::None
    );
    assert_eq!(
        shortcut.hold_elapsed(1, now + Duration::from_millis(SHORTCUT_HOLD_MS + 1), None,),
        LiveShortcutAction::None
    );
}

#[test]
fn projected_session_end_clears_owned_mode_after_start_is_acknowledged() {
    let mut shortcut = LiveShortcutInteraction::default();
    let now = Instant::now();

    assert_eq!(
        shortcut.pressed(now, None),
        LiveShortcutAction::ScheduleHold(1)
    );
    assert_eq!(
        shortcut.released(now + Duration::from_millis(30), None),
        LiveShortcutAction::None
    );
    assert_eq!(
        shortcut.pressed(now + Duration::from_millis(90), None),
        LiveShortcutAction::Start(LiveCaptureMode::Toggle)
    );
    assert_eq!(
        shortcut.released(now + Duration::from_millis(120), None),
        LiveShortcutAction::None
    );
    shortcut.finish_start(Some(LiveCaptureMode::Toggle));

    assert_eq!(
        shortcut.pressed(now + Duration::from_millis(500), None),
        LiveShortcutAction::ScheduleHold(2)
    );
}

#[test]
fn failed_toggle_start_clears_the_toggle_stop_latch() {
    let mut shortcut = LiveShortcutInteraction::default();
    let now = Instant::now();

    assert_eq!(
        shortcut.pressed(now, None),
        LiveShortcutAction::ScheduleHold(1)
    );
    shortcut.released(now + Duration::from_millis(25), None);
    assert_eq!(
        shortcut.pressed(now + Duration::from_millis(75), None),
        LiveShortcutAction::Start(LiveCaptureMode::Toggle)
    );
    shortcut.released(now + Duration::from_millis(100), None);
    shortcut.finish_start(None);

    assert_eq!(
        shortcut.pressed(now + Duration::from_millis(450), None),
        LiveShortcutAction::ScheduleHold(2)
    );
}

#[test]
fn projected_modes_stop_with_their_own_contract_without_cross_mode_taps() {
    let mut push_to_talk = LiveShortcutInteraction::default();
    let now = Instant::now();

    assert_eq!(
        push_to_talk.pressed(now, Some(LiveCaptureMode::PushToTalk)),
        LiveShortcutAction::None
    );
    assert_eq!(
        push_to_talk.released(
            now + Duration::from_millis(20),
            Some(LiveCaptureMode::PushToTalk),
        ),
        LiveShortcutAction::Stop
    );
    assert_eq!(
        push_to_talk.released(now + Duration::from_millis(40), None),
        LiveShortcutAction::None
    );

    let mut toggle = LiveShortcutInteraction::default();
    assert_eq!(
        toggle.pressed(now, Some(LiveCaptureMode::Toggle)),
        LiveShortcutAction::Stop
    );
    assert_eq!(
        toggle.released(now + Duration::from_millis(20), None),
        LiveShortcutAction::None
    );
}

#[test]
fn delayed_hold_timer_cannot_convert_a_double_tap_into_push_to_talk() {
    let mut shortcut = LiveShortcutInteraction::default();
    let now = Instant::now();

    assert_eq!(
        shortcut.pressed(now, None),
        LiveShortcutAction::ScheduleHold(1)
    );
    shortcut.released(now + Duration::from_millis(30), None);
    assert_eq!(
        shortcut.pressed(now + Duration::from_millis(80), None),
        LiveShortcutAction::Start(LiveCaptureMode::Toggle)
    );
    assert_eq!(
        shortcut.hold_elapsed(1, now + Duration::from_millis(SHORTCUT_HOLD_MS + 40), None,),
        LiveShortcutAction::None
    );
}
