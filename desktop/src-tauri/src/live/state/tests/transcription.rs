use super::super::*;
use crate::live::settings::LiveSettings;

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
