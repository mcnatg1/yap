use super::*;

fn fallback_test_view(status: nemotron::FallbackModelStatus) -> nemotron::FallbackModelView {
    nemotron::FallbackModelView {
        id: nemotron::MODEL_ID.into(),
        label: "Nemotron local fallback".into(),
        status,
        installed_bytes: None,
        total_bytes: None,
        progress_percent: None,
        speed_mbps: None,
        message: None,
        models_dir: "C:/models/nemotron".into(),
    }
}

#[test]
fn fallback_model_install_state_coalesces_and_cancels_idempotently() {
    let state = FallbackModelInstallState::new();
    let initial = fallback_test_view(nemotron::FallbackModelStatus::Downloading);
    let operation = state
        .begin(FallbackModelInstallPhase::Installing, initial.clone(), true)
        .unwrap();

    state.set_phase(
        operation.generation(),
        FallbackModelInstallPhase::Verifying,
        fallback_test_view(nemotron::FallbackModelStatus::Verifying),
    );
    let second = state.begin(
        FallbackModelInstallPhase::Verifying,
        fallback_test_view(nemotron::FallbackModelStatus::Verifying),
        false,
    );
    assert_eq!(
        second.unwrap_err().status,
        nemotron::FallbackModelStatus::Verifying
    );

    state.cancel_install();
    state.cancel_install();
    assert!(operation.is_cancelled());
}

#[test]
fn fallback_model_status_prefers_transient_progress_view() {
    let state = FallbackModelInstallState::new();
    let operation = state
        .begin(
            FallbackModelInstallPhase::Installing,
            fallback_test_view(nemotron::FallbackModelStatus::Downloading),
            true,
        )
        .unwrap();
    let mut progress = fallback_test_view(nemotron::FallbackModelStatus::Downloading);
    progress.progress_percent = Some(42.0);
    state.set_progress(operation.generation(), progress.clone());

    let view = status(&state);

    assert_eq!(view.progress_percent, Some(42.0));
    assert_eq!(view.status, nemotron::FallbackModelStatus::Downloading);
}

#[test]
fn fallback_model_progress_throttle_emits_first_delta_and_final() {
    let mut throttle = FallbackProgressThrottle::default();
    let base = Instant::now();
    let mut first = fallback_test_view(nemotron::FallbackModelStatus::Downloading);
    first.progress_percent = Some(10.0);
    let mut tiny_delta = first.clone();
    tiny_delta.progress_percent = Some(10.4);
    let mut final_view = first.clone();
    final_view.progress_percent = Some(100.0);
    final_view.installed_bytes = Some(10);
    final_view.total_bytes = Some(10);

    assert!(throttle.should_emit(&first, base));
    assert!(!throttle.should_emit(&tiny_delta, base + Duration::from_millis(50)));
    assert!(throttle.should_emit(
        &tiny_delta,
        base + FALLBACK_PROGRESS_MIN_INTERVAL + Duration::from_millis(1)
    ));
    assert!(throttle.should_emit(&final_view, base + Duration::from_millis(75)));
}

#[test]
fn fallback_model_sanitize_drops_non_finite_progress_values() {
    let mut view = fallback_test_view(nemotron::FallbackModelStatus::Downloading);
    view.progress_percent = Some(f32::NAN);
    view.speed_mbps = Some(f32::INFINITY);

    let sanitized = sanitize_fallback_model_view(view);

    assert_eq!(sanitized.progress_percent, None);
    assert_eq!(sanitized.speed_mbps, None);
}

#[test]
fn cancel_marks_install_active_during_verifying_phase() {
    let state = FallbackModelInstallState::new();
    let operation = state
        .begin(
            FallbackModelInstallPhase::Installing,
            fallback_test_view(nemotron::FallbackModelStatus::Downloading),
            true,
        )
        .unwrap();
    state.set_phase(
        operation.generation(),
        FallbackModelInstallPhase::Verifying,
        fallback_test_view(nemotron::FallbackModelStatus::Verifying),
    );

    let _ = cancel_install(&state).unwrap();

    assert!(operation.is_cancelled());
}

#[test]
fn model_mutation_rejects_active_install_or_verify() {
    let state = FallbackModelInstallState::new();
    state
        .begin(
            FallbackModelInstallPhase::Installing,
            fallback_test_view(nemotron::FallbackModelStatus::Downloading),
            true,
        )
        .unwrap();

    let error = ensure_model_mutation_idle(&state).unwrap_err();

    assert_eq!(error.code, SttError::Busy.code());
}

