use std::sync::{Condvar, Mutex};

use crate::audio::recording::RecordingFinalizeResult;
use crate::audio::session::SessionId;

pub(super) struct StopCompletion<T> {
    state: Mutex<StopCompletionState<T>>,
    completed: Condvar,
}

enum StopCompletionState<T> {
    Pending,
    Finalizing,
    Finalized(Box<T>),
}

pub(super) struct RecordingFinalization {
    state: Mutex<RecordingFinalizationState>,
    completed: Condvar,
}

enum RecordingFinalizationState {
    Pending,
    Finalizing,
    Finalized(Box<Option<RecordingFinalizeResult>>),
    Failed {
        error: String,
        session_id: Option<SessionId>,
    },
}

struct RecordingFinalizationLease<'a> {
    finalization: &'a RecordingFinalization,
    completed: bool,
}

impl<T> StopCompletion<T>
where
    T: Clone,
{
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(StopCompletionState::Pending),
            completed: Condvar::new(),
        }
    }

    pub(super) fn reset(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "live stop completion state became unavailable")?;
        if matches!(*state, StopCompletionState::Finalizing) {
            return Err("Previous live stop is still finalizing.".into());
        }
        *state = StopCompletionState::Pending;
        Ok(())
    }

    pub(super) fn complete_with<F>(&self, complete: F) -> T
    where
        F: FnOnce() -> T,
    {
        let mut complete = Some(complete);
        loop {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match &*state {
                StopCompletionState::Finalized(result) => return (**result).clone(),
                StopCompletionState::Pending => {
                    *state = StopCompletionState::Finalizing;
                    drop(state);
                    let complete = complete
                        .take()
                        .expect("one stop finalizer owns the completion");
                    let result = complete();
                    let mut state = self
                        .state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    *state = StopCompletionState::Finalized(Box::new(result.clone()));
                    self.completed.notify_all();
                    return result;
                }
                StopCompletionState::Finalizing => {
                    drop(
                        self.completed
                            .wait(state)
                            .unwrap_or_else(|poisoned| poisoned.into_inner()),
                    );
                }
            }
        }
    }
}

impl RecordingFinalization {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(RecordingFinalizationState::Pending),
            completed: Condvar::new(),
        }
    }

    pub(super) fn prepare_for_new_recording(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "recording finalization state became unavailable")?;
        if matches!(*state, RecordingFinalizationState::Finalizing) {
            return Err("Previous live recording is still finalizing.".into());
        }
        *state = RecordingFinalizationState::Pending;
        Ok(())
    }

    pub(super) fn finalize_with<F>(
        &self,
        finalize: F,
    ) -> Result<Option<RecordingFinalizeResult>, String>
    where
        F: FnOnce() -> (
            Result<Option<RecordingFinalizeResult>, String>,
            Option<SessionId>,
        ),
    {
        let lease = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "recording finalization state became unavailable")?;
            loop {
                match &*state {
                    RecordingFinalizationState::Finalized(result) => return Ok((**result).clone()),
                    RecordingFinalizationState::Failed { error, .. } => return Err(error.clone()),
                    RecordingFinalizationState::Pending => {
                        *state = RecordingFinalizationState::Finalizing;
                        break RecordingFinalizationLease::new(self);
                    }
                    RecordingFinalizationState::Finalizing => {
                        state = self
                            .completed
                            .wait(state)
                            .map_err(|_| "recording finalization state became unavailable")?;
                    }
                }
            }
        };
        let (result, session_id) = finalize();
        lease.finish(result, session_id)
    }

    pub(super) fn failure(&self) -> Option<(SessionId, String)> {
        let state = self.state.lock().ok()?;
        match &*state {
            RecordingFinalizationState::Failed {
                error,
                session_id: Some(session_id),
            } => Some((session_id.clone(), error.clone())),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(super) fn is_finalizing_for_test(&self) -> bool {
        matches!(
            *self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            RecordingFinalizationState::Finalizing
        )
    }
}

impl<'a> RecordingFinalizationLease<'a> {
    fn new(finalization: &'a RecordingFinalization) -> Self {
        Self {
            finalization,
            completed: false,
        }
    }

    fn finish(
        mut self,
        result: Result<Option<RecordingFinalizeResult>, String>,
        session_id: Option<SessionId>,
    ) -> Result<Option<RecordingFinalizeResult>, String> {
        let mut state = self
            .finalization
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *state = match &result {
            Ok(result) => RecordingFinalizationState::Finalized(Box::new(result.clone())),
            Err(error) => RecordingFinalizationState::Failed {
                error: error.clone(),
                session_id,
            },
        };
        self.completed = true;
        self.finalization.completed.notify_all();
        result
    }
}

impl Drop for RecordingFinalizationLease<'_> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let mut state = self
            .finalization
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(*state, RecordingFinalizationState::Finalizing) {
            *state = RecordingFinalizationState::Failed {
                error: "recording finalization interrupted before completion".into(),
                session_id: None,
            };
        }
        self.finalization.completed.notify_all();
    }
}
