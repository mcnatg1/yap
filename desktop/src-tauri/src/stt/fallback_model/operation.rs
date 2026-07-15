use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Mutex},
};

use crate::stt::{dispatch::SttCommandError, error::SttError, model::DownloadOperation, nemotron};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FallbackModelInstallPhase {
    Installing,
    Verifying,
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(super) struct FallbackModelInstallSnapshot {
    pub(super) generation: Option<u64>,
    pub(super) phase: Option<FallbackModelInstallPhase>,
    pub(super) error: Option<SttCommandError>,
}

#[derive(Debug, Clone)]
struct ActiveFallbackModelOperation {
    phase: FallbackModelInstallPhase,
    cancellable: bool,
    terminal_claimed: bool,
    operation: DownloadOperation,
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

    pub(super) fn begin(
        &self,
        phase: FallbackModelInstallPhase,
        view: nemotron::FallbackModelView,
        cancellable: bool,
    ) -> Result<DownloadOperation, Box<nemotron::FallbackModelView>> {
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
        let operation = DownloadOperation::new(generation);
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
    pub(super) fn snapshot(&self) -> FallbackModelInstallSnapshot {
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

    pub(super) fn current_view(&self) -> Option<nemotron::FallbackModelView> {
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

    pub(super) fn set_phase(
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

    pub(super) fn set_progress(&self, generation: u64, view: nemotron::FallbackModelView) -> bool {
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
    pub(super) fn cancel_generation(&self, generation: u64) -> bool {
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

    #[cfg(test)]
    pub(super) fn claim_terminal_for_test(&self, generation: u64) -> Result<(), SttCommandError> {
        self.claim_terminal(generation)
    }

    pub(super) fn cancel_install(&self) -> bool {
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

    #[cfg(test)]
    pub(super) fn finish_generation_for_test(
        &self,
        generation: u64,
        view: nemotron::FallbackModelView,
        error: Option<SttCommandError>,
    ) -> Result<(), SttCommandError> {
        self.finish_generation(generation, view, error)
    }
}

pub(super) fn ensure_model_mutation_idle(
    install_state: &FallbackModelInstallState,
) -> Result<(), SttCommandError> {
    if install_state.is_active() {
        return Err(SttCommandError::from(SttError::Busy));
    }
    Ok(())
}

pub(super) fn model_operation_error(code: &str, message: &str) -> SttCommandError {
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

pub(super) fn operation_cleanup_result(
    operation: &DownloadOperation,
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

pub(super) fn finalize_operation<C, P>(
    install_state: &FallbackModelInstallState,
    operation: &DownloadOperation,
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
