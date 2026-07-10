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
    ClockMappingRevision, LossAccumulator, Timeline, TimelineEvent, TryDrain,
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
    error: Mutex<Option<String>>,
    #[cfg(test)]
    after_publish_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    after_receive_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
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
    pub recording: BoundedSink<PreparedFrame>,
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
        error: Mutex::new(None),
        #[cfg(test)]
        after_publish_hook: Mutex::new(None),
        #[cfg(test)]
        after_receive_hook: Mutex::new(None),
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
        let sender = match self.state.sender.lock() {
            Ok(sender) => sender,
            Err(_) => {
                self.record_drop("sink state became unavailable");
                return Err(SinkSendError::Closed);
            }
        };
        let Some(sender) = sender.as_ref() else {
            self.record_drop("sink closed");
            return Err(SinkSendError::Closed);
        };
        if self.state.closed.load(Ordering::Acquire) {
            self.record_drop("sink closed");
            return Err(SinkSendError::Closed);
        }
        let Some(reserved_queued) = self.reserve_queue_slot() else {
            self.record_drop("sink queue is full");
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
                self.record_drop("sink queue is full");
                Err(SinkSendError::Full)
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.state.queued_frames.store(0, Ordering::Release);
                self.state.published_frames.store(0, Ordering::Release);
                self.state.closed.store(true, Ordering::Release);
                self.record_drop("sink receiver disconnected");
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

    pub fn outcome(&self) -> SinkOutcome {
        SinkOutcome {
            kind: self.kind,
            accepted_frames: self.state.accepted_frames.load(Ordering::Acquire),
            dropped_frames: self.state.dropped_frames.load(Ordering::Acquire),
            closed: self.state.closed.load(Ordering::Acquire),
            error: self.state.error.lock().ok().and_then(|error| error.clone()),
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

    fn record_drop(&self, error: &str) {
        self.state.dropped_frames.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut current) = self.state.error.lock() {
            current.get_or_insert_with(|| error.to_string());
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionEvent {
    TrackConfigured(u32),
    ClockMapped(u32),
    ResamplerReset {
        track_revision: u32,
        clock_revision: u32,
    },
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
    pending_losses: Vec<crate::audio::timeline::LossSnapshot>,
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
            pending_losses: Vec::new(),
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
        let pending_losses = self.drain_losses(losses)?;
        if self.capture_config.is_none() {
            let first_source_position_frames = pending_losses
                .first()
                .map_or(packet.source_position_frames, |loss| {
                    loss.first_source_position_frames
                });
            self.ensure_configuration(packet, first_source_position_frames, 0)?;
            for loss in pending_losses {
                self.apply_loss(loss)?;
            }
        } else {
            for loss in &pending_losses {
                self.apply_loss(*loss)?;
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
        let _ = self.ports.recording.try_send(prepared.clone());
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
        for loss in self.drain_losses(losses)? {
            self.consume_loss(loss)?;
        }
        Ok(())
    }

    pub fn consume_loss(
        &mut self,
        loss: crate::audio::timeline::LossSnapshot,
    ) -> Result<(), String> {
        if self.capture_config.is_none() {
            self.pending_losses.push(loss);
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
        Ok(())
    }

    pub fn close(&mut self) {
        self.ports.recording.close();
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
        if let Some(sink) = self.sink(kind) {
            sink.close();
        }
    }

    pub fn outcome(&self, kind: SinkKind) -> Option<SinkOutcome> {
        self.sink(kind).map(BoundedSink::outcome)
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
        self.sink(kind).map(BoundedSink::high_water_mark)
    }

    pub fn close_count(&self, kind: SinkKind) -> usize {
        self.sink(kind).map_or(0, BoundedSink::close_count)
    }

    pub fn timeline_events(&self) -> &[TimelineEvent] {
        self.timeline.events()
    }

    pub fn revision_events(&self) -> &[RevisionEvent] {
        &self.revision_events
    }

    #[cfg(test)]
    fn set_loss_pending_hook_for_test(&mut self, hook: Arc<dyn Fn() + Send + Sync>) {
        self.loss_pending_hook = Some(hook);
    }

    fn sink(&self, kind: SinkKind) -> Option<&BoundedSink<PreparedFrame>> {
        match kind {
            SinkKind::Recording => Some(&self.ports.recording),
            SinkKind::LocalAsr => self.ports.local_asr.as_ref(),
            SinkKind::SpeakerEvidence => self.ports.speaker_evidence.as_ref(),
            SinkKind::ServerTransport => self.ports.server_transport.as_ref(),
        }
    }

    fn drain_losses(
        &mut self,
        losses: &LossAccumulator,
    ) -> Result<Vec<crate::audio::timeline::LossSnapshot>, String> {
        let mut drained = std::mem::take(&mut self.pending_losses);
        loop {
            match losses.try_drain() {
                Ok(TryDrain::Snapshot(loss)) => drained.push(loss),
                Ok(TryDrain::Pending) => {
                    #[cfg(test)]
                    if let Some(hook) = self.loss_pending_hook.take() {
                        hook();
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(TryDrain::Empty) => return Ok(drained),
                Err(error) => return Err(format!("Capture loss timing failed: {error}")),
            }
        }
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
        self.timeline
            .configure_track(
                TrackConfigurationRevision::new(
                    self.track_id.clone(),
                    self.track_revision,
                    session_time_ms,
                    packet.sample_rate_hz,
                )
                .map_err(|error| format!("Capture track configuration failed: {error}"))?,
            )
            .map_err(|error| format!("Capture track configuration failed: {error}"))?;
        self.revision_events
            .push(RevisionEvent::TrackConfigured(self.track_revision));
        self.timeline
            .map_clock(
                ClockMappingRevision::new(
                    self.track_id.clone(),
                    self.clock_revision,
                    source_position_frames,
                    session_time_ms,
                )
                .map_err(|error| format!("Capture clock mapping failed: {error}"))?,
            )
            .map_err(|error| format!("Capture clock mapping failed: {error}"))?;
        self.revision_events
            .push(RevisionEvent::ClockMapped(self.clock_revision));
        self.capture_config = Some(configuration);
        self.resampler = Some(LinearResampler::new(
            packet.sample_rate_hz,
            TARGET_SAMPLE_RATE_HZ,
        ));
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
mod tests {
    use std::sync::{Arc, Barrier};
    use std::time::{Duration, Instant};

    use crate::audio::capture::CapturePacket;
    use crate::audio::frame::GapCause;
    use crate::audio::session::{SessionId, TrackId};
    use crate::audio::timeline::{LossAccumulator, TimelineEvent};

    use super::{
        bounded_sink, Coordinator, CoordinatorPorts, RevisionEvent, SinkKind,
        EVIDENCE_QUEUE_CAPACITY, LOCAL_ASR_QUEUE_CAPACITY, RECORDING_QUEUE_CAPACITY,
        SERVER_TRANSPORT_QUEUE_CAPACITY,
    };

    fn session() -> SessionId {
        SessionId::new("test-session").unwrap()
    }

    fn track() -> TrackId {
        TrackId::new("test-microphone").unwrap()
    }

    fn packet(position: u64) -> CapturePacket {
        CapturePacket {
            source_position_frames: position,
            channels: 2,
            sample_rate_hz: 48_000,
            samples: [0.25_f32, -0.25].into_iter().cycle().take(960).collect(),
        }
    }

    fn ports(
        recording_capacity: usize,
        local_asr_capacity: Option<usize>,
    ) -> (
        CoordinatorPorts,
        super::BoundedReceiver<crate::audio::frame::PreparedFrame>,
        Option<super::BoundedReceiver<crate::audio::frame::PreparedFrame>>,
    ) {
        let (recording, recording_rx) = bounded_sink(SinkKind::Recording, recording_capacity);
        let (local_asr, local_asr_rx) = local_asr_capacity
            .map(|capacity| bounded_sink(SinkKind::LocalAsr, capacity))
            .map_or((None, None), |(sink, receiver)| {
                (Some(sink), Some(receiver))
            });
        (
            CoordinatorPorts {
                recording,
                local_asr,
                speaker_evidence: None,
                server_transport: None,
            },
            recording_rx,
            local_asr_rx,
        )
    }

    #[test]
    fn recording_continues_when_local_asr_is_absent() {
        let (ports, recording_rx, _) = ports(2, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();

        coordinator.consume(&packet(0), &losses).unwrap();

        let frame = recording_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(frame.metadata.sample_rate_hz, 16_000);
        assert_eq!(frame.metadata.channels, 1);
        assert!(coordinator.outcome(SinkKind::LocalAsr).is_none());
    }

    #[test]
    fn stalled_asr_does_not_block_recording_or_callback_intake() {
        let (ports, recording_rx, local_asr_rx) = ports(2, Some(1));
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();
        let started = Instant::now();

        coordinator.consume(&packet(0), &losses).unwrap();
        coordinator.consume(&packet(480), &losses).unwrap();

        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(
            recording_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .metadata
                .sequence,
            0
        );
        assert_eq!(
            recording_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .metadata
                .sequence,
            1
        );
        assert_eq!(
            local_asr_rx
                .unwrap()
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .metadata
                .sequence,
            0
        );
        assert_eq!(
            coordinator
                .outcome(SinkKind::LocalAsr)
                .unwrap()
                .dropped_frames,
            1
        );
    }

    #[test]
    fn one_sink_failure_does_not_close_other_sinks() {
        let (ports, recording_rx, local_asr_rx) = ports(1, Some(1));
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();
        coordinator.close_sink(SinkKind::LocalAsr);
        drop(local_asr_rx);

        coordinator.consume(&packet(0), &losses).unwrap();

        assert_eq!(
            recording_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .metadata
                .sequence,
            0
        );
        assert!(!coordinator.outcome(SinkKind::Recording).unwrap().closed);
        assert!(coordinator.outcome(SinkKind::LocalAsr).unwrap().closed);
    }

    #[test]
    fn finalization_closes_every_sink_exactly_once() {
        let (ports, _, _) = ports(1, Some(1));
        let mut coordinator = Coordinator::new(session(), track(), ports);

        coordinator.close();
        coordinator.close();

        for outcome in coordinator.outcomes() {
            assert!(outcome.closed);
            assert_eq!(coordinator.close_count(outcome.kind), 1);
        }
    }

    #[test]
    fn composed_result_marks_only_the_failed_or_degraded_sinks() {
        let (ports, recording_rx, local_asr_rx) = ports(1, Some(1));
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();
        coordinator.close_sink(SinkKind::LocalAsr);
        drop(local_asr_rx);

        coordinator.consume(&packet(0), &losses).unwrap();
        drop(recording_rx);

        let recording = coordinator.outcome(SinkKind::Recording).unwrap();
        let asr = coordinator.outcome(SinkKind::LocalAsr).unwrap();
        assert_eq!(recording.dropped_frames, 0);
        assert_eq!(recording.error, None);
        assert!(asr.closed);
        assert!(asr.error.is_some());
    }

    #[test]
    fn queue_capacities_and_high_water_marks_are_visible() {
        assert_eq!(RECORDING_QUEUE_CAPACITY, 128);
        assert_eq!(LOCAL_ASR_QUEUE_CAPACITY, 64);
        assert_eq!(EVIDENCE_QUEUE_CAPACITY, 32);
        assert_eq!(SERVER_TRANSPORT_QUEUE_CAPACITY, 64);

        let (ports, _recording_rx, _) = ports(2, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();
        coordinator.consume(&packet(0), &losses).unwrap();
        coordinator.consume(&packet(480), &losses).unwrap();

        assert_eq!(coordinator.high_water_mark(SinkKind::Recording), Some(2));
    }

    #[test]
    fn queue_accounting_reserves_before_publish_and_rolls_back_failed_sends() {
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
        let published = Arc::new(Barrier::new(2));
        let release_sender = Arc::new(Barrier::new(2));
        let pause_once = Arc::new(std::sync::atomic::AtomicBool::new(true));
        sink.set_after_publish_hook_for_test({
            let published = Arc::clone(&published);
            let release_sender = Arc::clone(&release_sender);
            let pause_once = Arc::clone(&pause_once);
            Arc::new(move || {
                if pause_once.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    published.wait();
                    release_sender.wait();
                }
            })
        });
        let sender = sink.clone();
        let worker = std::thread::spawn(move || sender.try_send(1_u8));

        published.wait();
        assert_eq!(receiver.recv_timeout(Duration::from_secs(1)).unwrap(), 1);
        release_sender.wait();
        assert!(worker.join().unwrap().is_ok());
        assert_eq!(sink.queued_frames_for_test(), 0);
        assert_eq!(sink.high_water_mark(), 1);

        assert!(sink.try_send(2).is_ok());
        assert!(matches!(sink.try_send(3), Err(super::SinkSendError::Full)));
        assert_eq!(sink.queued_frames_for_test(), 1);
        assert_eq!(sink.high_water_mark(), 1);
        assert_eq!(sink.outcome().accepted_frames, 2);
        assert_eq!(sink.outcome().dropped_frames, 1);
        drop(receiver);
        assert!(matches!(
            sink.try_send(4),
            Err(super::SinkSendError::Closed)
        ));
        assert_eq!(sink.queued_frames_for_test(), 0);
        assert_eq!(sink.high_water_mark(), 1);

        let (capacity_sink, _capacity_receiver) = bounded_sink(SinkKind::Recording, 2);
        assert!(capacity_sink.try_send(1).is_ok());
        assert!(capacity_sink.try_send(2).is_ok());
        assert!(matches!(
            capacity_sink.try_send(3),
            Err(super::SinkSendError::Full)
        ));
        assert_eq!(capacity_sink.high_water_mark(), 2);
        assert_eq!(capacity_sink.outcome().accepted_frames, 2);
    }

    #[test]
    fn receive_claim_keeps_depth_and_high_water_at_capacity_during_a_send_interleaving() {
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
        sink.try_send(1_u8).unwrap();

        let received = Arc::new(Barrier::new(2));
        let release_receiver = Arc::new(Barrier::new(2));
        receiver.set_after_receive_hook_for_test({
            let received = Arc::clone(&received);
            let release_receiver = Arc::clone(&release_receiver);
            Arc::new(move || {
                received.wait();
                release_receiver.wait();
            })
        });
        let receiver_worker =
            std::thread::spawn(move || receiver.recv_timeout(Duration::from_secs(1)));

        received.wait();
        assert_eq!(sink.queued_frames_for_test(), 0);
        sink.try_send(2_u8).unwrap();
        assert_eq!(sink.queued_frames_for_test(), 1);
        assert_eq!(sink.high_water_mark(), 1);
        release_receiver.wait();

        assert_eq!(receiver_worker.join().unwrap().unwrap(), 1);
        assert_eq!(sink.high_water_mark(), 1);
    }

    #[test]
    fn cloned_producers_preserve_exact_high_water_and_roll_back_failed_sends() {
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 2);
        let start = Arc::new(Barrier::new(3));
        let complete = Arc::new(Barrier::new(3));
        let first = sink.clone();
        let second = sink.clone();
        let first_start = Arc::clone(&start);
        let first_complete = Arc::clone(&complete);
        let first_worker = std::thread::spawn(move || {
            first_start.wait();
            let result = first.try_send(1_u8);
            first_complete.wait();
            result
        });
        let second_start = Arc::clone(&start);
        let second_complete = Arc::clone(&complete);
        let second_worker = std::thread::spawn(move || {
            second_start.wait();
            let result = second.try_send(2_u8);
            second_complete.wait();
            result
        });

        start.wait();
        complete.wait();
        assert!(first_worker.join().unwrap().is_ok());
        assert!(second_worker.join().unwrap().is_ok());
        assert_eq!(sink.high_water_mark(), 2);
        assert_eq!(sink.queued_frames_for_test(), 2);
        let mut received = [
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
        ];
        received.sort_unstable();
        assert_eq!(received, [1, 2]);
        assert_eq!(sink.queued_frames_for_test(), 0);

        let (full_sink, _full_receiver) = bounded_sink(SinkKind::Recording, 1);
        assert!(full_sink.try_send(1_u8).is_ok());
        assert!(matches!(
            full_sink.try_send(2_u8),
            Err(super::SinkSendError::Full)
        ));
        assert_eq!(full_sink.queued_frames_for_test(), 1);
        assert_eq!(full_sink.high_water_mark(), 1);

        let (disconnected_sink, disconnected_receiver) = bounded_sink(SinkKind::Recording, 1);
        drop(disconnected_receiver);
        assert!(matches!(
            disconnected_sink.try_send(1_u8),
            Err(super::SinkSendError::Closed)
        ));
        assert_eq!(disconnected_sink.queued_frames_for_test(), 0);
        assert_eq!(disconnected_sink.high_water_mark(), 0);
    }

    #[test]
    fn source_positions_and_losses_leave_a_timeline_gap() {
        let (ports, recording_rx, _) = ports(1, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = Arc::new(LossAccumulator::new());
        losses.record(0, 480, GapCause::SinkUnavailable);

        coordinator.consume(&packet(480), &losses).unwrap();

        let frame = recording_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(frame.metadata.start_ms, 10);
        assert!(coordinator.timeline_events().iter().any(|event| matches!(event, TimelineEvent::Gap(gap) if gap.start_ms == 0 && gap.duration_ms == 10)));
    }

    #[test]
    fn initial_losses_use_the_first_packet_clock_without_inventing_elapsed_time() {
        let (ports, recording_rx, _) = ports(1, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();
        losses.record(1_000, 480, GapCause::SinkUnavailable);

        coordinator.consume(&packet(1_480), &losses).unwrap();

        let frame = recording_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(frame.metadata.start_ms, 10);
        assert!(coordinator.timeline_events().iter().any(
            |event| matches!(event, TimelineEvent::Gap(gap) if gap.start_ms == 0 && gap.duration_ms == 10 && gap.source_position_frames == 1_000)
        ));
    }

    #[test]
    fn losses_before_a_rate_change_use_the_old_clock_before_new_revisions() {
        let (ports, _recording_rx, _) = ports(2, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();

        coordinator.consume(&packet(0), &losses).unwrap();
        losses.record(480, 480, GapCause::SinkUnavailable);
        coordinator
            .consume(
                &CapturePacket {
                    sample_rate_hz: 44_100,
                    ..packet(960)
                },
                &losses,
            )
            .unwrap();

        let events = coordinator.timeline_events();
        let gap_index = events
            .iter()
            .position(|event| matches!(event, TimelineEvent::Gap(gap) if gap.start_ms == 10 && gap.duration_ms == 10))
            .unwrap();
        let new_configuration_index = events
            .iter()
            .position(|event| matches!(event, TimelineEvent::TrackConfigured(configuration) if configuration.revision == 2 && configuration.effective_at_ms == 20 && configuration.sample_rate_hz == 44_100))
            .unwrap();
        let new_clock_index = events
            .iter()
            .position(|event| matches!(event, TimelineEvent::ClockMapped(clock) if clock.revision == 2 && clock.source_position_frames == 960 && clock.session_time_ms == 20))
            .unwrap();
        assert!(gap_index < new_configuration_index);
        assert!(new_configuration_index < new_clock_index);
        assert!(events.iter().any(
            |event| matches!(event, TimelineEvent::Frame(frame) if frame.sequence == 1 && frame.start_ms == 20)
        ));
    }

    #[test]
    fn pending_loss_registration_blocks_rate_change_until_the_old_clock_applies_it() {
        let (ports, _recording_rx, _) = ports(2, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = Arc::new(LossAccumulator::new());
        coordinator.consume(&packet(0), &losses).unwrap();

        let registration_started = Arc::new(Barrier::new(2));
        let release_registration = Arc::new(Barrier::new(2));
        let loss_writer = {
            let losses = Arc::clone(&losses);
            let registration_started = Arc::clone(&registration_started);
            let release_registration = Arc::clone(&release_registration);
            std::thread::spawn(move || {
                losses.record_with_registration_hooks(
                    480,
                    480,
                    GapCause::SinkUnavailable,
                    || {
                        registration_started.wait();
                    },
                    || {
                        release_registration.wait();
                    },
                );
            })
        };
        registration_started.wait();

        let rate_change_pending = Arc::new(Barrier::new(2));
        coordinator.set_loss_pending_hook_for_test({
            let rate_change_pending = Arc::clone(&rate_change_pending);
            Arc::new(move || {
                rate_change_pending.wait();
            })
        });
        std::thread::scope(|scope| {
            let consume = scope.spawn(|| {
                coordinator.consume(
                    &CapturePacket {
                        sample_rate_hz: 44_100,
                        ..packet(960)
                    },
                    &losses,
                )
            });
            rate_change_pending.wait();
            release_registration.wait();
            consume.join().unwrap().unwrap();
        });
        loss_writer.join().unwrap();

        let events = coordinator.timeline_events();
        let gap_index = events
            .iter()
            .position(|event| matches!(event, TimelineEvent::Gap(gap) if gap.start_ms == 10 && gap.duration_ms == 10))
            .unwrap();
        let new_configuration_index = events
            .iter()
            .position(|event| matches!(event, TimelineEvent::TrackConfigured(configuration) if configuration.revision == 2))
            .unwrap();
        assert!(gap_index < new_configuration_index);
    }

    #[test]
    fn resampler_resets_follow_emitted_revision_events() {
        let (ports, _recording_rx, _) = ports(2, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();

        coordinator.consume(&packet(0), &losses).unwrap();
        coordinator
            .consume(
                &CapturePacket {
                    sample_rate_hz: 44_100,
                    ..packet(480)
                },
                &losses,
            )
            .unwrap();

        assert_eq!(
            coordinator.revision_events(),
            &[
                RevisionEvent::TrackConfigured(1),
                RevisionEvent::ClockMapped(1),
                RevisionEvent::ResamplerReset {
                    track_revision: 1,
                    clock_revision: 1,
                },
                RevisionEvent::TrackConfigured(2),
                RevisionEvent::ClockMapped(2),
                RevisionEvent::ResamplerReset {
                    track_revision: 2,
                    clock_revision: 2,
                },
            ]
        );
    }

    #[test]
    fn sink_workers_shutdown_after_ports_close() {
        let (ports, recording_rx, _) = ports(1, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let worker = std::thread::spawn(move || {
            while recording_rx.recv_timeout(Duration::from_millis(10)).is_ok() {}
        });

        coordinator.close();

        worker.join().unwrap();
    }
}
