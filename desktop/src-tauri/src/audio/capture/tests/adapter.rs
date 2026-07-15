use super::super::adapter::{buffer_capacity_samples, join_capture_worker};

const CHANNELS: u16 = 2;

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
fn panicked_capture_worker_join_is_reported() {
    let worker = std::thread::spawn(|| panic!("synthetic capture worker panic"));

    assert!(join_capture_worker(worker).is_err());
}
