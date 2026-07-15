use std::{collections::HashSet, sync::mpsc};

use crate::audio::{
    frame::GapCause,
    timeline::{LossSnapshot, TimelineError},
};

use super::super::new_callback_boundary;

const CHANNELS: u16 = 2;
const SAMPLE_RATE_HZ: u32 = 48_000;

#[test]
fn construction_preallocates_exactly_eight_fixed_capacity_buffers() {
    let (mut callback, ports) = new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 4, 8, 0).unwrap();

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
    let (mut callback, ports) = new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 4, 8, 0).unwrap();
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
    let (mut callback, ports) = new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 2, 8, 0).unwrap();
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
    let (mut callback, ports) = new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 4, 8, 0).unwrap();

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
    let (mut callback, ports) = new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 2, 0, 0).unwrap();

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
    let (mut callback, ports) = new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 2, 8, 0).unwrap();
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
    let (mut callback, ports) = new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 4, 8, 0).unwrap();

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
    let (mut callback, ports) = new_callback_boundary(CHANNELS, SAMPLE_RATE_HZ, 4, 8, 100).unwrap();

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
