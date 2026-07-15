use std::time::Duration;

use super::{request, TestDirectory};
use crate::media_protocol::{inspect_media_source, open_unchanged_media_source, MediaOwner};

#[test]
fn admitted_source_lease_is_not_retargeted_by_path_replacement() {
    let directory = TestDirectory::new("replacement");
    let path = directory.join("meeting.wav");
    let original = directory.join("original.wav");
    std::fs::write(&path, b"original bytes").unwrap();
    let owner = MediaOwner::with_capacity_for_test(4);
    let admission = owner.admit(&path, 32 * 1024 * 1024).unwrap();
    match std::fs::rename(&path, &original) {
        Ok(()) => std::fs::write(&path, b"replacement bytes").unwrap(),
        Err(error) if cfg!(windows) => {
            assert!(path.is_file(), "lease failure was unexpected: {error}");
        }
        Err(error) => panic!("unexpected replacement failure: {error}"),
    }

    let response = request(&admission.url, "GET", None);
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"original bytes");
    assert_eq!(owner.active_admission_count_for_test(), 1);
}

#[test]
fn preprocessing_opens_the_exact_validated_source_without_following_replacements() {
    let directory = TestDirectory::new("preprocessing-source");
    let path = directory.join("meeting.wav");
    let original = directory.join("original.wav");
    std::fs::write(&path, b"original bytes").unwrap();
    let fingerprint = inspect_media_source(&path).unwrap();

    let opened = open_unchanged_media_source(&path, &fingerprint).unwrap();
    assert_eq!(opened.metadata().unwrap().len(), 14);
    drop(opened);

    std::fs::rename(&path, &original).unwrap();
    std::fs::write(&path, b"replacement bytes").unwrap();
    assert!(open_unchanged_media_source(&path, &fingerprint).is_err());
}

#[test]
fn preprocessing_rejects_same_identity_same_length_rewrites() {
    let directory = TestDirectory::new("preprocessing-rewrite");
    let path = directory.join("meeting.wav");
    std::fs::write(&path, b"first bytes").unwrap();
    let fingerprint = inspect_media_source(&path).unwrap();

    std::thread::sleep(Duration::from_millis(10));
    std::fs::write(&path, b"other bytes").unwrap();

    assert!(open_unchanged_media_source(&path, &fingerprint).is_err());
}
