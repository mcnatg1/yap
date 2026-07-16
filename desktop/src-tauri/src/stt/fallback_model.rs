#[cfg(test)]
use std::time::{Duration, Instant};

use tauri::AppHandle;

use crate::stt::{dispatch::SttCommandError, error::SttError, nemotron, settings};

mod operation;
mod progress;

pub use operation::FallbackModelInstallState;
use operation::{
    ensure_model_mutation_idle, finalize_operation, operation_cleanup_result,
    FallbackModelInstallPhase,
};
use progress::{
    emit_fallback_progress, emit_fallback_status, emit_terminal_status, fallback_model_phase_view,
    fallback_model_terminal_command_view, persisted_fallback_model_view,
    sanitize_fallback_model_view, FallbackProgressEmitter,
};
#[cfg(test)]
use progress::{FallbackProgressThrottle, FALLBACK_PROGRESS_MIN_INTERVAL};

pub fn status(install_state: &FallbackModelInstallState) -> nemotron::FallbackModelView {
    install_state
        .current_view()
        .unwrap_or_else(persisted_fallback_model_view)
}

pub async fn install<M>(
    app: AppHandle,
    install_state: FallbackModelInstallState,
    force: bool,
    model_mutation: M,
) -> Result<nemotron::FallbackModelView, SttCommandError>
where
    M: Send + 'static,
{
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
        let _model_mutation = model_mutation;
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

fn fallback_model_command_error(code: &str, error: &impl std::fmt::Display) -> SttCommandError {
    SttCommandError {
        code: code.into(),
        message: format!("{error}"),
    }
}

#[cfg(test)]
mod tests;
