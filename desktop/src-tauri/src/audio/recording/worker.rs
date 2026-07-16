use super::*;

pub struct RecordingSinkHandle {
    sink: BoundedSink<RecordingInput>,
    session_id: SessionId,
    state: Mutex<RecordingSinkState>,
    completed: Condvar,
    #[cfg(test)]
    finalization_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

struct RecordingSinkState {
    worker: Option<JoinHandle<RecordingFinalizeResult>>,
    result: Option<Result<RecordingFinalizeResult, String>>,
    finalizing: bool,
}

impl RecordingSinkHandle {
    pub fn spawn(
        directory: PathBuf,
        session_id: SessionId,
        sink: BoundedSink<RecordingInput>,
        receiver: BoundedReceiver<RecordingInput>,
    ) -> Self {
        Self::spawn_inner(directory, session_id, sink, receiver, None)
    }

    pub(crate) fn spawn_reserved(
        reservation: RecordingReservation,
        sink: BoundedSink<RecordingInput>,
        receiver: BoundedReceiver<RecordingInput>,
    ) -> Self {
        let directory = reservation.paths.directory.clone();
        let session_id = reservation.session_id().clone();
        Self::spawn_inner(directory, session_id, sink, receiver, Some(reservation))
    }

    fn spawn_inner(
        directory: PathBuf,
        session_id: SessionId,
        sink: BoundedSink<RecordingInput>,
        receiver: BoundedReceiver<RecordingInput>,
        reservation: Option<RecordingReservation>,
    ) -> Self {
        let worker_session_id = session_id.clone();
        let worker_sink = sink.clone();
        let worker = std::thread::spawn(move || {
            run_recording_worker(
                directory,
                worker_session_id,
                receiver,
                reservation,
                worker_sink,
            )
        });
        Self::with_worker(sink, session_id, worker)
    }

