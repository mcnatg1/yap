use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
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

#[derive(Debug, Clone, Default)]
struct FallbackModelInstallSnapshot {
    phase: Option<FallbackModelInstallPhase>,
    view: Option<nemotron::FallbackModelView>,
    progress: Option<nemotron::FallbackModelView>,
    error: Option<SttCommandError>,
}

#[derive(Debug, Default)]
struct FallbackModelInstallInner {
    phase: Option<FallbackModelInstallPhase>,
    view: Option<nemotron::FallbackModelView>,
    progress: Option<nemotron::FallbackModelView>,
    error: Option<SttCommandError>,
}

#[derive(Clone, Default)]
pub struct FallbackModelInstallState {
    inner: Arc<Mutex<FallbackModelInstallInner>>,
    cancellation: Arc<Mutex<Option<Arc<AtomicBool>>>>,
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
    ) -> Result<Option<Arc<AtomicBool>>, nemotron::FallbackModelView> {
        {
            let mut inner = self.inner.lock().expect("fallback model state poisoned");
            if inner.phase.is_some() {
                return Err(inner
                    .progress
                    .clone()
                    .or_else(|| inner.view.clone())
                    .unwrap_or(view));
            }
            inner.phase = Some(phase);
            inner.view = Some(view);
            inner.progress = None;
            inner.error = None;
        }

        let token = cancellable.then(|| Arc::new(AtomicBool::new(false)));
        let mut cancellation = self
            .cancellation
            .lock()
            .expect("fallback model cancellation state poisoned");
        *cancellation = token.clone();
        Ok(token)
    }

    fn snapshot(&self) -> FallbackModelInstallSnapshot {
        let inner = self.inner.lock().expect("fallback model state poisoned");
        FallbackModelInstallSnapshot {
            phase: inner.phase,
            view: inner.view.clone(),
            progress: inner.progress.clone(),
            error: inner.error.clone(),
        }
    }

    fn current_view(&self) -> Option<nemotron::FallbackModelView> {
        let snapshot = self.snapshot();
        if snapshot.error.is_some() {
            return snapshot.progress.or(snapshot.view);
        }
        snapshot.progress.or(snapshot.view)
    }

    fn set_phase(&self, phase: FallbackModelInstallPhase, view: nemotron::FallbackModelView) {
        let mut inner = self.inner.lock().expect("fallback model state poisoned");
        inner.phase = Some(phase);
        inner.view = Some(view);
        inner.progress = None;
        inner.error = None;
    }

    fn set_progress(&self, view: nemotron::FallbackModelView) {
        let mut inner = self.inner.lock().expect("fallback model state poisoned");
        inner.progress = Some(view.clone());
        inner.view = Some(view);
    }

    fn set_error(&self, error: SttCommandError) {
        let mut inner = self.inner.lock().expect("fallback model state poisoned");
        inner.error = Some(error);
    }

    fn cancel_install(&self) {
        if let Some(token) = self
            .cancellation
            .lock()
            .expect("fallback model cancellation state poisoned")
            .as_ref()
        {
            token.store(true, Ordering::Relaxed);
        }
    }

    fn clear(&self) {
        {
            let mut inner = self.inner.lock().expect("fallback model state poisoned");
            *inner = FallbackModelInstallInner::default();
        }
        let mut cancellation = self
            .cancellation
            .lock()
            .expect("fallback model cancellation state poisoned");
        *cancellation = None;
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
    throttle: FallbackProgressThrottle,
}

impl FallbackProgressEmitter {
    fn new(app: AppHandle, install_state: FallbackModelInstallState) -> Self {
        Self {
            app,
            install_state,
            throttle: FallbackProgressThrottle::default(),
        }
    }

