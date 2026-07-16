use super::*;

#[test]
fn app_open_path_allows_only_recordings_and_transcripts() {
    assert!(is_yap_media_or_transcript_path(std::path::Path::new(
        "recording.mp3"
    )));
    assert!(is_yap_media_or_transcript_path(std::path::Path::new(
        "recording.MP4"
    )));
    assert!(is_yap_media_or_transcript_path(std::path::Path::new(
        "recording.txt"
    )));
    assert!(!is_yap_media_or_transcript_path(std::path::Path::new(
        "script.ps1"
    )));
}

#[test]
fn app_open_path_rejects_media_named_directories() {
    let dir = temp_test_dir("open-media-dir");
    let media_dir = dir.join("clip.wav");
    std::fs::create_dir_all(&media_dir).unwrap();

    let err = openable_app_path(media_dir.display().to_string()).unwrap_err();

    assert_eq!(
        err,
        "Only Yap recording and transcript files can be opened."
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn openable_app_path_rejects_unregistered_external_media() {
    let dir = temp_test_dir("open-unregistered-media");
    let registry = dir.join("registry.json");
    let owned_dir = dir.join("owned");
    let media = dir.join("meeting.wav");
    std::fs::create_dir_all(&owned_dir).unwrap();
    std::fs::write(&media, b"RIFF").unwrap();

    let error =
        openable_app_path_from(media.display().to_string(), &registry, &owned_dir).unwrap_err();

    assert_eq!(error, "Recording file is not registered for playback.");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn openable_app_path_accepts_registered_external_media() {
    let dir = temp_test_dir("open-registered-media");
    let registry = dir.join("registry.json");
    let owned_dir = dir.join("owned");
    let media = dir.join("meeting.wav");
    std::fs::create_dir_all(&owned_dir).unwrap();
    std::fs::write(&media, b"RIFF").unwrap();
    register_playback_path_at(media.display().to_string(), &registry).unwrap();

    let opened =
        openable_app_path_from(media.display().to_string(), &registry, &owned_dir).unwrap();

    assert_eq!(opened.file_name().unwrap(), "meeting.wav");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn openable_app_path_rejects_uncommitted_yap_owned_live_transcripts() {
    let dir = temp_test_dir("open-owned-live-transcript");
    let registry = dir.join("registry.json");
    let owned_dir = dir.join("owned");
    let transcript = owned_dir.join("live-400.txt");
    std::fs::create_dir_all(&owned_dir).unwrap();
    std::fs::write(&transcript, "hello").unwrap();

    assert!(
        openable_app_path_from(transcript.display().to_string(), &registry, &owned_dir).is_err()
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn pre_release_owned_paths_are_rejected_by_every_native_action() {
    let dir = temp_test_dir("pre-release-action-authorization");
    let registry = dir.join("registry.json");
    let transcript = dir.join("live-1720656000000.txt");
    let audio = dir.join("live-1720656000000.wav");
    std::fs::write(&transcript, "untrusted\n").unwrap();
    std::fs::write(&audio, b"RIFF").unwrap();

    assert!(read_text_file_at_from_dir(transcript.display().to_string(), &dir).is_err());
    assert!(read_text_preview_at_from_dir(transcript.display().to_string(), 20, &dir).is_err());
    assert!(
        write_polished_text_at_from_dir(transcript.display().to_string(), "no".into(), &dir)
            .is_err()
    );
    assert!(openable_app_path_from(transcript.display().to_string(), &registry, &dir).is_err());
    assert!(openable_app_path_from(audio.display().to_string(), &registry, &dir).is_err());
    assert!(
        register_playback_path_at_from_owned_dir(audio.display().to_string(), &registry, &dir)
            .is_err()
    );
    assert!(transcript.is_file());
    assert!(audio.is_file());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playback_path_accepts_canonical_media_files() {
    let dir = temp_test_dir("playback-media");
    let media = dir.join("Clip.WAV");
    std::fs::write(&media, b"RIFF").unwrap();

    let path = playable_recording_path(media.display().to_string()).unwrap();

    assert!(path.is_absolute());
    assert_eq!(path.file_name().unwrap(), "Clip.WAV");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playback_path_rejects_transcripts() {
    let dir = temp_test_dir("playback-transcript");
    let transcript = dir.join("clip.txt");
    std::fs::write(&transcript, "hello").unwrap();

    let error = playable_recording_path(transcript.display().to_string()).unwrap_err();

    assert_eq!(error, "Choose a supported audio or video file.");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playback_path_rejects_media_named_directories() {
    let dir = temp_test_dir("playback-media-dir");
    let media_dir = dir.join("clip.wav");
    std::fs::create_dir_all(&media_dir).unwrap();

    let error = playable_recording_path(media_dir.display().to_string()).unwrap_err();

    assert_eq!(error, "Choose a supported audio or video file.");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playback_path_rejects_missing_files() {
    let dir = temp_test_dir("playback-missing");
    let missing = dir.join("missing.wav");

    let error = playable_recording_path(missing.display().to_string()).unwrap_err();

    assert_eq!(error, "File no longer exists.");
    std::fs::remove_dir_all(dir).ok();
}
