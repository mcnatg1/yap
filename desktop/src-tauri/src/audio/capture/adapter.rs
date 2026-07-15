use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};

use super::callback::{new_callback_boundary, CaptureCallback, CapturePorts, CaptureSample};

const DEFAULT_BUFFER_CAPACITY_SAMPLES: usize = 8192;
const PACKET_CHANNEL_CAPACITY: usize = 8;

pub struct CaptureAdapter {
    stream: cpal::Stream,
    worker: JoinHandle<()>,
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

pub(super) fn buffer_capacity_samples(
    buffer_size: cpal::BufferSize,
    channels: u16,
) -> Result<usize, String> {
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
