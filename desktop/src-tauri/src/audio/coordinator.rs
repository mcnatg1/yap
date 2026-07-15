use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant};

use crate::audio::capture::CapturePacket;
use crate::audio::frame::{PreparedFrame, TrackConfigurationRevision};
use crate::audio::preprocess::{downmix_to_mono, rms_level, AudioLevelNormalizer, LinearResampler};
use crate::audio::session::{SessionId, TrackId};
use crate::audio::timeline::{
    ClockMappingRevision, LossAccumulator, RecordingInput, RecordingRevisionTransition, Timeline,
    TryDrain,
};

pub const RECORDING_QUEUE_CAPACITY: usize = 128;
pub const LOCAL_ASR_QUEUE_CAPACITY: usize = 64;
pub const EVIDENCE_QUEUE_CAPACITY: usize = 32;
pub const SERVER_TRANSPORT_QUEUE_CAPACITY: usize = 64;
const TARGET_SAMPLE_RATE_HZ: u32 = 16_000;

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
pub(crate) enum SinkGatePhase {
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

struct SinkCompletionGate {
    phase: SinkGatePhase,
    degradation: Option<String>,
}

impl Default for SinkCompletionGate {
    fn default() -> Self {
        Self {
            phase: SinkGatePhase::Accepting,
            degradation: None,
        }
    }
}

struct BoundedSinkState<T> {
    sender: Mutex<Option<mpsc::SyncSender<T>>>,
    queue_capacity: usize,
    accepted_frames: AtomicU64,
    dropped_frames: AtomicU64,
    queued_frames: AtomicUsize,
    published_frames: AtomicUsize,
    high_water_mark: AtomicUsize,
    closed: AtomicBool,
    close_count: AtomicUsize,
    completion: Mutex<SinkCompletionGate>,
    #[cfg(test)]
    after_publish_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    after_receive_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    before_completion_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

#[derive(Clone)]
pub struct BoundedSink<T> {
    kind: SinkKind,
    state: Arc<BoundedSinkState<T>>,
}

pub struct BoundedReceiver<T> {
    receiver: mpsc::Receiver<T>,
    state: Arc<BoundedSinkState<T>>,
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

