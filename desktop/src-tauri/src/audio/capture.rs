use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};

use crate::audio::frame::GapCause;
use crate::audio::timeline::LossAccumulator;

const CAPTURE_BUFFER_COUNT: usize = 8;
const DEFAULT_BUFFER_CAPACITY_SAMPLES: usize = 8192;
const PACKET_CHANNEL_CAPACITY: usize = CAPTURE_BUFFER_COUNT;

pub struct CaptureAdapter {
    stream: cpal::Stream,
    worker: JoinHandle<()>,
}

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

impl CaptureAdapter {
    pub fn open<W>(
        device: cpal::Device,
        config: cpal::StreamConfig,
        sample_format: cpal::SampleFormat,
        run_worker: W,
    ) -> Result<Self, String>
    where
        W: FnOnce(CapturePorts, mpsc::Receiver<cpal::StreamError>) + Send + 'static,
    {
        let capacity = buffer_capacity_samples(config.buffer_size, config.channels)?;
        let (callback, ports) = new_callback_boundary(
            config.channels,
            config.sample_rate.0,
            capacity,
            PACKET_CHANNEL_CAPACITY,
            0,
        )?;
        let (errors, error_receiver) = mpsc::sync_channel(1);
        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                build_capture_stream::<f32>(&device, &config, callback, errors)
            }
            cpal::SampleFormat::I16 => {
                build_capture_stream::<i16>(&device, &config, callback, errors)
            }
            cpal::SampleFormat::U16 => {
                build_capture_stream::<u16>(&device, &config, callback, errors)
            }
            format => Err(format!("Unsupported microphone format: {format}")),
        }?;
        let worker = std::thread::spawn(move || run_worker(ports, error_receiver));
        if let Err(error) = stream.play() {
            drop(stream);
            let message = format!("Microphone access failed: {error}");
            return match join_capture_worker(worker) {
                Ok(()) => Err(message),
                Err(join_error) => Err(format!("{message}; {join_error}")),
            };
        }
        Ok(Self { stream, worker })
    }

    pub fn shutdown(self) -> Result<(), String> {
        let Self { stream, worker } = self;
        drop(stream);
        join_capture_worker(worker)
    }
}

pub(crate) fn join_capture_worker(worker: JoinHandle<()>) -> Result<(), String> {
    if worker.thread().id() == std::thread::current().id() {
        return Err("Capture worker attempted to join itself.".to_string());
    }
    worker
        .join()
        .map_err(|_| "Capture worker panicked during shutdown.".to_string())
}

fn buffer_capacity_samples(buffer_size: cpal::BufferSize, channels: u16) -> Result<usize, String> {
    if channels == 0 {
        return Err("Invalid microphone channel count.".into());
    }
    match buffer_size {
        cpal::BufferSize::Default => Ok(DEFAULT_BUFFER_CAPACITY_SAMPLES),
        cpal::BufferSize::Fixed(frames) => usize::try_from(frames)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(channels)))
            .filter(|samples| *samples > 0)
            .ok_or_else(|| "Invalid microphone buffer size.".to_string()),
    }
}

