use super::super::*;
use crate::live::settings::LiveSettings;

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
fn a_new_block_supersedes_startup_shortcut_block_ownership() {
    let state = LiveSessionState::new(LiveSettings::default());
    state.mark_startup_shortcut_failure(false);

    state.block_with_error("Local model unavailable.");

    assert!(!state.take_startup_shortcut_failure(false));
}
