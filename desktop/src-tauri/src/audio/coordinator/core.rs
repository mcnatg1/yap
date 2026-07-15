use std::sync::Arc;

use crate::audio::capture::CapturePacket;
use crate::audio::frame::PreparedFrame;
use crate::audio::preprocess::{downmix_to_mono, rms_level, AudioLevelNormalizer, LinearResampler};
use crate::audio::session::{SessionId, TrackId};
use crate::audio::timeline::{LossAccumulator, RecordingInput, Timeline};

use super::pending_losses::PendingLosses;
use super::sink_types::CoordinatorPorts;
use super::TARGET_SAMPLE_RATE_HZ;

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

pub struct Coordinator {
    pub(super) track_id: TrackId,
    pub(super) ports: CoordinatorPorts,
    pub(super) timeline: Timeline,
    pub(super) capture_config: Option<(u16, u32)>,
    pub(super) track_revision: u32,
    pub(super) clock_revision: u32,
    pub(super) last_session_end_ms: u64,
    pub(super) resampler: Option<LinearResampler>,
    pub(super) level_normalizer: AudioLevelNormalizer,
    pub(super) pending_losses: PendingLosses,
    #[cfg(test)]
    pub(super) revision_events: Vec<RevisionEvent>,
    #[cfg(test)]
    pub(super) loss_pending_hook: Option<Arc<dyn Fn() + Send + Sync>>,
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
}