    fn close_with_error(&self, error: &str) {
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
    fn set_after_publish_hook_for_test(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.state.after_publish_hook.lock().unwrap() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_before_completion_hook_for_test(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.state.before_completion_hook.lock().unwrap() = Some(hook);
    }

    #[cfg(test)]
    fn queued_frames_for_test(&self) -> usize {
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

impl<T> BoundedReceiver<T> {
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, mpsc::RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.claim_published_frame() {
                match self
                    .receiver
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                {
                    Ok(item) => {
                        #[cfg(test)]
                        self.run_after_receive_hook_for_test();
                        return Ok(item);
                    }
                    Err(error) => {
                        self.restore_claimed_frame();
                        return Err(error);
                    }
                }
            }
            if self.state.closed.load(Ordering::Acquire) {
                return Err(mpsc::RecvTimeoutError::Disconnected);
            }
            if Instant::now() >= deadline {
                return Err(mpsc::RecvTimeoutError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[cfg(test)]
    fn set_after_receive_hook_for_test(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.state.after_receive_hook.lock().unwrap() = Some(hook);
    }

    fn claim_published_frame(&self) -> bool {
        let mut published = self.state.published_frames.load(Ordering::Acquire);
        loop {
            if published == 0 {
                return false;
            }
            match self.state.published_frames.compare_exchange_weak(
                published,
                published - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let result = self.state.queued_frames.fetch_update(
                        Ordering::AcqRel,
                        Ordering::Acquire,
                        |queued| queued.checked_sub(1),
                    );
                    debug_assert!(result.is_ok(), "a published frame must reserve queue depth");
                    return true;
                }
                Err(observed) => published = observed,
            }
        }
    }

    fn restore_claimed_frame(&self) {
        self.state.queued_frames.fetch_add(1, Ordering::AcqRel);
        self.state.published_frames.fetch_add(1, Ordering::Release);
    }

    #[cfg(test)]
    fn run_after_receive_hook_for_test(&self) {
        if let Some(hook) = self.state.after_receive_hook.lock().unwrap().as_ref() {
            hook();
        }
    }
}

impl<T> Drop for BoundedReceiver<T> {
    fn drop(&mut self) {
        self.state.queued_frames.store(0, Ordering::Release);
        self.state.published_frames.store(0, Ordering::Release);
        self.state.closed.store(true, Ordering::Release);
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionEvent {
    TrackConfigured(u32),
    ClockMapped(u32),
    ResamplerReset {
        track_revision: u32,
        clock_revision: u32,
    },
}

const PENDING_LOSS_CAPACITY: usize = 64;
const LOSS_DRAIN_ATTEMPT_LIMIT: usize = PENDING_LOSS_CAPACITY;

struct PendingLosses {
    snapshots: VecDeque<crate::audio::timeline::LossSnapshot>,
}

impl PendingLosses {
    fn new() -> Self {
        Self {
            snapshots: VecDeque::with_capacity(PENDING_LOSS_CAPACITY),
        }
    }

    fn push(&mut self, loss: crate::audio::timeline::LossSnapshot) -> bool {
        if self.snapshots.len() < PENDING_LOSS_CAPACITY {
            self.snapshots.push_back(loss);
            return true;
        }
        let Some(previous) = self.snapshots.back_mut() else {
            return false;
        };
        let Some(previous_end) = previous
            .first_source_position_frames
            .checked_add(previous.dropped_frames)
        else {
            return false;
        };
        let Some(merged_frames) = previous.dropped_frames.checked_add(loss.dropped_frames) else {
            return false;
        };
        if previous.cause != loss.cause
            || previous_end != loss.first_source_position_frames
            || loss.generation <= previous.generation
        {
            return false;
        }
        previous.dropped_frames = merged_frames;
        previous.generation = loss.generation;
        true
    }

    fn front(&self) -> Option<&crate::audio::timeline::LossSnapshot> {
        self.snapshots.front()
    }

    fn pop_front(&mut self) -> Option<crate::audio::timeline::LossSnapshot> {
        self.snapshots.pop_front()
    }

    fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.snapshots.len()
    }

    fn clear(&mut self) {
        self.snapshots.clear();
    }
}

pub struct Coordinator {
    track_id: TrackId,
    ports: CoordinatorPorts,
    timeline: Timeline,
    capture_config: Option<(u16, u32)>,
    track_revision: u32,
    clock_revision: u32,
    last_session_end_ms: u64,
    resampler: Option<LinearResampler>,
    level_normalizer: AudioLevelNormalizer,
    pending_losses: PendingLosses,
    #[cfg(test)]
    revision_events: Vec<RevisionEvent>,
    #[cfg(test)]
    loss_pending_hook: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Coordinator {
    pub fn new(session_id: SessionId, track_id: TrackId, ports: CoordinatorPorts) -> Self {
        Self {
            timeline: Timeline::new(session_id.clone()),
            track_id,
            ports,
            capture_config: None,
            track_revision: 0,
            clock_revision: 0,
            last_session_end_ms: 0,
            resampler: None,
            level_normalizer: AudioLevelNormalizer::new(),
            pending_losses: PendingLosses::new(),
            #[cfg(test)]
            revision_events: Vec::new(),
            #[cfg(test)]
            loss_pending_hook: None,
        }
    }

    pub fn consume(
        &mut self,
        packet: &CapturePacket,
        losses: &LossAccumulator,
    ) -> Result<f32, String> {
        let result = self.consume_inner(packet, losses);
        if let Err(error) = &result {
            self.ports.recording.degrade(error);
        }
        result
    }

    fn consume_inner(
        &mut self,
        packet: &CapturePacket,
        losses: &LossAccumulator,
    ) -> Result<f32, String> {
        self.drain_losses(losses)?;
        if self.capture_config.is_none() {
            let first_source_position_frames = self
                .pending_losses
                .front()
                .map_or(packet.source_position_frames, |loss| {
                    loss.first_source_position_frames
                });
            self.ensure_configuration(packet, first_source_position_frames, 0)?;
            while let Some(loss) = self.pending_losses.pop_front() {
                self.apply_loss(loss)?;
            }
        } else {
            while let Some(loss) = self.pending_losses.pop_front() {
                self.apply_loss(loss)?;
            }
            self.ensure_configuration(
                packet,
                packet.source_position_frames,
                self.last_session_end_ms,
            )?;
        }

        let channels = usize::from(packet.channels);
        if channels == 0
            || packet.samples.is_empty()
            || !packet.samples.len().is_multiple_of(channels)
        {
            return Err("Invalid captured audio packet.".into());
        }
        let frame_count = u64::try_from(packet.samples.len() / channels)
            .map_err(|_| "Captured audio packet is too large.")?;
        let mut metadata = self
            .timeline
            .frame(
                &self.track_id,
                packet.source_position_frames,
                frame_count,
                packet.channels,
            )
            .map_err(|error| format!("Capture timeline frame failed: {error}"))?;
        self.last_session_end_ms = metadata
            .start_ms
            .checked_add(u64::from(metadata.duration_ms))
            .ok_or_else(|| "Capture timeline frame overflowed.".to_string())?;

        let mono = downmix_to_mono(&packet.samples, channels);
        let level = self.level_normalizer.normalized_level(rms_level(&mono));
        let samples = self
            .resampler
            .as_mut()
            .expect("resampler is initialized with the capture configuration")
            .push(&mono);
        if samples.is_empty() {
            return Ok(level);
        }
        metadata.sample_rate_hz = TARGET_SAMPLE_RATE_HZ;
        metadata.channels = 1;
        metadata.sample_count = samples.len();
        let prepared = PreparedFrame {
            metadata,
            samples: Arc::from(samples),
        };
        let _ = self
            .ports
            .recording
            .try_send(RecordingInput::PreparedFrame(prepared.clone()));
        for sink in [
            self.ports.local_asr.as_ref(),
            self.ports.speaker_evidence.as_ref(),
            self.ports.server_transport.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let _ = sink.try_send(prepared.clone());
        }
        Ok(level)
    }

    pub fn poll_losses(&mut self, losses: &LossAccumulator) -> Result<(), String> {
        let result = self.poll_losses_inner(losses);
        if let Err(error) = &result {
            self.ports.recording.degrade(error);
        }
        result
    }

    fn poll_losses_inner(&mut self, losses: &LossAccumulator) -> Result<(), String> {
        self.drain_losses(losses)?;
        if self.capture_config.is_some() {
            while let Some(loss) = self.pending_losses.pop_front() {
                self.apply_loss(loss)?;
            }
        }
        Ok(())
    }

    pub fn consume_loss(
        &mut self,
        loss: crate::audio::timeline::LossSnapshot,
    ) -> Result<(), String> {
        let result = self.consume_loss_inner(loss);
        if let Err(error) = &result {
            self.ports.recording.degrade(error);
        }
        result
    }

    fn consume_loss_inner(
        &mut self,
        loss: crate::audio::timeline::LossSnapshot,
    ) -> Result<(), String> {
        if self.capture_config.is_none() {
            if !self.pending_losses.push(loss) {
                self.ports
                    .recording
                    .degrade("recording pending-loss capacity exhausted");
            }
            return Ok(());
        }
        self.apply_loss(loss)
    }

    fn apply_loss(&mut self, loss: crate::audio::timeline::LossSnapshot) -> Result<(), String> {
        let gap = self
            .timeline
            .gap(&self.track_id, loss)
            .map_err(|error| format!("Capture timeline gap failed: {error}"))?;
        self.last_session_end_ms = gap
            .start_ms
            .checked_add(u64::from(gap.duration_ms))
            .ok_or_else(|| "Capture timeline gap overflowed.".to_string())?;
        let _ = self.ports.recording.try_send(RecordingInput::Gap(gap));
        Ok(())
    }

    pub fn close(&mut self) {
        if self.pending_losses.is_empty() {
            self.ports.recording.close();
        } else {
            self.pending_losses.clear();
            self.ports
                .recording
                .degrade("recording closed with unpublished pre-configuration loss");
            self.ports.recording.close();
        }
        for sink in [
            self.ports.local_asr.as_ref(),
            self.ports.speaker_evidence.as_ref(),
            self.ports.server_transport.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            sink.close();
        }
    }

    pub fn close_sink(&mut self, kind: SinkKind) {
        if kind == SinkKind::Recording {
            self.ports
                .recording
                .close_with_error("recording sink closed before capture completion");
        } else if let Some(sink) = self.frame_sink(kind) {
            sink.close();
        }
    }

    pub fn outcome(&self, kind: SinkKind) -> Option<SinkOutcome> {
        match kind {
            SinkKind::Recording => Some(self.ports.recording.outcome()),
            _ => self.frame_sink(kind).map(BoundedSink::outcome),
        }
    }

    pub(crate) fn degrade_recording(&self, error: &str) {
        self.ports.recording.degrade(error);
    }

    pub fn outcomes(&self) -> Vec<SinkOutcome> {
        [
            SinkKind::Recording,
            SinkKind::LocalAsr,
            SinkKind::SpeakerEvidence,
            SinkKind::ServerTransport,
        ]
        .into_iter()
        .filter_map(|kind| self.outcome(kind))
        .collect()
    }

    pub fn high_water_mark(&self, kind: SinkKind) -> Option<usize> {
        match kind {
            SinkKind::Recording => Some(self.ports.recording.high_water_mark()),
            _ => self.frame_sink(kind).map(BoundedSink::high_water_mark),
        }
    }

    pub fn close_count(&self, kind: SinkKind) -> usize {
        match kind {
            SinkKind::Recording => self.ports.recording.close_count(),
            _ => self.frame_sink(kind).map_or(0, BoundedSink::close_count),
        }
    }

    #[cfg(test)]
    pub fn revision_events(&self) -> &[RevisionEvent] {
        &self.revision_events
    }

    #[cfg(test)]
    fn set_loss_pending_hook_for_test(&mut self, hook: Arc<dyn Fn() + Send + Sync>) {
        self.loss_pending_hook = Some(hook);
    }

    fn frame_sink(&self, kind: SinkKind) -> Option<&BoundedSink<PreparedFrame>> {
        match kind {
            SinkKind::Recording => None,
            SinkKind::LocalAsr => self.ports.local_asr.as_ref(),
            SinkKind::SpeakerEvidence => self.ports.speaker_evidence.as_ref(),
            SinkKind::ServerTransport => self.ports.server_transport.as_ref(),
        }
    }

    fn drain_losses(&mut self, losses: &LossAccumulator) -> Result<(), String> {
        for _ in 0..LOSS_DRAIN_ATTEMPT_LIMIT {
            match losses.try_drain() {
                Ok(TryDrain::Snapshot(loss)) => {
                    if !self.pending_losses.push(loss) {
                        self.ports
                            .recording
                            .degrade("recording pending-loss capacity exhausted");
                    }
                }
                Ok(TryDrain::Pending) => {
                    #[cfg(test)]
                    if let Some(hook) = self.loss_pending_hook.take() {
                        hook();
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(TryDrain::Empty) => return Ok(()),
                Err(error) => return Err(format!("Capture loss timing failed: {error}")),
            }
        }
        Err("Capture loss drain did not quiesce.".into())
    }

    fn ensure_configuration(
        &mut self,
        packet: &CapturePacket,
        source_position_frames: u64,
        session_time_ms: u64,
    ) -> Result<(), String> {
        let configuration = (packet.channels, packet.sample_rate_hz);
        if configuration.0 == 0 || configuration.1 == 0 {
            return Err("Invalid microphone configuration.".into());
        }
        if self.capture_config == Some(configuration) {
            return Ok(());
        }
        self.track_revision = self
            .track_revision
            .checked_add(1)
            .ok_or_else(|| "Capture track revision overflowed.".to_string())?;
        self.clock_revision = self
            .clock_revision
            .checked_add(1)
            .ok_or_else(|| "Capture clock revision overflowed.".to_string())?;
        let track_configuration = TrackConfigurationRevision::new(
            self.track_id.clone(),
            self.track_revision,
            session_time_ms,
            packet.sample_rate_hz,
        )
        .map_err(|error| format!("Capture track configuration failed: {error}"))?;
        let clock_mapping = ClockMappingRevision::new(
            self.track_id.clone(),
            self.clock_revision,
            source_position_frames,
            session_time_ms,
        )
        .map_err(|error| format!("Capture clock mapping failed: {error}"))?;
        let recording_transition =
            RecordingRevisionTransition::new(track_configuration.clone(), clock_mapping.clone())
                .map_err(|error| format!("Capture recording revision failed: {error}"))?;
        self.timeline
            .configure_track(track_configuration)
            .map_err(|error| format!("Capture track configuration failed: {error}"))?;
        self.timeline
            .map_clock(clock_mapping.clone())
            .map_err(|error| format!("Capture clock mapping failed: {error}"))?;
        let _ = self
            .ports
            .recording
            .try_send(RecordingInput::RevisionTransition(recording_transition));
        #[cfg(test)]
        {
            self.revision_events
                .push(RevisionEvent::TrackConfigured(self.track_revision));
            self.revision_events
                .push(RevisionEvent::ClockMapped(self.clock_revision));
        }
        self.capture_config = Some(configuration);
        self.resampler = Some(LinearResampler::new(
            packet.sample_rate_hz,
            TARGET_SAMPLE_RATE_HZ,
        ));
        #[cfg(test)]
        self.revision_events.push(RevisionEvent::ResamplerReset {
            track_revision: self.track_revision,
            clock_revision: self.clock_revision,
        });
        Ok(())
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests;
