use super::super::*;
use crate::live::settings::LiveSettings;

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
