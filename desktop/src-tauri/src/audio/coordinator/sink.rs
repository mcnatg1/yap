use std::sync::{atomic::Ordering, mpsc};

#[cfg(test)]
use std::sync::Arc;

use super::sink_types::{
    BoundedSink, SinkCompletionGate, SinkDegradeResult, SinkGatePhase, SinkOutcome, SinkSendError,
};

impl<T> BoundedSink<T> {
    pub fn try_send(&self, frame: T) -> Result<(), SinkSendError> {
        let mut completion = self
            .state
            .completion
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if completion.phase != SinkGatePhase::Accepting {
            self.state.dropped_frames.fetch_add(1, Ordering::Relaxed);
            return Err(SinkSendError::Closed);
        }
        let sender = match self.state.sender.lock() {
            Ok(sender) => sender,
            Err(_) => {
                self.record_drop_locked(&mut completion, "sink state became unavailable");
                return Err(SinkSendError::Closed);
            }
        };
        let Some(sender) = sender.as_ref() else {
            self.record_drop_locked(&mut completion, "sink closed");
            return Err(SinkSendError::Closed);
        };
        if self.state.closed.load(Ordering::Acquire) {
            self.record_drop_locked(&mut completion, "sink closed");
            return Err(SinkSendError::Closed);
        }
        let Some(reserved_queued) = self.reserve_queue_slot() else {
            self.record_drop_locked(&mut completion, "sink queue is full");
            return Err(SinkSendError::Full);
        };
        match sender.try_send(frame) {
            Ok(()) => {
                self.state.published_frames.fetch_add(1, Ordering::Release);
                #[cfg(test)]
                self.run_after_publish_hook_for_test();
                self.state.accepted_frames.fetch_add(1, Ordering::Relaxed);
                self.observe_high_water_mark(reserved_queued);
                Ok(())
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.rollback_reservation();
                self.record_drop_locked(&mut completion, "sink queue is full");
                Err(SinkSendError::Full)
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.state.queued_frames.store(0, Ordering::Release);
                self.state.published_frames.store(0, Ordering::Release);
                self.state.closed.store(true, Ordering::Release);
                self.record_drop_locked(&mut completion, "sink receiver disconnected");
                Err(SinkSendError::Closed)
            }
        }
    }

    pub fn close(&self) {
        let Ok(mut sender) = self.state.sender.lock() else {
            self.state.closed.store(true, Ordering::Release);
            return;
        };
        if sender.take().is_some() {
            self.state.close_count.fetch_add(1, Ordering::Relaxed);
            self.state.closed.store(true, Ordering::Release);
        }
    }

    pub(super) fn close_with_error(&self, error: &str) {
        self.degrade(error);
        self.close();
    }

    pub(crate) fn degrade(&self, error: &str) -> SinkDegradeResult {
        let mut completion = self
            .state
            .completion
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match completion.phase {
            SinkGatePhase::Accepting => {
                completion
                    .degradation
                    .get_or_insert_with(|| error.to_string());
                SinkDegradeResult::Accepted
            }
            SinkGatePhase::Completing => SinkDegradeResult::CompletionInProgress,
            SinkGatePhase::Published => SinkDegradeResult::Published,
        }
    }

    pub(crate) fn begin_completion(&self) -> Option<String> {
        #[cfg(test)]
        self.run_before_completion_hook_for_test();
        let mut completion = self
            .state
            .completion
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        debug_assert_eq!(completion.phase, SinkGatePhase::Accepting);
        completion.phase = SinkGatePhase::Completing;
        completion.degradation.clone()
    }

    pub(crate) fn mark_published(&self) {
        self.state
            .completion
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .phase = SinkGatePhase::Published;
    }

    pub fn outcome(&self) -> SinkOutcome {
        let error = self
            .state
            .completion
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .degradation
            .clone();
        SinkOutcome {
            kind: self.kind,
            accepted_frames: self.state.accepted_frames.load(Ordering::Acquire),
            dropped_frames: self.state.dropped_frames.load(Ordering::Acquire),
            closed: self.state.closed.load(Ordering::Acquire),
            error,
        }
    }

    pub fn high_water_mark(&self) -> usize {
        self.state.high_water_mark.load(Ordering::Acquire)
    }

    pub fn close_count(&self) -> usize {
        self.state.close_count.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn set_after_publish_hook_for_test(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.state.after_publish_hook.lock().unwrap() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_before_completion_hook_for_test(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.state.before_completion_hook.lock().unwrap() = Some(hook);
    }

    #[cfg(test)]
    pub(super) fn queued_frames_for_test(&self) -> usize {
        self.state.queued_frames.load(Ordering::Acquire)
    }

    fn reserve_queue_slot(&self) -> Option<usize> {
        let mut queued = self.state.queued_frames.load(Ordering::Acquire);
        loop {
            if queued >= self.state.queue_capacity {
                return None;
            }
            match self.state.queued_frames.compare_exchange_weak(
                queued,
                queued + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(queued + 1),
                Err(observed) => queued = observed,
            }
        }
    }

    fn rollback_reservation(&self) {
        let result =
            self.state
                .queued_frames
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                    queued.checked_sub(1)
                });
        debug_assert!(result.is_ok(), "a failed sink send must have a reservation");
    }

    #[cfg(test)]
    fn run_after_publish_hook_for_test(&self) {
        if let Some(hook) = self.state.after_publish_hook.lock().unwrap().as_ref() {
            hook();
        }
    }

    #[cfg(test)]
    fn run_before_completion_hook_for_test(&self) {
        if let Some(hook) = self.state.before_completion_hook.lock().unwrap().as_ref() {
            hook();
        }
    }

    fn record_drop_locked(&self, completion: &mut SinkCompletionGate, error: &str) {
        self.state.dropped_frames.fetch_add(1, Ordering::Relaxed);
        if completion.phase == SinkGatePhase::Accepting {
            completion
                .degradation
                .get_or_insert_with(|| error.to_string());
        }
    }

    fn observe_high_water_mark(&self, queued: usize) {
        let mut current = self.state.high_water_mark.load(Ordering::Acquire);
        while queued > current {
            match self.state.high_water_mark.compare_exchange_weak(
                current,
                queued,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    #[cfg(debug_assertions)]
                    crate::stt::log_yap(&format!(
                        "audio sink {:?} queue high-water mark={queued}",
                        self.kind
                    ));
                    break;
                }
                Err(observed) => current = observed,
            }
        }
    }
}
