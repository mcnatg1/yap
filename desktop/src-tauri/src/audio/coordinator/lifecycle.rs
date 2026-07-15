use std::time::Duration;

#[cfg(test)]
use std::sync::Arc;

use crate::audio::capture::CapturePacket;
use crate::audio::frame::{PreparedFrame, TrackConfigurationRevision};
use crate::audio::preprocess::LinearResampler;
use crate::audio::timeline::{
    ClockMappingRevision, LossAccumulator, LossSnapshot, RecordingInput,
    RecordingRevisionTransition, TryDrain,
};

use super::core::Coordinator;
#[cfg(test)]
use super::core::RevisionEvent;
use super::pending_losses::LOSS_DRAIN_ATTEMPT_LIMIT;
use super::sink_types::{BoundedSink, SinkKind, SinkOutcome};
use super::TARGET_SAMPLE_RATE_HZ;

impl Coordinator {
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

    pub fn consume_loss(&mut self, loss: LossSnapshot) -> Result<(), String> {
        let result = self.consume_loss_inner(loss);
        if let Err(error) = &result {
            self.ports.recording.degrade(error);
        }
        result
    }

    fn consume_loss_inner(&mut self, loss: LossSnapshot) -> Result<(), String> {
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

    pub(super) fn apply_loss(&mut self, loss: LossSnapshot) -> Result<(), String> {
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
    pub(super) fn set_loss_pending_hook_for_test(&mut self, hook: Arc<dyn Fn() + Send + Sync>) {
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

    pub(super) fn drain_losses(&mut self, losses: &LossAccumulator) -> Result<(), String> {
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

    pub(super) fn ensure_configuration(
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
