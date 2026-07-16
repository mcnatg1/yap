#[cfg(test)]
use crate::audio::{
    coordinator::{bounded_sink, SinkKind},
    recording::RecordingSinkHandle,
};
use crate::audio::{recording::RecordingFinalizeResult, session::SessionId};
use std::sync::atomic::Ordering;

use super::{
    log_worker_shutdown_errors, stream_session::StreamFinisher, LiveRuntime, LiveStopResult,
    StreamFinishStatus,
};

impl LiveRuntime {
    pub fn stop(&self) -> LiveStopResult {
        self.cancel_pending_start();
        self.run_stop_lifecycle(|| {
            let stream = self.stop_stream();
            self.finish_stop(stream)
        })
    }

    pub(crate) fn stop_stream(&self) -> StreamFinishStatus {
        let (finisher, adapter_status, shutdown_errors) = {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            let (shutdown_errors, adapter_status) = inner.stop_capture();
            (
                adapter_status
                    .is_none()
                    .then(|| inner.stream_finisher())
                    .flatten(),
                adapter_status,
                shutdown_errors,
            )
        };
        log_worker_shutdown_errors(shutdown_errors);
        let finish_status = adapter_status.unwrap_or_else(|| {
            finisher
                .as_ref()
                .map(StreamFinisher::finish_session)
                .unwrap_or(StreamFinishStatus::NoStream)
        });
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        if finish_status.should_retire_stream() {
            inner.retire_stream_detached_reader();
        }
        self.active_session.store(0, Ordering::SeqCst);
        inner.mark_used();
        finish_status
    }

    pub(crate) fn finish_stop(&self, stream: StreamFinishStatus) -> LiveStopResult {
        self.stop_completion.complete_with(|| LiveStopResult {
            stream,
            recording: self.finalize_recording(),
        })
    }

    pub(crate) fn finalize_recording(&self) -> Result<Option<RecordingFinalizeResult>, String> {
        self.recording_finalization
            .finalize_with(|| match self.inner.lock() {
                Ok(mut inner) => {
                    let recording = inner.take_recording();
                    let session_id = recording
                        .as_ref()
                        .map(|recording| recording.session_id().clone());
                    (
                        recording.map(|recording| recording.finalize()).transpose(),
                        session_id,
                    )
                }
                Err(_) => (Err("live runtime became unavailable".into()), None),
            })
    }

    pub(crate) fn recording_finalization_failure(&self) -> Option<(SessionId, String)> {
        self.recording_finalization.failure()
    }

    pub(super) fn ensure_recording_ready_to_start(&self) -> Result<(), String> {
        let prior_recording = self
            .inner
            .lock()
            .map_err(|_| "live runtime became unavailable")?
            .recording_is_present();
        if prior_recording {
            return Err("Previous live recording must be finalized before starting again.".into());
        }
        self.recording_finalization.prepare_for_new_recording()?;
        self.stop_completion.reset()
    }

    #[cfg(test)]
    pub(crate) fn install_unavailable_recording_for_test(&self, session_id: SessionId) {
        let (sink, _receiver) = bounded_sink(SinkKind::Recording, 1);
        self.inner.lock().unwrap().set_recording_for_test(
            RecordingSinkHandle::spawn_unavailable_for_test(sink, session_id),
        );
    }

    #[cfg(test)]
    pub(crate) fn install_panicking_recording_for_test(&self, session_id: SessionId) {
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
        self.inner.lock().unwrap().set_recording_for_test(
            RecordingSinkHandle::spawn_panicking_for_test(sink, receiver, session_id),
        );
    }
}
