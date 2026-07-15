use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize},
    mpsc, Arc, Mutex,
};

use crate::audio::frame::PreparedFrame;
use crate::audio::timeline::RecordingInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SinkKind {
    Recording,
    LocalAsr,
    SpeakerEvidence,
    ServerTransport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkOutcome {
    pub kind: SinkKind,
    pub accepted_frames: u64,
    pub dropped_frames: u64,
    pub closed: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkSendError {
    Full,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SinkGatePhase {
    Accepting,
    Completing,
    Published,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SinkDegradeResult {
    Accepted,
    CompletionInProgress,
    Published,
}

pub(super) struct SinkCompletionGate {
    pub(super) phase: SinkGatePhase,
    pub(super) degradation: Option<String>,
}

impl Default for SinkCompletionGate {
    fn default() -> Self {
        Self {
            phase: SinkGatePhase::Accepting,
            degradation: None,
        }
    }
}

pub(super) struct BoundedSinkState<T> {
    pub(super) sender: Mutex<Option<mpsc::SyncSender<T>>>,
    pub(super) queue_capacity: usize,
    pub(super) accepted_frames: AtomicU64,
    pub(super) dropped_frames: AtomicU64,
    pub(super) queued_frames: AtomicUsize,
    pub(super) published_frames: AtomicUsize,
    pub(super) high_water_mark: AtomicUsize,
    pub(super) closed: AtomicBool,
    pub(super) close_count: AtomicUsize,
    pub(super) completion: Mutex<SinkCompletionGate>,
    #[cfg(test)]
    pub(super) after_publish_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    pub(super) after_receive_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    pub(super) before_completion_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

#[derive(Clone)]
pub struct BoundedSink<T> {
    pub(super) kind: SinkKind,
    pub(super) state: Arc<BoundedSinkState<T>>,
}

pub struct BoundedReceiver<T> {
    pub(super) receiver: mpsc::Receiver<T>,
    pub(super) state: Arc<BoundedSinkState<T>>,
}

pub struct CoordinatorPorts {
    pub recording: BoundedSink<RecordingInput>,
    pub local_asr: Option<BoundedSink<PreparedFrame>>,
    pub speaker_evidence: Option<BoundedSink<PreparedFrame>>,
    pub server_transport: Option<BoundedSink<PreparedFrame>>,
}

pub fn bounded_sink<T>(kind: SinkKind, capacity: usize) -> (BoundedSink<T>, BoundedReceiver<T>) {
    let (sender, receiver) = mpsc::sync_channel(capacity);
    let state = Arc::new(BoundedSinkState {
        sender: Mutex::new(Some(sender)),
        queue_capacity: capacity,
        accepted_frames: AtomicU64::new(0),
        dropped_frames: AtomicU64::new(0),
        queued_frames: AtomicUsize::new(0),
        published_frames: AtomicUsize::new(0),
        high_water_mark: AtomicUsize::new(0),
        closed: AtomicBool::new(false),
        close_count: AtomicUsize::new(0),
        completion: Mutex::new(SinkCompletionGate::default()),
        #[cfg(test)]
        after_publish_hook: Mutex::new(None),
        #[cfg(test)]
        after_receive_hook: Mutex::new(None),
        #[cfg(test)]
        before_completion_hook: Mutex::new(None),
    });
    (
        BoundedSink {
            kind,
            state: Arc::clone(&state),
        },
        BoundedReceiver { receiver, state },
    )
}
