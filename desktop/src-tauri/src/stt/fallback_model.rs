use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tauri::{AppHandle, Emitter};

use crate::stt::{dispatch::SttCommandError, error::SttError, nemotron, settings};

const FALLBACK_MODEL_STATUS_EVENT: &str = "fallback-model-status";
const FALLBACK_MODEL_PROGRESS_EVENT: &str = "fallback-model-progress";
const FALLBACK_PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(200);
const FALLBACK_PROGRESS_MIN_PERCENT_DELTA: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FallbackModelInstallPhase {
    Installing,
    Verifying,
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
struct FallbackModelInstallSnapshot {
    generation: Option<u64>,
    phase: Option<FallbackModelInstallPhase>,
    error: Option<SttCommandError>,
}

#[derive(Debug, Clone)]
struct ActiveFallbackModelOperation {
    phase: FallbackModelInstallPhase,
    cancellable: bool,
    terminal_claimed: bool,
    operation: crate::stt::model::DownloadOperation,
}

#[derive(Debug, Default)]
struct FallbackModelInstallInner {
    last_generation: u64,
    active: Option<ActiveFallbackModelOperation>,
    view: Option<nemotron::FallbackModelView>,
    progress: Option<nemotron::FallbackModelView>,
    error: Option<SttCommandError>,
}

#[derive(Clone, Default)]
pub struct FallbackModelInstallState {
    inner: Arc<Mutex<FallbackModelInstallInner>>,
}

impl FallbackModelInstallState {
    pub fn new() -> Self {
        Self::default()
    }

    fn begin(
        &self,
        phase: FallbackModelInstallPhase,
        view: nemotron::FallbackModelView,
        cancellable: bool,
    ) -> Result<crate::stt::model::DownloadOperation, Box<nemotron::FallbackModelView>> {
        let mut inner = self.inner.lock().expect("fallback model state poisoned");
        if inner.active.is_some() {
            return Err(Box::new(
                inner
                    .progress
                    .clone()
                    .or_else(|| inner.view.clone())
                    .unwrap_or(view),
            ));
        }
        let generation = inner
            .last_generation
            .checked_add(1)
            .expect("fallback model operation generation exhausted");
        let operation = crate::stt::model::DownloadOperation::new(generation);
        inner.last_generation = generation;
        inner.active = Some(ActiveFallbackModelOperation {
            phase,
            cancellable,
            terminal_claimed: false,
            operation: operation.clone(),
        });
        inner.view = Some(view);
        inner.progress = None;
        inner.error = None;
        Ok(operation)
    }

    #[cfg(test)]
    fn snapshot(&self) -> FallbackModelInstallSnapshot {
        let inner = self.inner.lock().expect("fallback model state poisoned");
        FallbackModelInstallSnapshot {
            generation: inner
                .active
                .as_ref()
                .map(|active| active.operation.generation()),
            phase: inner.active.as_ref().map(|active| active.phase),
            error: inner.error.clone(),
        }
    }

    fn current_view(&self) -> Option<nemotron::FallbackModelView> {
        let inner = self.inner.lock().expect("fallback model state poisoned");
        inner.progress.clone().or_else(|| inner.view.clone())
    }

    fn is_active(&self) -> bool {
        self.inner
            .lock()
            .expect("fallback model state poisoned")
            .active
            .is_some()
    }

    fn set_phase(
        &self,
        generation: u64,
        phase: FallbackModelInstallPhase,
        view: nemotron::FallbackModelView,
    ) -> bool {
        let mut inner = self.inner.lock().expect("fallback model state poisoned");
        let Some(active) = inner.active.as_mut() else {
            return false;
        };
        if active.operation.generation() != generation || active.terminal_claimed {
            return false;
        }
        active.phase = phase;
        inner.view = Some(view);
        inner.progress = None;
        inner.error = None;
        true
    }

    fn set_progress(&self, generation: u64, view: nemotron::FallbackModelView) -> bool {
        let mut inner = self.inner.lock().expect("fallback model state poisoned");
        if inner.active.as_ref().is_none_or(|active| {
            active.operation.generation() != generation || active.terminal_claimed
        }) {
            return false;
        }
        inner.progress = Some(view.clone());
        inner.view = Some(view);
        true
    }

    #[cfg(test)]
    fn cancel_generation(&self, generation: u64) -> bool {
        let inner = self.inner.lock().expect("fallback model state poisoned");
        let Some(active) = inner.active.as_ref() else {
            return false;
        };
        if active.operation.generation() != generation
            || !active.cancellable
            || active.terminal_claimed
        {
            return false;
        }
        active.operation.cancel();
        true
    }

    fn claim_terminal(&self, generation: u64) -> Result<(), SttCommandError> {
        let mut inner = self.inner.lock().expect("fallback model state poisoned");
        let Some(active) = inner.active.as_mut() else {
            return Err(stale_model_operation_error());
        };
        if active.operation.generation() != generation || active.terminal_claimed {
            return Err(stale_model_operation_error());
        }
        active.terminal_claimed = true;
        Ok(())
    }

    fn cancel_install(&self) -> bool {
        let inner = self.inner.lock().expect("fallback model state poisoned");
        let Some(active) = inner.active.as_ref() else {
            return false;
        };
        if !active.cancellable || active.terminal_claimed {
            return false;
        }
        active.operation.cancel();
        true
    }

    fn finish_generation(
        &self,
        generation: u64,
        view: nemotron::FallbackModelView,
        error: Option<SttCommandError>,
    ) -> Result<(), SttCommandError> {
        let mut inner = self.inner.lock().expect("fallback model state poisoned");
        if inner.active.as_ref().is_none_or(|active| {
            active.operation.generation() != generation || !active.terminal_claimed
        }) {
            return Err(stale_model_operation_error());
        }
        inner.active = None;
        inner.view = Some(view);
        inner.progress = None;
        inner.error = error;
        Ok(())
    }
}

#[derive(Debug, Default)]
struct FallbackProgressThrottle {
    emitted_once: bool,
    last_emit_at: Option<Instant>,
    last_progress_percent: Option<f32>,
}

impl FallbackProgressThrottle {
    fn should_emit(&mut self, view: &nemotron::FallbackModelView, now: Instant) -> bool {
        let progress_percent = view.progress_percent;
        let should_emit = !self.emitted_once
            || is_final_fallback_progress(view)
            || view.status != nemotron::FallbackModelStatus::Downloading
            || self
                .last_emit_at
                .is_none_or(|last| now.duration_since(last) >= FALLBACK_PROGRESS_MIN_INTERVAL)
            || percent_changed(
                self.last_progress_percent,
                progress_percent,
                FALLBACK_PROGRESS_MIN_PERCENT_DELTA,
            );

        if should_emit {
            self.emitted_once = true;
            self.last_emit_at = Some(now);
            self.last_progress_percent = progress_percent;
        }

        should_emit
    }
}

struct FallbackProgressEmitter {
    app: AppHandle,
    install_state: FallbackModelInstallState,
    operation: crate::stt::model::DownloadOperation,
    throttle: FallbackProgressThrottle,
    publication_failure: Option<SttCommandError>,
}

impl FallbackProgressEmitter {
    fn new(
        app: AppHandle,
        install_state: FallbackModelInstallState,
        operation: crate::stt::model::DownloadOperation,
    ) -> Self {
        Self {
            app,
            install_state,
            operation,
            throttle: FallbackProgressThrottle::default(),
            publication_failure: None,
        }
    }

    fn publish(&mut self, view: nemotron::FallbackModelView) {
        if self.publication_failure.is_some() {
            return;
        }
        let view = sanitize_fallback_model_view(view);
        if !self
            .install_state
            .set_progress(self.operation.generation(), view.clone())
        {
            self.fail_publication(model_operation_error(
                "MODEL_OPERATION_STALE",
                "Progress arrived for an inactive model operation.",
            ));
            return;
        }
        if self.throttle.should_emit(&view, Instant::now()) {
            if let Err(error) = self.app.emit(FALLBACK_MODEL_PROGRESS_EVENT, &view) {
                self.fail_publication(model_operation_error(
                    "MODEL_PROGRESS_PUBLISH_FAILED",
                    &format!("Could not publish model progress: {error}"),
                ));
            }
        }
    }

    fn fail_publication(&mut self, error: SttCommandError) {
        self.operation.cancel();
        self.publication_failure = Some(error);
    }

    fn take_failure(&mut self) -> Option<SttCommandError> {
        self.publication_failure.take()
    }
}

pub fn status(install_state: &FallbackModelInstallState) -> nemotron::FallbackModelView {
    install_state
        .current_view()
        .unwrap_or_else(persisted_fallback_model_view)
}

pub async fn install(
    app: AppHandle,
    install_state: FallbackModelInstallState,
    force: bool,
) -> Result<nemotron::FallbackModelView, SttCommandError> {
    let initial_view = fallback_model_phase_view(
        true,
        nemotron::FallbackModelStatus::Downloading,
        Some("Preparing download".into()),
    );
    let operation = match install_state.begin(
        FallbackModelInstallPhase::Installing,
        initial_view.clone(),
        true,
    ) {
        Ok(operation) => operation,
        Err(active) => return Ok(*active),
    };
    if let Err(error) = emit_fallback_progress(&app, &install_state, &operation, initial_view) {
        return finalize_operation(
            &install_state,
            &operation,
            fallback_model_terminal_command_view(&error),
            Some(error),
            || operation_cleanup_result(&operation),
            |_| Ok(()),
        );
    }

    let progress_app = app.clone();
    let progress_state = install_state.clone();
    let worker_operation = operation.clone();
    let joined = tauri::async_runtime::spawn_blocking(move || {
        let mut progress =
            FallbackProgressEmitter::new(progress_app, progress_state, worker_operation.clone());
        settings::set_local_fallback_enabled(true).map_err(SttCommandError::from)?;
        let model_result = nemotron::ensure_model_with_progress(
            force,
            |view| progress.publish(view),
            &worker_operation,
        )
        .map_err(SttCommandError::from);
        if let Some(error) = progress.take_failure() {
            return Err(error);
        }
        model_result?;
        if worker_operation.is_cancelled() {
            return Err(SttCommandError::from(SttError::ModelInstallCancelled));
        }
        Ok(nemotron::model_status(true))
    })
    .await;

    let result = joined.unwrap_or_else(|_| Err(SttCommandError::from(SttError::SidecarCrash)));
    let (terminal_view, terminal_error) = match result {
        Ok(view) => (sanitize_fallback_model_view(view), None),
        Err(error) => (fallback_model_terminal_command_view(&error), Some(error)),
    };
    finalize_operation(
        &install_state,
        &operation,
        terminal_view,
        terminal_error,
        || operation_cleanup_result(&operation),
        |view| emit_terminal_status(&app, view),
    )
}

pub fn cancel_install(
    install_state: &FallbackModelInstallState,
) -> Result<nemotron::FallbackModelView, SttCommandError> {
    install_state.cancel_install();
    Ok(status(install_state))
}

pub async fn verify(
    app: AppHandle,
    install_state: FallbackModelInstallState,
) -> Result<nemotron::FallbackModelView, SttCommandError> {
    let initial_view = fallback_model_phase_view(
        settings::local_fallback_enabled(),
        nemotron::FallbackModelStatus::Verifying,
        Some("Verifying files".into()),
    );
    let operation = match install_state.begin(
        FallbackModelInstallPhase::Verifying,
        initial_view.clone(),
        false,
    ) {
        Ok(operation) => operation,
        Err(active) => return Ok(*active),
    };
    if let Err(error) = emit_fallback_status(
        &app,
        &install_state,
        &operation,
        FallbackModelInstallPhase::Verifying,
        initial_view,
    ) {
        return finalize_operation(
            &install_state,
            &operation,
            fallback_model_terminal_command_view(&error),
            Some(error),
            || operation_cleanup_result(&operation),
            |_| Ok(()),
        );
    }

    let progress_app = app.clone();
    let progress_state = install_state.clone();
    let worker_operation = operation.clone();
    let joined = tauri::async_runtime::spawn_blocking(move || {
        let mut progress =
            FallbackProgressEmitter::new(progress_app, progress_state, worker_operation.clone());
        let view = sanitize_fallback_model_view(nemotron::verify_model_with_progress(
            settings::local_fallback_enabled(),
            |view| progress.publish(view),
            || worker_operation.is_cancelled(),
        ));
        if let Some(error) = progress.take_failure() {
            Err(error)
        } else {
            Ok(view)
        }
    })
    .await;

    let result = joined.unwrap_or_else(|_| Err(SttCommandError::from(SttError::SidecarCrash)));
    let (terminal_view, terminal_error) = match result {
        Ok(view) => (view, None),
        Err(error) => (fallback_model_terminal_command_view(&error), Some(error)),
    };
    finalize_operation(
        &install_state,
        &operation,
        terminal_view,
        terminal_error,
        || operation_cleanup_result(&operation),
        |view| emit_terminal_status(&app, view),
    )
}

pub fn remove(
    install_state: &FallbackModelInstallState,
) -> Result<nemotron::FallbackModelView, SttCommandError> {
    ensure_model_mutation_idle(install_state)?;
    nemotron::remove_model().map_err(SttCommandError::from)?;
    settings::set_local_fallback_enabled(false)?;
    Ok(nemotron::model_status(false))
}

pub fn set_enabled(
    install_state: &FallbackModelInstallState,
    enabled: bool,
) -> Result<nemotron::FallbackModelView, SttCommandError> {
    ensure_model_mutation_idle(install_state)?;
    settings::set_local_fallback_enabled(enabled)?;
    Ok(nemotron::model_status(enabled))
}

pub fn open_folder() -> Result<(), SttCommandError> {
    let root = nemotron::root_dir();
    std::fs::create_dir_all(&root)
        .map_err(|error| fallback_model_command_error("MODEL_FOLDER_OPEN_FAILED", &error))?;
    tauri_plugin_opener::open_path(&root, None::<&str>)
        .map_err(|error| fallback_model_command_error("MODEL_FOLDER_OPEN_FAILED", &error))
}

fn persisted_fallback_model_view() -> nemotron::FallbackModelView {
    nemotron::model_status(settings::local_fallback_enabled())
}

fn fallback_model_phase_view(
    enabled: bool,
    status: nemotron::FallbackModelStatus,
    message: Option<String>,
) -> nemotron::FallbackModelView {
    let mut view = nemotron::model_status(enabled);
    view.status = status;
    view.installed_bytes = None;
    view.total_bytes = None;
    view.progress_percent = None;
    view.speed_mbps = None;
    view.message = message;
    view
}

fn fallback_model_terminal_command_view(error: &SttCommandError) -> nemotron::FallbackModelView {
    let enabled = settings::local_fallback_enabled();
    if matches!(
        error.code.as_str(),
        "MODEL_INSTALL_CANCELLED" | "MODEL_MISSING" | "MODEL_CORRUPT"
    ) {
        return persisted_fallback_model_view();
    }
    let mut view = nemotron::model_status(enabled);
    view.status = nemotron::FallbackModelStatus::Error;
    view.installed_bytes = None;
    view.total_bytes = None;
    view.progress_percent = None;
    view.speed_mbps = None;
    view.message = Some(error.message.clone());
    view
}

fn emit_fallback_status(
    app: &AppHandle,
    install_state: &FallbackModelInstallState,
    operation: &crate::stt::model::DownloadOperation,
    phase: FallbackModelInstallPhase,
    view: nemotron::FallbackModelView,
) -> Result<(), SttCommandError> {
    let view = sanitize_fallback_model_view(view);
    if !install_state.set_phase(operation.generation(), phase, view.clone()) {
        return Err(model_operation_error(
            "MODEL_OPERATION_STALE",
            "Model status belongs to an inactive operation.",
        ));
    }
    app.emit(FALLBACK_MODEL_STATUS_EVENT, &view)
        .map_err(|error| {
            model_operation_error(
                "MODEL_STATUS_PUBLISH_FAILED",
                &format!("Could not publish model status: {error}"),
            )
        })
}

fn emit_fallback_progress(
    app: &AppHandle,
    install_state: &FallbackModelInstallState,
    operation: &crate::stt::model::DownloadOperation,
    view: nemotron::FallbackModelView,
) -> Result<(), SttCommandError> {
    let view = sanitize_fallback_model_view(view);
    if !install_state.set_progress(operation.generation(), view.clone()) {
        return Err(model_operation_error(
            "MODEL_OPERATION_STALE",
            "Model progress belongs to an inactive operation.",
        ));
    }
    app.emit(FALLBACK_MODEL_PROGRESS_EVENT, &view)
        .map_err(|error| {
            model_operation_error(
                "MODEL_PROGRESS_PUBLISH_FAILED",
                &format!("Could not publish model progress: {error}"),
            )
        })
}

fn emit_terminal_status(
    app: &AppHandle,
    view: &nemotron::FallbackModelView,
) -> Result<(), SttCommandError> {
    app.emit(FALLBACK_MODEL_STATUS_EVENT, view)
        .map_err(|error| {
            model_operation_error(
                "MODEL_STATUS_PUBLISH_FAILED",
                &format!("Could not publish terminal model status: {error}"),
            )
        })
}

fn sanitize_fallback_model_view(
    mut view: nemotron::FallbackModelView,
) -> nemotron::FallbackModelView {
    if view
        .progress_percent
        .is_some_and(|value| !value.is_finite())
    {
        view.progress_percent = None;
    }
    if view.speed_mbps.is_some_and(|value| !value.is_finite()) {
        view.speed_mbps = None;
    }
    view
}

fn is_final_fallback_progress(view: &nemotron::FallbackModelView) -> bool {
    match view.status {
        nemotron::FallbackModelStatus::Downloading => {
            view.progress_percent
                .is_some_and(|percent| percent >= 100.0)
                || matches!(
                    (view.installed_bytes, view.total_bytes),
                    (Some(installed), Some(total)) if total > 0 && installed >= total
                )
        }
        _ => true,
    }
}

fn percent_changed(previous: Option<f32>, next: Option<f32>, delta: f32) -> bool {
    match (previous, next) {
        (Some(previous), Some(next)) => (next - previous).abs() >= delta,
        (None, Some(_)) | (Some(_), None) => true,
        (None, None) => false,
    }
}

fn fallback_model_command_error(code: &str, error: &impl std::fmt::Display) -> SttCommandError {
    SttCommandError {
        code: code.into(),
        message: format!("{error}"),
    }
}

fn ensure_model_mutation_idle(
    install_state: &FallbackModelInstallState,
) -> Result<(), SttCommandError> {
    if install_state.is_active() {
        return Err(SttCommandError::from(SttError::Busy));
    }
    Ok(())
}

fn model_operation_error(code: &str, message: &str) -> SttCommandError {
    SttCommandError {
        code: code.into(),
        message: message.into(),
    }
}

fn stale_model_operation_error() -> SttCommandError {
    model_operation_error(
        "MODEL_OPERATION_STALE",
        "Model operation is no longer active.",
    )
}

fn operation_cleanup_result(
    operation: &crate::stt::model::DownloadOperation,
) -> Result<(), SttCommandError> {
    match operation.take_cleanup_failure() {
        Some(message) => Err(model_operation_error("MODEL_TEMP_CLEANUP_FAILED", &message)),
        None => Ok(()),
    }
}

fn merge_errors(
    primary: Option<SttCommandError>,
    secondary: Option<SttCommandError>,
) -> Option<SttCommandError> {
    match (primary, secondary) {
        (Some(mut primary), Some(secondary)) => {
            primary.message.push_str("; ");
            primary.message.push_str(&secondary.code);
            primary.message.push_str(": ");
            primary.message.push_str(&secondary.message);
            Some(primary)
        }
        (Some(error), None) | (None, Some(error)) => Some(error),
        (None, None) => None,
    }
}

fn finalize_operation<C, P>(
    install_state: &FallbackModelInstallState,
    operation: &crate::stt::model::DownloadOperation,
    final_view: nemotron::FallbackModelView,
    worker_error: Option<SttCommandError>,
    cleanup: C,
    publish: P,
) -> Result<nemotron::FallbackModelView, SttCommandError>
where
    C: FnOnce() -> Result<(), SttCommandError>,
    P: FnOnce(&nemotron::FallbackModelView) -> Result<(), SttCommandError>,
{
    install_state.claim_terminal(operation.generation())?;
    let finalization = catch_unwind(AssertUnwindSafe(|| {
        let cleanup_error = cleanup().err();
        let publication_error = publish(&final_view).err();
        let error = merge_errors(cleanup_error, worker_error.clone());
        merge_errors(publication_error, error)
    }));
    let terminal_error = match finalization {
        Ok(error) => error,
        Err(_) => merge_errors(
            Some(model_operation_error(
                "MODEL_FINALIZATION_PANIC",
                "Model finalization panicked.",
            )),
            worker_error,
        ),
    };

    install_state.finish_generation(
        operation.generation(),
        final_view.clone(),
        terminal_error.clone(),
    )?;
    match terminal_error {
        Some(error) => Err(error),
        None => Ok(final_view),
    }
}

#[cfg(test)]
mod tests {
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

        state.claim_terminal(first_generation).unwrap();
        state
            .finish_generation(
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
            .finish_generation(
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
            .finish_generation(
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
}