    fn publish(&mut self, view: nemotron::FallbackModelView) {
        let view = sanitize_fallback_model_view(view);
        self.install_state.set_progress(view.clone());
        if self.throttle.should_emit(&view, Instant::now()) {
            let _ = self.app.emit(FALLBACK_MODEL_PROGRESS_EVENT, &view);
        }
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
    let cancellation = match install_state.begin(
        FallbackModelInstallPhase::Installing,
        initial_view.clone(),
        true,
    ) {
        Ok(cancellation) => cancellation,
        Err(active) => return Ok(active),
    };
    emit_fallback_progress(&app, &install_state, initial_view);

    tauri::async_runtime::spawn_blocking(move || {
        let final_view = {
            let mut progress = FallbackProgressEmitter::new(app.clone(), install_state.clone());
            let result = (|| -> Result<nemotron::FallbackModelView, SttError> {
                settings::set_local_fallback_enabled(true)?;
                let cancellation = cancellation.clone();
                let is_cancelled = || {
                    cancellation
                        .as_ref()
                        .is_some_and(|token| token.load(Ordering::Relaxed))
                };
                nemotron::ensure_model_with_progress(
                    force,
                    |view| progress.publish(view),
                    is_cancelled,
                )?;
                let verifying_view = fallback_model_phase_view(
                    true,
                    nemotron::FallbackModelStatus::Verifying,
                    Some("Verifying files".into()),
                );
                emit_fallback_status_with_phase(
                    &app,
                    &install_state,
                    FallbackModelInstallPhase::Verifying,
                    verifying_view,
                );
                let view = nemotron::verify_model_with_progress(
                    true,
                    |view| progress.publish(view),
                    is_cancelled,
                );
                if is_cancelled() {
                    let _ = nemotron::remove_model();
                    return Err(SttError::ModelInstallCancelled);
                }
                Ok(view)
            })();

            match result {
                Ok(view) => sanitize_fallback_model_view(view),
                Err(SttError::ModelInstallCancelled) => {
                    let _ = nemotron::remove_model();
                    install_state.set_error(SttCommandError::from(SttError::ModelInstallCancelled));
                    persisted_fallback_model_view()
                }
                Err(error) => {
                    install_state.set_error(SttCommandError::from(error));
                    sanitize_fallback_model_view(fallback_model_terminal_view(error))
                }
            }
        };

        emit_fallback_status(&app, &install_state, final_view.clone());
        install_state.clear();
        Ok(final_view)
    })
    .await
    .map_err(|_| SttCommandError::from(SttError::SidecarCrash))?
}

pub fn cancel_install(
    install_state: &FallbackModelInstallState,
) -> Result<nemotron::FallbackModelView, SttCommandError> {
    if install_state.snapshot().phase.is_some() {
        install_state.cancel_install();
    }
    Ok(status(install_state))
}

pub fn verify(
    app: AppHandle,
    install_state: FallbackModelInstallState,
) -> Result<nemotron::FallbackModelView, SttCommandError> {
    let initial_view = fallback_model_phase_view(
        settings::local_fallback_enabled(),
        nemotron::FallbackModelStatus::Verifying,
        Some("Verifying files".into()),
    );
    match install_state.begin(
        FallbackModelInstallPhase::Verifying,
        initial_view.clone(),
        false,
    ) {
        Ok(_) => emit_fallback_status(&app, &install_state, initial_view),
        Err(active) => return Ok(active),
    }

    tauri::async_runtime::block_on(async move {
        tauri::async_runtime::spawn_blocking(move || {
            let final_view = {
                let mut progress = FallbackProgressEmitter::new(app.clone(), install_state.clone());
                sanitize_fallback_model_view(nemotron::verify_model_with_progress(
                    settings::local_fallback_enabled(),
                    |view| progress.publish(view),
                    || false,
                ))
            };

            emit_fallback_status(&app, &install_state, final_view.clone());
            install_state.clear();
            Ok(final_view)
        })
        .await
        .map_err(|_| SttCommandError::from(SttError::SidecarCrash))?
    })
}

pub fn remove() -> Result<nemotron::FallbackModelView, SttCommandError> {
    nemotron::remove_model().map_err(SttCommandError::from)?;
    settings::set_local_fallback_enabled(false)?;
    Ok(nemotron::model_status(false))
}

pub fn set_enabled(enabled: bool) -> Result<nemotron::FallbackModelView, SttCommandError> {
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

fn fallback_model_terminal_view(error: SttError) -> nemotron::FallbackModelView {
    let enabled = settings::local_fallback_enabled();
    match error {
        SttError::ModelInstallCancelled | SttError::ModelMissing | SttError::ModelCorrupt => {
            persisted_fallback_model_view()
        }
        other => {
            let mut view = nemotron::model_status(enabled);
            view.status = nemotron::FallbackModelStatus::Error;
            view.installed_bytes = None;
            view.total_bytes = None;
            view.progress_percent = None;
            view.speed_mbps = None;
            view.message = Some(other.user_message().to_string());
            view
        }
    }
}

fn emit_fallback_status(
    app: &AppHandle,
    install_state: &FallbackModelInstallState,
    view: nemotron::FallbackModelView,
) {
    let phase = install_state
        .snapshot()
        .phase
        .unwrap_or(FallbackModelInstallPhase::Verifying);
    emit_fallback_status_with_phase(app, install_state, phase, view);
}

fn emit_fallback_status_with_phase(
    app: &AppHandle,
    install_state: &FallbackModelInstallState,
    phase: FallbackModelInstallPhase,
    view: nemotron::FallbackModelView,
) {
    let view = sanitize_fallback_model_view(view);
    install_state.set_phase(phase, view.clone());
    let _ = app.emit(FALLBACK_MODEL_STATUS_EVENT, &view);
}

fn emit_fallback_progress(
    app: &AppHandle,
    install_state: &FallbackModelInstallState,
    view: nemotron::FallbackModelView,
) {
    let view = sanitize_fallback_model_view(view);
    install_state.set_progress(view.clone());
    let _ = app.emit(FALLBACK_MODEL_PROGRESS_EVENT, &view);
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
        let cancellation = state
            .begin(FallbackModelInstallPhase::Installing, initial.clone(), true)
            .unwrap()
            .expect("install should create a cancellation token");

        state.set_phase(
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
        assert!(cancellation.load(Ordering::Relaxed));
    }

    #[test]
    fn fallback_model_status_prefers_transient_progress_view() {
        let state = FallbackModelInstallState::new();
        state
            .begin(
                FallbackModelInstallPhase::Installing,
                fallback_test_view(nemotron::FallbackModelStatus::Downloading),
                true,
            )
            .unwrap();
        let mut progress = fallback_test_view(nemotron::FallbackModelStatus::Downloading);
        progress.progress_percent = Some(42.0);
        state.set_progress(progress.clone());

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
        let cancellation = state
            .begin(
                FallbackModelInstallPhase::Installing,
                fallback_test_view(nemotron::FallbackModelStatus::Downloading),
                true,
            )
            .unwrap()
            .expect("install should create cancellation token");
        state.set_phase(
            FallbackModelInstallPhase::Verifying,
            fallback_test_view(nemotron::FallbackModelStatus::Verifying),
        );

        let _ = cancel_install(&state).unwrap();

        assert!(cancellation.load(Ordering::Relaxed));
    }
}
