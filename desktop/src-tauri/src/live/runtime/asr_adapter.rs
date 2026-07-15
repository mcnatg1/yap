use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::audio::coordinator::{
    bounded_sink, BoundedReceiver, BoundedSink, SinkKind, LOCAL_ASR_QUEUE_CAPACITY,
};
use crate::audio::frame::PreparedFrame;

use super::super::stream::StreamMessage;
use super::worker::join_worker;

pub(super) const ASR_ADAPTER_DRAIN_TIMEOUT: Duration = Duration::from_millis(6000);
const ASR_ADAPTER_CANCEL_GRACE: Duration = Duration::from_millis(100);

pub(super) struct SessionAsrAdapter {
    frames_tx: BoundedSink<PreparedFrame>,
    cancelled: Arc<AtomicBool>,
    completed_rx: Option<mpsc::Receiver<()>>,
    worker: Option<JoinHandle<()>>,
    cleanup_error: Option<String>,
}

pub(super) struct PendingAsrAdapter {
    frames_tx: BoundedSink<PreparedFrame>,
    frames_rx: Option<BoundedReceiver<PreparedFrame>>,
}

struct AdapterReapPayload {
    worker: JoinHandle<()>,
    completed_rx: mpsc::Receiver<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdapterDrainStatus {
    Drained,
    TimedOut,
    TimedOutRetained,
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_REAPER_SPAWN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl SessionAsrAdapter {
    #[cfg(test)]
    pub(super) fn start(samples_tx: mpsc::SyncSender<StreamMessage>, session: u64) -> Self {
        PendingAsrAdapter::new().start(samples_tx, session)
    }

    #[cfg(test)]
    fn start_with_completion_hook<F>(
        samples_tx: mpsc::SyncSender<StreamMessage>,
        session: u64,
        completion_hook: F,
    ) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        PendingAsrAdapter::new().start_with_completion_hook(samples_tx, session, completion_hook)
    }

    #[cfg(test)]
    pub(super) fn start_with_completion_gate_for_test(
        samples_tx: mpsc::SyncSender<StreamMessage>,
        session: u64,
        completion_gate: Arc<std::sync::Barrier>,
    ) -> Self {
        Self::start_with_completion_hook(samples_tx, session, move || {
            completion_gate.wait();
        })
    }

    #[cfg(test)]
    pub(super) fn sink(&self) -> BoundedSink<PreparedFrame> {
        self.frames_tx.clone()
    }

    #[cfg(test)]
    pub(super) fn join_after_capture(&mut self) -> Result<(), String> {
        self.drain_after_capture(ASR_ADAPTER_DRAIN_TIMEOUT)
            .and_then(|status| match status {
                AdapterDrainStatus::Drained => Ok(()),
                AdapterDrainStatus::TimedOut | AdapterDrainStatus::TimedOutRetained => {
                    Err("ASR adapter drain timed out.".to_string())
                }
            })
    }

    pub(super) fn drain_after_capture(
        &mut self,
        timeout: Duration,
    ) -> Result<AdapterDrainStatus, String> {
        self.frames_tx.close();
        match self.wait_for_completion(timeout)? {
            AdapterDrainStatus::Drained => Ok(AdapterDrainStatus::Drained),
            AdapterDrainStatus::TimedOut => {
                self.cancelled.store(true, Ordering::SeqCst);
                match self.close_and_reap_after_cancel() {
                    Ok(()) => Ok(AdapterDrainStatus::TimedOut),
                    Err(error) => {
                        self.cleanup_error = Some(error);
                        Ok(AdapterDrainStatus::TimedOutRetained)
                    }
                }
            }
            AdapterDrainStatus::TimedOutRetained => {
                unreachable!("only drain_after_capture retains adapter ownership")
            }
        }
    }

    fn wait_for_completion(&mut self, timeout: Duration) -> Result<AdapterDrainStatus, String> {
        let completed_rx = self
            .completed_rx
            .as_ref()
            .ok_or_else(|| "ASR adapter completion was already consumed.".to_string())?;
        match completed_rx.recv_timeout(timeout) {
            Ok(()) => {
                self.completed_rx.take();
                if let Some(worker) = self.worker.take() {
                    join_worker(worker)?;
                }
                Ok(AdapterDrainStatus::Drained)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(AdapterDrainStatus::TimedOut),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(worker) = self.worker.take() {
                    join_worker(worker)?;
                }
                Err("ASR adapter stopped without reporting completion.".to_string())
            }
        }
    }

    fn close_and_reap_after_cancel(&mut self) -> Result<(), String> {
        self.frames_tx.close();
        match self.wait_for_completion(ASR_ADAPTER_CANCEL_GRACE) {
            Ok(AdapterDrainStatus::Drained) => Ok(()),
            Ok(AdapterDrainStatus::TimedOut) => self.hand_off_worker(),
            Ok(AdapterDrainStatus::TimedOutRetained) => {
                unreachable!("only drain_after_capture retains adapter ownership")
            }
            Err(error) => Err(error),
        }
    }

    fn hand_off_worker(&mut self) -> Result<(), String> {
        let (Some(worker), Some(completed_rx)) = (self.worker.take(), self.completed_rx.take())
        else {
            return Ok(());
        };
        let payload = Arc::new(Mutex::new(Some(AdapterReapPayload {
            worker,
            completed_rx,
        })));
        let reaper_payload = Arc::clone(&payload);
        match spawn_adapter_reaper(move || {
            let payload = reaper_payload
                .lock()
                .expect("ASR adapter reaper payload poisoned")
                .take()
                .expect("ASR adapter reaper owns one payload");
            let _ = payload.completed_rx.recv();
            if let Err(error) = join_worker(payload.worker) {
                crate::stt::log_yap(&format!("live ASR adapter reaper failed: {error}"));
            }
        }) {
            Ok(_) => Ok(()),
            Err(error) => {
                let payload = payload
                    .lock()
                    .map_err(|_| "ASR adapter reaper payload became unavailable.".to_string())?
                    .take()
                    .expect("failed reaper spawn leaves the adapter payload owned locally");
                self.worker = Some(payload.worker);
                self.completed_rx = Some(payload.completed_rx);
                Err(error)
            }
        }
    }

    pub(super) fn cancel_and_join(&mut self) -> Result<(), String> {
        self.cancelled.store(true, Ordering::SeqCst);
        self.close_and_reap_after_cancel()
    }

    pub(super) fn take_cleanup_error(&mut self) -> Option<String> {
        self.cleanup_error.take()
    }

    pub(super) fn retains_cleanup_ownership(&self) -> bool {
        self.worker.is_some() || self.completed_rx.is_some()
    }

    #[cfg(test)]
    pub(super) fn retains_cleanup_ownership_for_test(&self) -> bool {
        self.worker.is_some() && self.completed_rx.is_some()
    }
}

impl PendingAsrAdapter {
    pub(super) fn new() -> Self {
        let (frames_tx, frames_rx) = bounded_sink(SinkKind::LocalAsr, LOCAL_ASR_QUEUE_CAPACITY);
        Self {
            frames_tx,
            frames_rx: Some(frames_rx),
        }
    }