fn build_capture_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut callback: CaptureCallback,
    errors: mpsc::SyncSender<cpal::StreamError>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + CaptureSample,
{
    device
        .build_input_stream(
            config,
            move |input: &[T], _| callback.write(input),
            move |error| {
                let _ = errors.try_send(error);
            },
            Some(Duration::from_millis(250)),
        )
        .map_err(|error| format!("Microphone access failed: {error}"))
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
    fn write<T>(&mut self, input: &[T])
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

trait CaptureSample {
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::mpsc;

    use crate::audio::frame::GapCause;
    use crate::audio::timeline::{LossSnapshot, TimelineError};

    use super::{buffer_capacity_samples, join_capture_worker, new_callback_boundary};

    const CHANNELS: u16 = 2;
    const SAMPLE_RATE_HZ: u32 = 48_000;

    #[test]
    fn callback_capacity_uses_fixed_frames_or_default_samples() {
        assert_eq!(
            buffer_capacity_samples(cpal::BufferSize::Fixed(256), CHANNELS),
            Ok(512)
        );
        assert_eq!(
            buffer_capacity_samples(cpal::BufferSize::Default, CHANNELS),
            Ok(8192)
        );
    }

    #[test]
    fn construction_preallocates_exactly_eight_fixed_capacity_buffers() {
        let (mut callback, ports) =
            new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 4, 8, 0).unwrap();

        for _ in 0..8 {
            callback.write(&[0.25_f32, -0.25]);
        }

        let packets = (0..8)
            .map(|_| ports.packets.recv().unwrap())
            .collect::<Vec<_>>();
        let allocations = packets
            .iter()
            .map(|packet| packet.samples.as_ptr())
            .collect::<HashSet<_>>();
        assert_eq!(allocations.len(), 8);
        assert!(packets.iter().all(|packet| packet.samples.capacity() == 4));
        assert_eq!(ports.losses.drain(), Ok(None));
    }

    #[test]
    fn returned_buffer_is_reused_without_reallocation() {
        let (mut callback, ports) =
            new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 4, 8, 0).unwrap();
        for _ in 0..8 {
            callback.write(&[0.5_f32, -0.5]);
        }
        let mut packets = (0..8)
            .map(|_| ports.packets.recv().unwrap())
            .collect::<Vec<_>>();
        let returned = packets.pop().unwrap().samples;
        let allocation = returned.as_ptr();
        ports.returned_buffers.send(returned).unwrap();

        callback.write(&[1_i16, -1]);

        let reused = ports.packets.recv().unwrap();
        assert_eq!(reused.samples.as_ptr(), allocation);
        assert_eq!(reused.samples.capacity(), 4);
    }

    #[test]
    fn pool_empty_records_one_exact_loss_and_advances_position() {
        let (mut callback, ports) =
            new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 2, 8, 0).unwrap();
        for _ in 0..8 {
            callback.write(&[0.0_f32, 0.0]);
        }
        let mut packets = (0..8)
            .map(|_| ports.packets.recv().unwrap())
            .collect::<Vec<_>>();

        callback.write(&[0.0_f32, 0.0]);
        let returned = packets.pop().unwrap().samples;
        ports.returned_buffers.send(returned).unwrap();
        callback.write(&[0.0_f32, 0.0]);

        assert_eq!(
            ports.losses.drain(),
            Ok(Some(LossSnapshot {
                first_source_position_frames: 8,
                dropped_frames: 1,
                cause: GapCause::CallbackPoolExhausted,
                generation: 0,
            }))
        );
        assert_eq!(ports.packets.recv().unwrap().source_position_frames, 9);
    }

    #[test]
    fn oversized_callback_is_discarded_without_growing_a_buffer() {
        let (mut callback, ports) =
            new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 4, 8, 0).unwrap();

        callback.write(&[0.0_f32; 6]);
        callback.write(&[0.0_f32; 2]);

        assert_eq!(
            ports.losses.drain(),
            Ok(Some(LossSnapshot {
                first_source_position_frames: 0,
                dropped_frames: 3,
                cause: GapCause::OversizedCallback,
                generation: 0,
            }))
        );
        let packet = ports.packets.recv().unwrap();
        assert_eq!(packet.source_position_frames, 3);
        assert_eq!(packet.samples.capacity(), 4);
    }

    #[test]
    fn full_packet_channel_returns_each_buffer_and_records_each_loss_once() {
        let (mut callback, ports) =
            new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 2, 0, 0).unwrap();

        for _ in 0..9 {
            callback.write(&[0_u16, u16::MAX]);
        }

        assert_eq!(
            ports.losses.drain(),
            Ok(Some(LossSnapshot {
                first_source_position_frames: 0,
                dropped_frames: 9,
                cause: GapCause::SinkUnavailable,
                generation: 0,
            }))
        );
    }

    #[test]
    fn disconnected_packet_channel_returns_each_buffer_and_records_each_loss_once() {
        let (mut callback, ports) =
            new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 2, 8, 0).unwrap();
        drop(ports.packets);

        for _ in 0..9 {
            callback.write(&[0.0_f32, 0.0]);
        }

        assert_eq!(
            ports.losses.drain(),
            Ok(Some(LossSnapshot {
                first_source_position_frames: 0,
                dropped_frames: 9,
                cause: GapCause::SinkUnavailable,
                generation: 0,
            }))
        );
    }

    #[test]
    fn source_positions_count_frames_not_interleaved_samples() {
        let (mut callback, ports) =
            new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 4, 8, 0).unwrap();

        callback.write(&[0.0_f32; 4]);
        let first = ports.packets.recv().unwrap();
        ports.returned_buffers.send(first.samples).unwrap();
        callback.write(&[0.0_f32; 2]);
        let second = ports.packets.recv().unwrap();

        assert_eq!(first.source_position_frames, 0);
        assert_eq!(second.source_position_frames, 2);
        assert_eq!(second.channels, CHANNELS);
        assert_eq!(second.sample_rate_hz, SAMPLE_RATE_HZ);
    }

    #[test]
    fn source_position_overflow_is_fail_visible() {
        let (mut callback, ports) =
            new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 4, 8, u64::MAX - 1).unwrap();

        callback.write(&[0.0_f32; 4]);

        assert_eq!(ports.losses.drain(), Err(TimelineError::InvalidTiming));
        assert!(matches!(
            ports.packets.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn malformed_callback_records_its_ceil_frame_interval_and_keeps_positions_honest() {
        let (mut callback, ports) =
            new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 4, 8, 100).unwrap();

        callback.write(&[0.0_f32; 3]);
        callback.write(&[0.0_f32; 2]);

        assert_eq!(
            ports.losses.drain(),
            Ok(Some(LossSnapshot {
                first_source_position_frames: 100,
                dropped_frames: 2,
                cause: GapCause::DeviceDiscontinuity,
                generation: 0,
            }))
        );
        assert_eq!(ports.packets.recv().unwrap().source_position_frames, 102);
    }

    #[test]
    fn panicked_capture_worker_join_is_reported() {
        let worker = std::thread::spawn(|| panic!("synthetic capture worker panic"));

        assert!(join_capture_worker(worker).is_err());
    }
}