    pub(super) fn with_worker(
        sink: BoundedSink<RecordingInput>,
        session_id: SessionId,
        worker: JoinHandle<RecordingFinalizeResult>,
    ) -> Self {
        Self {
            sink,
            session_id,
            state: Mutex::new(RecordingSinkState {
                worker: Some(worker),
                result: None,
                finalizing: false,
            }),
            completed: Condvar::new(),
            #[cfg(test)]
            finalization_count: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    #[cfg(test)]
    pub(crate) fn spawn_with_fault_for_test(
        directory: PathBuf,
        session_id: SessionId,
        sink: BoundedSink<RecordingInput>,
        receiver: BoundedReceiver<RecordingInput>,
        fault: CommitFaultPoint,
        append_write_attempts: Arc<std::sync::atomic::AtomicUsize>,
        journal_write_attempts: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        let worker_session_id = session_id.clone();
        let worker_sink = sink.clone();
        let worker = std::thread::spawn(move || {
            let mut recording = match StreamingRecording::create_with_fault(
                &directory,
                worker_session_id.clone(),
                fault,
            ) {
                Ok(recording) => recording,
                Err(error) => return worker_creation_failure(worker_session_id, error),
            };
            recording.append_write_attempts = Some(append_write_attempts);
            recording.journal_write_attempts = Some(journal_write_attempts);
            recording.sync_interval_samples = 1;
            drain_recording_worker(recording, worker_session_id, receiver, worker_sink)
        });
        Self::with_worker(sink, session_id, worker)
    }

    #[cfg(test)]
    pub(crate) fn spawn_panicking_for_test(
        sink: BoundedSink<RecordingInput>,
        _receiver: BoundedReceiver<RecordingInput>,
        session_id: SessionId,
    ) -> Self {
        Self::with_worker(
            sink,
            session_id,
            std::thread::spawn(|| -> RecordingFinalizeResult {
                panic!("injected recording worker panic")
            }),
        )
    }

    #[cfg(test)]
    pub(crate) fn spawn_unavailable_for_test(
        sink: BoundedSink<RecordingInput>,
        session_id: SessionId,
    ) -> Self {
        Self {
            sink,
            session_id,
            state: Mutex::new(RecordingSinkState {
                worker: None,
                result: None,
                finalizing: false,
            }),
            completed: Condvar::new(),
            finalization_count: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    #[cfg(test)]
    pub(crate) fn spawn_with_finalization_counter_for_test(
        directory: PathBuf,
        session_id: SessionId,
        sink: BoundedSink<RecordingInput>,
        receiver: BoundedReceiver<RecordingInput>,
    ) -> (Self, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let handle = Self::spawn(directory, session_id, sink, receiver);
        let count = std::sync::Arc::clone(&handle.finalization_count);
        (handle, count)
    }

    pub fn sink(&self) -> BoundedSink<RecordingInput> {
        self.sink.clone()
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn finalize(&self) -> Result<RecordingFinalizeResult, String> {
        self.sink.close();
        let worker = loop {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "recording handle became unavailable")?;
            if let Some(result) = &state.result {
                return result.clone();
            }
            if !state.finalizing {
                state.finalizing = true;
                #[cfg(test)]
                self.finalization_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                break state.worker.take();
            }
            state = self
                .completed
                .wait(state)
                .map_err(|_| "recording handle became unavailable")?;
            drop(state);
        };
        let result = match worker {
            Some(worker) => worker
                .join()
                .map_err(|_| "recording worker panicked during finalization".to_string()),
            None => Err("recording worker is unavailable".to_string()),
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.sink.mark_published();
        state.finalizing = false;
        state.result = Some(result.clone());
        self.completed.notify_all();
        result
    }

    pub fn abort(&self, reason: impl Into<String>) -> Result<RecordingFinalizeResult, String> {
        match self.sink.degrade(&reason.into()) {
            SinkDegradeResult::Accepted => self.finalize(),
            SinkDegradeResult::CompletionInProgress => {
                Err("recording completion is already in progress".into())
            }
            SinkDegradeResult::Published => self.finalize(),
        }
    }
}

fn run_recording_worker(
    directory: PathBuf,
    session_id: SessionId,
    receiver: BoundedReceiver<RecordingInput>,
    reservation: Option<RecordingReservation>,
    sink: BoundedSink<RecordingInput>,
) -> RecordingFinalizeResult {
    let recording = match reservation {
        Some(reservation) => StreamingRecording::create_reserved(reservation),
        None => StreamingRecording::create(&directory, session_id.clone()),
    };
    let recording = match recording {
        Ok(recording) => recording,
        Err(error) => return worker_creation_failure(session_id, error),
    };
    drain_recording_worker(recording, session_id, receiver, sink)
}

fn worker_creation_failure(session_id: SessionId, error: String) -> RecordingFinalizeResult {
    RecordingFinalizeResult {
        session_id,
        status: CaptureStatus::Partial,
        committed: None,
        partial_lineage: None,
        error: Some(error),
        sidecar_receipt: None,
    }
}

pub(super) fn drain_recording_worker(
    mut recording: StreamingRecording,
    session_id: SessionId,
    receiver: BoundedReceiver<RecordingInput>,
    sink: BoundedSink<RecordingInput>,
) -> RecordingFinalizeResult {
    let mut input_failed = false;
    loop {
        match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(input) => {
                if !input_failed {
                    if let Err(error) = recording.append_input(input) {
                        sink.degrade(&error);
                        recording.abort(error);
                        input_failed = true;
                    }
                    if recording.journal.sink_degraded {
                        sink.degrade("recording sequence discontinuity");
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if let Some(reason) = sink.begin_completion() {
        recording.abort(reason);
    }
    recording
        .finalize()
        .unwrap_or_else(|error| RecordingFinalizeResult {
            session_id,
            status: CaptureStatus::Partial,
            committed: None,
            partial_lineage: None,
            error: Some(error),
            sidecar_receipt: None,
        })
}