#[test]
fn stale_generation_cannot_cancel_or_finish_a_new_operation() {
    let state = FallbackModelInstallState::new();
    let first = state
        .begin(
            FallbackModelInstallPhase::Installing,
            fallback_test_view(nemotron::FallbackModelStatus::Downloading),
            true,
        )
        .unwrap();
    let first_generation = first.generation();
    assert!(state.cancel_generation(first_generation));
    assert!(first.is_cancelled());
    assert!(state
        .begin(
            FallbackModelInstallPhase::Installing,
            fallback_test_view(nemotron::FallbackModelStatus::Downloading),
            true,
        )
        .is_err());

    state.claim_terminal_for_test(first_generation).unwrap();
    state
        .finish_generation_for_test(
            first_generation,
            fallback_test_view(nemotron::FallbackModelStatus::Missing),
            None,
        )
        .unwrap();
    let second = state
        .begin(
            FallbackModelInstallPhase::Installing,
            fallback_test_view(nemotron::FallbackModelStatus::Downloading),
            true,
        )
        .unwrap();

    assert!(second.generation() > first_generation);
    assert!(!state.cancel_generation(first_generation));
    assert!(!second.is_cancelled());
    assert!(state
        .finish_generation_for_test(
            first_generation,
            fallback_test_view(nemotron::FallbackModelStatus::Missing),
            None,
        )
        .is_err());
    assert_eq!(state.snapshot().generation, Some(second.generation()));
}

#[test]
fn finalization_persists_cleanup_failure_and_releases_once() {
    let state = FallbackModelInstallState::new();
    let operation = state
        .begin(
            FallbackModelInstallPhase::Installing,
            fallback_test_view(nemotron::FallbackModelStatus::Downloading),
            true,
        )
        .unwrap();
    let cleanup_error = SttCommandError {
        code: "MODEL_TEMP_CLEANUP_FAILED".into(),
        message: "could not remove operation temp".into(),
    };

    let error = finalize_operation(
        &state,
        &operation,
        fallback_test_view(nemotron::FallbackModelStatus::Error),
        None,
        || Err(cleanup_error.clone()),
        |_| Ok(()),
    )
    .unwrap_err();

    assert_eq!(error.code, cleanup_error.code);
    assert_eq!(state.snapshot().error.unwrap().code, cleanup_error.code);
    assert!(state.snapshot().phase.is_none());
    assert!(state
        .finish_generation_for_test(
            operation.generation(),
            fallback_test_view(nemotron::FallbackModelStatus::Missing),
            None,
        )
        .is_err());
}

#[test]
fn finalization_persists_publication_failure_and_releases() {
    let state = FallbackModelInstallState::new();
    let operation = state
        .begin(
            FallbackModelInstallPhase::Installing,
            fallback_test_view(nemotron::FallbackModelStatus::Downloading),
            true,
        )
        .unwrap();
    let publication_error = SttCommandError {
        code: "MODEL_STATUS_PUBLISH_FAILED".into(),
        message: "status event failed".into(),
    };

    let error = finalize_operation(
        &state,
        &operation,
        fallback_test_view(nemotron::FallbackModelStatus::Error),
        None,
        || Ok(()),
        |_| Err(publication_error.clone()),
    )
    .unwrap_err();

    assert_eq!(error.code, publication_error.code);
    assert_eq!(state.snapshot().error.unwrap().code, publication_error.code);
    assert!(state.snapshot().phase.is_none());
}

#[test]
fn finalization_converts_panic_to_persisted_failure_and_releases() {
    let state = FallbackModelInstallState::new();
    let operation = state
        .begin(
            FallbackModelInstallPhase::Installing,
            fallback_test_view(nemotron::FallbackModelStatus::Downloading),
            true,
        )
        .unwrap();

    let error = finalize_operation(
        &state,
        &operation,
        fallback_test_view(nemotron::FallbackModelStatus::Error),
        Some(SttCommandError::from(SttError::SidecarCrash)),
        || Ok(()),
        |_| -> Result<(), SttCommandError> { panic!("forced publication panic") },
    )
    .unwrap_err();

    assert_eq!(error.code, "MODEL_FINALIZATION_PANIC");
    assert_eq!(state.snapshot().error.unwrap().code, error.code);
    assert!(state.snapshot().phase.is_none());
    assert!(state
        .begin(
            FallbackModelInstallPhase::Installing,
            fallback_test_view(nemotron::FallbackModelStatus::Downloading),
            true,
        )
        .is_ok());
}

#[test]
fn duplicate_terminal_caller_cannot_run_cleanup_or_publication() {
    use std::cell::Cell;

    let state = FallbackModelInstallState::new();
    let operation = state
        .begin(
            FallbackModelInstallPhase::Installing,
            fallback_test_view(nemotron::FallbackModelStatus::Downloading),
            true,
        )
        .unwrap();
    finalize_operation(
        &state,
        &operation,
        fallback_test_view(nemotron::FallbackModelStatus::Ready),
        None,
        || Ok(()),
        |_| Ok(()),
    )
    .unwrap();

    let cleanup_calls = Cell::new(0);
    let publication_calls = Cell::new(0);
    let error = finalize_operation(
        &state,
        &operation,
        fallback_test_view(nemotron::FallbackModelStatus::Error),
        None,
        || {
            cleanup_calls.set(cleanup_calls.get() + 1);
            Ok(())
        },
        |_| {
            publication_calls.set(publication_calls.get() + 1);
            Ok(())
        },
    )
    .unwrap_err();

    assert_eq!(error.code, "MODEL_OPERATION_STALE");
    assert_eq!(cleanup_calls.get(), 0);
    assert_eq!(publication_calls.get(), 0);
}
