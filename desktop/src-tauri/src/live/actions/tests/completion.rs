use std::cell::RefCell;

use super::super::completion::{
    apply_injection_result, run_completion_effects_with, run_completion_effects_with_mode,
    CompletionMode, INJECTION_COPIED_ERROR,
};
use crate::live::{
    injection::InjectionOutcome,
    settings::LiveSettings,
    state::{LiveSessionState, LiveSessionView},
};

#[test]
fn successful_retry_clears_only_injection_feedback() {
    let state = LiveSessionState::new(LiveSettings::default());
    state.update(|view| view.error = Some(INJECTION_COPIED_ERROR.into()));

    let recovered = apply_injection_result(&state, Ok(Some(InjectionOutcome::Injected)));
    assert_eq!(recovered.error, None);

    state.update(|view| view.error = Some("Live transcription stopped unexpectedly.".into()));
    let unrelated = apply_injection_result(&state, Ok(Some(InjectionOutcome::Injected)));
    assert_eq!(
        unrelated.error.as_deref(),
        Some("Live transcription stopped unexpectedly.")
    );

    state.update(|view| {
        view.error = Some(format!(
            "Live transcription stopped unexpectedly. {INJECTION_COPIED_ERROR}"
        ));
    });
    let combined = apply_injection_result(&state, Ok(Some(InjectionOutcome::Injected)));
    assert_eq!(
        combined.error.as_deref(),
        Some("Live transcription stopped unexpectedly.")
    );

    state.update(|view| view.error = Some("Couldn't save this recording to Home.".into()));
    let copied = apply_injection_result(
        &state,
        Ok(Some(InjectionOutcome::CopiedOnly("focus changed".into()))),
    );
    assert_eq!(
        copied.error.as_deref(),
        Some(
            "Couldn't save this recording to Home. Couldn't insert text here. Transcript copied; press Ctrl+V."
        )
    );
    let failed = apply_injection_result(&state, Err("clipboard busy".into()));
    assert_eq!(
        failed.error.as_deref(),
        Some("Couldn't save this recording to Home. Couldn't insert or copy this transcript.")
    );
}

#[test]
fn completed_transcript_is_sent_to_the_injection_port() {
    let mut view = LiveSessionView::from_settings(&LiveSettings::default());
    view.final_text = Some("  Thank you.  ".into());
    let injected = RefCell::new(Vec::new());

    let effects = run_completion_effects_with(
        &view,
        |_| {},
        |text| {
            injected.borrow_mut().push(text.to_string());
            Ok(())
        },
        || Ok(()),
    );

    assert_eq!(effects.injection, Ok(Some(())));
    assert_eq!(effects.save, Ok(()));
    assert_eq!(injected.into_inner(), vec!["Thank you.".to_string()]);
}

#[test]
fn completion_effects_remember_and_inject_before_saving() {
    let mut view = LiveSessionView::from_settings(&LiveSettings::default());
    view.final_text = Some("Finished words".into());
    let events = RefCell::new(Vec::<String>::new());

    let effects = run_completion_effects_with(
        &view,
        |text| events.borrow_mut().push(format!("remember:{text}")),
        |text| {
            events.borrow_mut().push(format!("inject:{text}"));
            Ok(())
        },
        || {
            events.borrow_mut().push("save".into());
            Ok(())
        },
    );

    assert_eq!(effects.injection, Ok(Some(())));
    assert_eq!(effects.save, Ok(()));
    assert_eq!(
        events.into_inner(),
        vec!["remember:Finished words", "inject:Finished words", "save"]
    );
}

#[test]
fn quit_completion_remembers_and_saves_without_injecting() {
    let mut view = LiveSessionView::from_settings(&LiveSettings::default());
    view.final_text = Some("Finished words".into());
    let events = RefCell::new(Vec::<String>::new());

    let effects = run_completion_effects_with_mode(
        &view,
        CompletionMode::Quit,
        |text| events.borrow_mut().push(format!("remember:{text}")),
        |_| -> Result<(), String> {
            events.borrow_mut().push("inject".into());
            Ok(())
        },
        || {
            events.borrow_mut().push("save".into());
            Ok(())
        },
    );

    assert_eq!(effects.injection, Ok(None));
    assert_eq!(effects.save, Ok(()));
    assert_eq!(events.into_inner(), vec!["remember:Finished words", "save"]);
}

#[test]
fn only_the_saving_claim_holder_runs_completion_effects() {
    let state = LiveSessionState::new(LiveSettings::default());
    state
        .try_begin_local_start(crate::live::state::LiveCaptureMode::PushToTalk, None, None)
        .unwrap();
    state.try_begin_listening_from_armed().unwrap();
    state.update_final("Finished words");
    let first = state.try_begin_saving(true).unwrap();
    assert!(state.try_begin_saving(true).is_none());
    let effects = RefCell::new(Vec::new());

    let _ = run_completion_effects_with(
        &first,
        |_| effects.borrow_mut().push("remember"),
        |_| {
            effects.borrow_mut().push("inject");
            Ok(())
        },
        || {
            effects.borrow_mut().push("save");
            Ok(())
        },
    );

    assert_eq!(effects.into_inner(), vec!["remember", "inject", "save"]);
}

#[test]
fn injection_failure_does_not_skip_save() {
    let mut view = LiveSessionView::from_settings(&LiveSettings::default());
    view.final_text = Some("Finished words".into());
    let events = RefCell::new(Vec::<String>::new());

    let effects = run_completion_effects_with(
        &view,
        |_| {},
        |_| -> Result<(), String> {
            events.borrow_mut().push("inject".into());
            Err("blocked".into())
        },
        || {
            events.borrow_mut().push("save".into());
            Ok(())
        },
    );

    assert_eq!(effects.injection, Err("blocked".into()));
    assert_eq!(effects.save, Ok(()));
    assert_eq!(events.into_inner(), vec!["inject", "save"]);
}

#[test]
fn empty_transcript_does_not_synthesize_input() {
    let view = LiveSessionView::from_settings(&LiveSettings::default());

    let effects = run_completion_effects_with(
        &view,
        |_| panic!("empty sessions must not update paste-last"),
        |_| -> Result<(), String> { panic!("empty sessions must not invoke the injection port") },
        || Ok(()),
    );

    assert_eq!(effects.injection, Ok(None));
    assert_eq!(effects.save, Ok(()));
}

#[test]
fn partial_transcript_does_not_synthesize_input() {
    let mut view = LiveSessionView::from_settings(&LiveSettings::default());
    view.partial_text = Some("not finalized".into());

    let effects = run_completion_effects_with(
        &view,
        |_| panic!("partial sessions must not update paste-last"),
        |_| -> Result<(), String> { panic!("partial sessions must not invoke the injection port") },
        || Ok(()),
    );

    assert_eq!(effects.injection, Ok(None));
    assert_eq!(effects.save, Ok(()));
}
