use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::stt::{dispatch::SttCommandError, model::DownloadOperation, nemotron, settings};

use super::operation::{
    model_operation_error, FallbackModelInstallPhase, FallbackModelInstallState,
};

const FALLBACK_MODEL_STATUS_EVENT: &str = "fallback-model-status";
const FALLBACK_MODEL_PROGRESS_EVENT: &str = "fallback-model-progress";
pub(super) const FALLBACK_PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(200);
const FALLBACK_PROGRESS_MIN_PERCENT_DELTA: f32 = 1.0;

#[derive(Debug, Default)]
pub(super) struct FallbackProgressThrottle {
    emitted_once: bool,
    last_emit_at: Option<Instant>,
    last_progress_percent: Option<f32>,
}

impl FallbackProgressThrottle {
    pub(super) fn should_emit(&mut self, view: &nemotron::FallbackModelView, now: Instant) -> bool {
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

pub(super) struct FallbackProgressEmitter {
    app: AppHandle,
    install_state: FallbackModelInstallState,
    operation: DownloadOperation,
    throttle: FallbackProgressThrottle,
    publication_failure: Option<SttCommandError>,
}

impl FallbackProgressEmitter {
    pub(super) fn new(
        app: AppHandle,
        install_state: FallbackModelInstallState,
        operation: DownloadOperation,
    ) -> Self {
        Self {
            app,
            install_state,
            operation,
            throttle: FallbackProgressThrottle::default(),
            publication_failure: None,
        }
    }

    pub(super) fn publish(&mut self, view: nemotron::FallbackModelView) {
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
            if let Err(error) = self.app.emit_to(
                crate::authorization::MAIN_WINDOW_LABEL,
                FALLBACK_MODEL_PROGRESS_EVENT,
                &view,
            ) {
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

    pub(super) fn take_failure(&mut self) -> Option<SttCommandError> {
        self.publication_failure.take()
    }
}

pub(super) fn persisted_fallback_model_view() -> nemotron::FallbackModelView {
    nemotron::model_status(settings::local_fallback_enabled())
}

pub(super) fn fallback_model_phase_view(
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

pub(super) fn fallback_model_terminal_command_view(
    error: &SttCommandError,
) -> nemotron::FallbackModelView {
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

pub(super) fn emit_fallback_status(
    app: &AppHandle,
    install_state: &FallbackModelInstallState,
    operation: &DownloadOperation,
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
    app.emit_to(
        crate::authorization::MAIN_WINDOW_LABEL,
        FALLBACK_MODEL_STATUS_EVENT,
        &view,
    )
    .map_err(|error| {
        model_operation_error(
            "MODEL_STATUS_PUBLISH_FAILED",
            &format!("Could not publish model status: {error}"),
        )
    })
}

pub(super) fn emit_fallback_progress(
    app: &AppHandle,
    install_state: &FallbackModelInstallState,
    operation: &DownloadOperation,
    view: nemotron::FallbackModelView,
) -> Result<(), SttCommandError> {
    let view = sanitize_fallback_model_view(view);
    if !install_state.set_progress(operation.generation(), view.clone()) {
        return Err(model_operation_error(
            "MODEL_OPERATION_STALE",
            "Model progress belongs to an inactive operation.",
        ));
    }
    app.emit_to(
        crate::authorization::MAIN_WINDOW_LABEL,
        FALLBACK_MODEL_PROGRESS_EVENT,
        &view,
    )
    .map_err(|error| {
        model_operation_error(
            "MODEL_PROGRESS_PUBLISH_FAILED",
            &format!("Could not publish model progress: {error}"),
        )
    })
}

pub(super) fn emit_terminal_status(
    app: &AppHandle,
    view: &nemotron::FallbackModelView,
) -> Result<(), SttCommandError> {
    app.emit_to(
        crate::authorization::MAIN_WINDOW_LABEL,
        FALLBACK_MODEL_STATUS_EVENT,
        view,
    )
    .map_err(|error| {
        model_operation_error(
            "MODEL_STATUS_PUBLISH_FAILED",
            &format!("Could not publish terminal model status: {error}"),
        )
    })
}

pub(super) fn sanitize_fallback_model_view(
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