    pub(super) fn sink(&self) -> BoundedSink<PreparedFrame> {
        self.frames_tx.clone()
    }

    pub(super) fn start(
        self,
        samples_tx: mpsc::SyncSender<StreamMessage>,
        session: u64,
    ) -> SessionAsrAdapter {
        self.start_with_completion_hook(samples_tx, session, || {})
    }

    fn start_with_completion_hook<F>(
        mut self,
        samples_tx: mpsc::SyncSender<StreamMessage>,
        session: u64,
        completion_hook: F,
    ) -> SessionAsrAdapter
    where
        F: FnOnce() + Send + 'static,
    {
        let frames_rx = self
            .frames_rx
            .take()
            .expect("pending ASR adapter starts one worker");
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let (completed_tx, completed_rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            run_session_asr_adapter_worker(frames_rx, samples_tx, session, worker_cancelled);
            completion_hook();
            let _ = completed_tx.send(());
        });
        SessionAsrAdapter {
            frames_tx: self.frames_tx,
            cancelled,
            completed_rx: Some(completed_rx),
            worker: Some(worker),
            cleanup_error: None,
        }
    }
}

fn spawn_adapter_reaper<F>(run: F) -> Result<JoinHandle<()>, String>
where
    F: FnOnce() + Send + 'static,
{
    #[cfg(test)]
    if FAIL_NEXT_REAPER_SPAWN.with(|fail| fail.replace(false)) {
        return Err("ASR adapter reaper could not start (synthetic failure).".to_string());
    }
    thread::Builder::new()
        .name("live-asr-adapter-reaper".to_string())
        .spawn(run)
        .map_err(|error| format!("ASR adapter reaper could not start: {error}"))
}

#[cfg(test)]
pub(super) fn set_reaper_spawn_failure_for_test() {
    FAIL_NEXT_REAPER_SPAWN.with(|fail| fail.set(true));
}

fn run_session_asr_adapter_worker(
    frames_rx: BoundedReceiver<PreparedFrame>,
    samples_tx: mpsc::SyncSender<StreamMessage>,
    session: u64,
    cancelled: Arc<AtomicBool>,
) {
    while !cancelled.load(Ordering::Acquire) {
        let frame = match frames_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(frame) => frame,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let mut message = StreamMessage::from_prepared(session, frame);
        loop {
            if cancelled.load(Ordering::Acquire) {
                return;
            }
            match samples_tx.try_send(message) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(returned)) => {
                    message = returned;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(mpsc::TrySendError::Disconnected(_)) => return,
            }
        }
    }
}
