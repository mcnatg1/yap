use std::sync::{mpsc, Arc};

use crate::audio::{frame::GapCause, timeline::LossAccumulator};

const CAPTURE_BUFFER_COUNT: usize = 8;

pub struct CapturePacket {
    pub source_position_frames: u64,
    pub channels: u16,
    pub sample_rate_hz: u32,
    pub samples: Vec<f32>,
}

pub struct CapturePorts {
    pub packets: mpsc::Receiver<CapturePacket>,
    pub returned_buffers: mpsc::SyncSender<Vec<f32>>,
    pub losses: Arc<LossAccumulator>,
}

pub(crate) struct CaptureCallback {
    channels: u16,
    sample_rate_hz: u32,
    buffer_capacity_samples: usize,
    source_position_frames: Option<u64>,
    packets: mpsc::SyncSender<CapturePacket>,
    returned_buffers: mpsc::Receiver<Vec<f32>>,
    return_sender: mpsc::SyncSender<Vec<f32>>,
    held_buffer: Option<Vec<f32>>,
    losses: Arc<LossAccumulator>,
}

pub(crate) fn new_callback_boundary(
    channels: u16,
    sample_rate_hz: u32,
    buffer_capacity_samples: usize,
    packet_capacity: usize,
    source_position_frames: u64,
) -> Result<(CaptureCallback, CapturePorts), String> {
    if channels == 0 || sample_rate_hz == 0 || buffer_capacity_samples == 0 {
        return Err("Invalid microphone configuration.".into());
    }

    let (packets, packet_receiver) = mpsc::sync_channel(packet_capacity);
    let (return_sender, returned_buffers) = mpsc::sync_channel(CAPTURE_BUFFER_COUNT);
    for _ in 0..CAPTURE_BUFFER_COUNT {
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(buffer_capacity_samples)
            .map_err(|_| "Microphone buffer allocation failed.".to_string())?;
        return_sender
            .send(buffer)
            .map_err(|_| "Microphone buffer pool initialization failed.".to_string())?;
    }
    let losses = Arc::new(LossAccumulator::new());
    let callback = CaptureCallback {
        channels,
        sample_rate_hz,
        buffer_capacity_samples,
        source_position_frames: Some(source_position_frames),
        packets,
        returned_buffers,
        return_sender: return_sender.clone(),
        held_buffer: None,
        losses: Arc::clone(&losses),
    };
    let ports = CapturePorts {
        packets: packet_receiver,
        returned_buffers: return_sender,
        losses,
    };
    Ok((callback, ports))
}

impl CaptureCallback {
    pub(super) fn write<T>(&mut self, input: &[T])
    where
        T: CaptureSample,
    {
        let Some(source_position_frames) = self.source_position_frames else {
            return;
        };
        let channels = usize::from(self.channels);
        if !input.len().is_multiple_of(channels) {
            let Some(frame_count) = callback_frame_count(input.len(), channels, true) else {
                self.losses.invalidate();
                self.source_position_frames = None;
                return;
            };
            self.record_discontinuity(source_position_frames, frame_count);
            return;
        }
        let Some(frame_count) = callback_frame_count(input.len(), channels, false) else {
            self.losses.invalidate();
            self.source_position_frames = None;
            return;
        };
        if frame_count == 0 {
            return;
        }
        let Some(next_source_position_frames) = source_position_frames.checked_add(frame_count)
        else {
            self.losses.record(
                source_position_frames,
                frame_count,
                GapCause::DeviceDiscontinuity,
            );
            self.source_position_frames = None;
            return;
        };
        self.source_position_frames = Some(next_source_position_frames);

        if input.len() > self.buffer_capacity_samples {
            self.losses.record(
                source_position_frames,
                frame_count,
                GapCause::OversizedCallback,
            );
            return;
        }

        let mut samples = match self
            .held_buffer
            .take()
            .or_else(|| self.returned_buffers.try_recv().ok())
        {
            Some(samples) => samples,
            None => {
                self.losses.record(
                    source_position_frames,
                    frame_count,
                    GapCause::CallbackPoolExhausted,
                );
                return;
            }
        };
        if samples.capacity() < input.len() {
            self.losses.record(
                source_position_frames,
                frame_count,
                GapCause::OversizedCallback,
            );
            self.return_buffer(samples);
            return;
        }
        samples.clear();
        // A callback is the smallest timing unit: publish every sample or one exact gap.
        for sample in input {
            let Some(sample) = sample.capture_f32() else {
                self.losses.record(
                    source_position_frames,
                    frame_count,
                    GapCause::DeviceDiscontinuity,
                );
                self.return_buffer(samples);
                return;
            };
            samples.push(sample);
        }

        let packet = CapturePacket {
            source_position_frames,
            channels: self.channels,
            sample_rate_hz: self.sample_rate_hz,
            samples,
        };
        match self.packets.try_send(packet) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(packet))
            | Err(mpsc::TrySendError::Disconnected(packet)) => {
                self.losses.record(
                    source_position_frames,
                    frame_count,
                    GapCause::SinkUnavailable,
                );
                self.return_buffer(packet.samples);
            }
        }
    }

    fn record_discontinuity(&mut self, source_position_frames: u64, frame_count: u64) {
        let Some(next_source_position_frames) = source_position_frames.checked_add(frame_count)
        else {
            self.losses.record(
                source_position_frames,
                frame_count,
                GapCause::DeviceDiscontinuity,
            );
            self.source_position_frames = None;
            return;
        };
        self.source_position_frames = Some(next_source_position_frames);
        self.losses.record(
            source_position_frames,
            frame_count,
            GapCause::DeviceDiscontinuity,
        );
    }

    fn return_buffer(&mut self, mut buffer: Vec<f32>) {
        buffer.clear();
        if let Err(error) = self.return_sender.try_send(buffer) {
            self.held_buffer = Some(match error {
                mpsc::TrySendError::Full(buffer) | mpsc::TrySendError::Disconnected(buffer) => {
                    buffer
                }
            });
        }
    }

    #[cfg(test)]
    pub(crate) fn write_f32_for_test(&mut self, input: &[f32]) {
        self.write(input);
    }
}

fn callback_frame_count(input_len: usize, channels: usize, ceil: bool) -> Option<u64> {
    let frames = if ceil {
        input_len.checked_add(channels.checked_sub(1)?)? / channels
    } else {
        input_len / channels
    };
    u64::try_from(frames).ok()
}

pub(super) trait CaptureSample {
    fn capture_f32(&self) -> Option<f32>;
}

impl CaptureSample for f32 {
    fn capture_f32(&self) -> Option<f32> {
        if !self.is_finite() {
            None
        } else if self.is_subnormal() {
            Some(0.0)
        } else {
            Some(self.clamp(-1.0, 1.0))
        }
    }
}

impl CaptureSample for i16 {
    fn capture_f32(&self) -> Option<f32> {
        Some((*self as f32 / i16::MAX as f32).clamp(-1.0, 1.0))
    }
}

impl CaptureSample for u16 {
    fn capture_f32(&self) -> Option<f32> {
        Some(((*self as f32 - 32_768.0) / 32_768.0).clamp(-1.0, 1.0))
    }
}
