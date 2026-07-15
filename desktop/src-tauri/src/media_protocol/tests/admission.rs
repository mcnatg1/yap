use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use super::{request, TestDirectory};
use crate::media_protocol::{admission_metadata, MediaOwner};

#[test]
fn admission_mints_fresh_opaque_urls_and_reclaims_oldest_at_capacity() {
    let directory = TestDirectory::new("opaque-capacity");
    let path = directory.join("private-meeting.wav");
    std::fs::write(&path, b"0123456789").unwrap();
    let owner = MediaOwner::with_capacity_for_test(2);

    let first = owner.admit(&path, 32 * 1024 * 1024).unwrap();
    let second = owner.admit(&path, 32 * 1024 * 1024).unwrap();
    let third = owner.admit(&path, 32 * 1024 * 1024).unwrap();

    assert_ne!(first.url, second.url);
    assert_ne!(second.url, third.url);
    assert!(!first.url.contains("private-meeting"));
    assert!(!first.url.contains(".wav"));
    assert_eq!(owner.active_admission_count_for_test(), 2);
    assert_eq!(request(&first.url, "GET", None).status, 404);
    assert_eq!(request(&third.url, "GET", None).body, b"0123456789");
}

#[test]
fn release_revokes_a_url_and_is_idempotent() {
    let directory = TestDirectory::new("release");
    let path = directory.join("meeting.wav");
    std::fs::write(&path, b"RIFFpayload").unwrap();
    let owner = MediaOwner::with_capacity_for_test(4);
    let admission = owner.admit(&path, 32 * 1024 * 1024).unwrap();

    assert!(owner.release(&admission.url));
    assert!(!owner.release(&admission.url));
    assert_eq!(request(&admission.url, "GET", None).status, 404);
    assert_eq!(owner.active_admission_count_for_test(), 0);
}

#[test]
fn admissions_expire_after_idle_and_absolute_lifetimes() {
    let directory = TestDirectory::new("expiry");
    let path = directory.join("meeting.wav");
    std::fs::write(&path, b"RIFFpayload").unwrap();
    let elapsed_ms = Arc::new(AtomicU64::new(0));
    let base = Instant::now();
    let clock_elapsed = Arc::clone(&elapsed_ms);
    let owner = MediaOwner::with_policy_for_test(
        4,
        Duration::from_millis(5),
        Duration::from_millis(10),
        Arc::new(move || base + Duration::from_millis(clock_elapsed.load(Ordering::Acquire))),
    );
    let admission = owner.admit(&path, 32 * 1024 * 1024).unwrap();

    elapsed_ms.store(4, Ordering::Release);
    assert_eq!(request(&admission.url, "HEAD", None).status, 200);
    elapsed_ms.store(8, Ordering::Release);
    assert_eq!(request(&admission.url, "HEAD", None).status, 200);
    elapsed_ms.store(11, Ordering::Release);
    assert_eq!(request(&admission.url, "HEAD", None).status, 404);
    assert_eq!(owner.active_admission_count_for_test(), 0);

    let idle_admission = owner.admit(&path, 32 * 1024 * 1024).unwrap();
    elapsed_ms.store(17, Ordering::Release);
    assert_eq!(request(&idle_admission.url, "HEAD", None).status, 404);
    assert_eq!(owner.active_admission_count_for_test(), 0);
}

#[test]
fn admission_preserves_u64_length_as_decimal_and_fails_closed_for_waveforms() {
    let length = 9_007_199_254_740_993_u64;
    let (byte_length, waveform_eligible) = admission_metadata(length, 32 * 1024 * 1024);

    assert_eq!(byte_length, "9007199254740993");
    assert!(!waveform_eligible);
}
