use std::{
    io::Read,
    sync::{Arc, OnceLock},
    time::Duration,
};

use serde::Serialize;
use tokio::{sync::Semaphore, time::timeout_at};

use crate::atomic_text::write as write_text_atomically;

const MAX_TRANSCRIPT_READ_BYTES: u64 = 2 * 1024 * 1024;
const MAX_HIDDEN_PRUNE_CANDIDATES: usize = 200;
const MAX_CONCURRENT_TRANSCRIPT_READS: usize = 4;
const TRANSCRIPT_READ_TIMEOUT: Duration = Duration::from_secs(8);
const TRANSCRIPT_WRITE_TIMEOUT: Duration = Duration::from_secs(15);
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedLiveTranscriptPathResolution {
    requested_path: String,
    canonical_path: Option<String>,
    missing: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingPlaybackAdmission {
    playback_path: String,
    byte_length: String,
    waveform_eligible: bool,
}

#[tauri::command]
pub fn restore_recording_playback_path(
    window: tauri::WebviewWindow,
    owner: tauri::State<'_, crate::media_protocol::MediaOwner>,
    path: String,
) -> Result<RecordingPlaybackAdmission, String> {
    ensure_main_window(&window)?;
    let path = crate::recording_access::restore_playback_path_at(
        path,
        &crate::recording_access::recording_job_playback_registry_path(),
        &crate::live::recordings::recordings_dir(),
    )?;
    mint_playback_admission(&path, &owner)
}

#[tauri::command]
pub fn release_recording_playback(
    window: tauri::WebviewWindow,
    owner: tauri::State<'_, crate::media_protocol::MediaOwner>,
    playback_path: String,
) -> Result<(), String> {
    ensure_main_window(&window)?;
    owner.release(&playback_path);
    Ok(())
}

#[tauri::command]
pub fn resolve_owned_live_transcript_paths(
    window: tauri::WebviewWindow,
    output_paths: Vec<String>,
) -> Result<Vec<OwnedLiveTranscriptPathResolution>, String> {
    ensure_main_window(&window)?;
    resolve_owned_live_transcript_paths_from_dir(
        output_paths,
        &crate::live::recordings::recordings_dir(),
    )
}

fn resolve_owned_live_transcript_paths_from_dir(
    output_paths: Vec<String>,
    owned_dir: &std::path::Path,
) -> Result<Vec<OwnedLiveTranscriptPathResolution>, String> {
    if output_paths.len() > MAX_HIDDEN_PRUNE_CANDIDATES {
        return Err(format!(
            "Hidden history reconciliation accepts at most {MAX_HIDDEN_PRUNE_CANDIDATES} paths."
        ));
    }
    let Ok(owned_dir) = owned_dir.canonicalize() else {
        return Ok(Vec::new());
    };

    let mut resolutions = Vec::new();
    for output_path in output_paths {
        let path = std::path::PathBuf::from(&output_path);
        if !path.is_absolute() || !crate::live::recordings::is_primary_live_transcript_path(&path) {
            continue;
        }
        let Some(parent) = path.parent() else {
            continue;
        };
        let Ok(parent) = parent.canonicalize() else {
            continue;
        };
        if parent != owned_dir {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let canonical_candidate = owned_dir.join(file_name);
        match std::fs::symlink_metadata(&canonical_candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                resolutions.push(OwnedLiveTranscriptPathResolution {
                    requested_path: output_path,
                    canonical_path: Some(crate::live::recordings::stable_existing_path_string(
                        &canonical_candidate,
                    )),
                    missing: true,
                });
            }
            Ok(metadata) if metadata.file_type().is_file() => {
                let Ok(canonical_path) = canonical_candidate.canonicalize() else {
                    continue;
                };
                if canonical_path.parent() != Some(owned_dir.as_path()) {
                    continue;
                }
                resolutions.push(OwnedLiveTranscriptPathResolution {
                    requested_path: output_path,
                    canonical_path: Some(crate::live::recordings::stable_existing_path_string(
                        &canonical_path,
                    )),
                    missing: false,
                });
            }
            Ok(_) | Err(_) => {}
        }
    }
    Ok(resolutions)
}

#[tauri::command]
pub async fn read_text_file(window: tauri::WebviewWindow, path: String) -> Result<String, String> {
    ensure_main_window(&window)?;
    run_bounded_transcript_io(
        transcript_read_limiter(),
        TRANSCRIPT_READ_TIMEOUT,
        "Transcript read",
        move || read_text_file_at(path),
    )
    .await
}

#[tauri::command]
pub async fn read_text_preview(
    window: tauri::WebviewWindow,
    path: String,
    max_chars: Option<usize>,
) -> Result<String, String> {
    ensure_main_window(&window)?;
    let max_chars = max_chars.unwrap_or(600);
    run_bounded_transcript_io(
        transcript_read_limiter(),
        TRANSCRIPT_READ_TIMEOUT,
        "Transcript preview",
        move || read_text_preview_at(path, max_chars),
    )
    .await
}

fn read_text_file_at(path: String) -> Result<String, String> {
    read_text_file_at_from_dir(path.clone(), &crate::live::recordings::recordings_dir())
        .or_else(|_| crate::jobs::read_published_remote_transcript(std::path::Path::new(&path)))
}

fn read_text_file_at_from_dir(path: String, owned_dir: &std::path::Path) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);
    let mut file = owned_live_transcript_file_from_dir(&path, "read", owned_dir)?;
    reject_oversized_transcript_file(&file)?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|err| format!("Failed to read transcript: {err}"))?;
    Ok(text)
}

fn read_text_preview_at(path: String, max_chars: usize) -> Result<String, String> {
    match read_text_preview_at_from_dir(
        path.clone(),
        max_chars,
        &crate::live::recordings::recordings_dir(),
    ) {
        Ok(preview) => Ok(preview),
        Err(_) => {
            let text = crate::jobs::read_published_remote_transcript(std::path::Path::new(&path))?;
            Ok(text.chars().take(max_chars.clamp(1, 4_000)).collect())
        }
    }
}

fn read_text_preview_at_from_dir(
    path: String,
    max_chars: usize,
    owned_dir: &std::path::Path,
) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);

    let max_chars = max_chars.clamp(1, 4_000);
    let file = owned_live_transcript_file_from_dir(&path, "read", owned_dir)?;
    let mut bytes = Vec::new();
    std::io::Read::take(file, (max_chars.saturating_mul(4).saturating_add(4)) as u64)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("Failed to read transcript: {err}"))?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.chars().take(max_chars).collect())
}

#[tauri::command]
pub async fn write_polished_text(
    window: tauri::WebviewWindow,
    path: String,
    text: String,
) -> Result<String, String> {
    ensure_main_window(&window)?;
    run_bounded_transcript_io(
        transcript_write_limiter(),
        TRANSCRIPT_WRITE_TIMEOUT,
        "Polished transcript write",
        move || write_polished_text_at(path, text),
    )
    .await
}

fn transcript_read_limiter() -> Arc<Semaphore> {
    static LIMITER: OnceLock<Arc<Semaphore>> = OnceLock::new();
    Arc::clone(LIMITER.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_TRANSCRIPT_READS))))
}

fn transcript_write_limiter() -> Arc<Semaphore> {
    static LIMITER: OnceLock<Arc<Semaphore>> = OnceLock::new();
    Arc::clone(LIMITER.get_or_init(|| Arc::new(Semaphore::new(1))))
}

async fn run_bounded_transcript_io<F>(
    limiter: Arc<Semaphore>,
    timeout: Duration,
    operation: &'static str,
    work: F,
) -> Result<String, String>
where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    let deadline = tokio::time::Instant::now() + timeout;
    let permit = timeout_at(deadline, limiter.acquire_owned())
        .await
        .map_err(|_| format!("{operation} timed out while waiting for filesystem capacity."))?
        .map_err(|_| format!("{operation} is unavailable during shutdown."))?;
    let task = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        work()
    });

    match timeout_at(deadline, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!("{operation} worker failed: {error}")),
        Err(_) => Err(format!("{operation} timed out.")),
    }
}

fn write_polished_text_at(path: String, text: String) -> Result<String, String> {
    write_polished_text_at_from_dir(path, text, &crate::live::recordings::recordings_dir())
}

fn write_polished_text_at_from_dir(
    path: String,
    text: String,
    owned_dir: &std::path::Path,
) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);
    let path = owned_live_transcript_path_from_dir(&path, "polished", owned_dir)?;
    let _source =
        crate::live::recordings::open_committed_live_transcript_from_dir(&path, owned_dir)
            .map_err(|_| {
                "Only Yap-owned canonical live transcripts can be polished.".to_string()
            })?;
    let output = polished_path(&path)?;
    write_text_atomically(&output, &text)
        .map_err(|err| format!("Failed to save polished transcript: {err}"))?;
    Ok(output.display().to_string())
}

fn polished_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| "Transcript path has no file name.".to_string())?;

    Ok(path.with_file_name(format!("{stem}.polished.txt")))
}

#[tauri::command]
pub fn open_app_path(window: tauri::WebviewWindow, path: String) -> Result<(), String> {
    ensure_main_window(&window)?;
    let path = openable_app_path(path)?;
    tauri_plugin_opener::open_path(&path, None::<&str>)
        .map_err(|err| format!("Failed to open file: {err}"))
}

#[tauri::command]
pub fn reveal_app_path(window: tauri::WebviewWindow, path: String) -> Result<(), String> {
    ensure_main_window(&window)?;
    let path = openable_app_path(path)?;
    tauri_plugin_opener::reveal_item_in_dir(path)
        .map_err(|err| format!("Failed to reveal file: {err}"))
}

fn openable_app_path(path: String) -> Result<std::path::PathBuf, String> {
    let candidate = std::path::PathBuf::from(&path);
    if crate::live::recordings::is_transcript_path(&candidate) {
        if let Ok(authorized) = crate::jobs::authorize_published_remote_transcript(&candidate) {
            return Ok(authorized);
        }
    }
    crate::recording_access::authorize_openable_app_path(
        path,
        &crate::recording_access::recording_job_playback_registry_path(),
        &crate::live::recordings::recordings_dir(),
    )
}

fn mint_playback_admission(
    path: &std::path::Path,
    owner: &crate::media_protocol::MediaOwner,
) -> Result<RecordingPlaybackAdmission, String> {
    let admission = owner.admit(path, crate::recording_access::MAX_DECODED_WAVEFORM_BYTES)?;
    Ok(RecordingPlaybackAdmission {
        playback_path: admission.url,
        byte_length: admission.byte_length,
        waveform_eligible: admission.waveform_eligible,
    })
}

fn canonical_existing_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if !path.exists() {
        return Err("File no longer exists.".into());
    }
    path.canonicalize()
        .map_err(|err| format!("Failed to resolve file path: {err}"))
}

fn canonical_transcript_path(
    path: &std::path::Path,
    action: &str,
) -> Result<std::path::PathBuf, String> {
    if !crate::live::recordings::is_transcript_path(path) {
        return Err(format!("Only transcript text files can be {action}."));
    }
    let path = canonical_existing_path(path)?;
    if !path.is_file() || !crate::live::recordings::is_transcript_path(&path) {
        return Err(format!("Only transcript text files can be {action}."));
    }
    Ok(path)
}

fn owned_live_transcript_path_from_dir(
    path: &std::path::Path,
    action: &str,
    owned_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let path = canonical_transcript_path(path, action)?;
    crate::live::recordings::canonical_committed_live_path_from_dir(&path, owned_dir, true)
        .map_err(|_| format!("Only Yap-owned canonical live transcripts can be {action}."))
}

fn owned_live_transcript_file_from_dir(
    path: &std::path::Path,
    action: &str,
    owned_dir: &std::path::Path,
) -> Result<std::fs::File, String> {
    let path = canonical_transcript_path(path, action)?;
    crate::live::recordings::open_committed_live_transcript_from_dir(&path, owned_dir)
        .map_err(|_| format!("Only Yap-owned canonical live transcripts can be {action}."))
}

fn reject_oversized_transcript_file(file: &std::fs::File) -> Result<(), String> {
    let length = file
        .metadata()
        .map_err(|err| format!("Failed to inspect transcript: {err}"))?
        .len();
    if length > MAX_TRANSCRIPT_READ_BYTES {
        return Err(
            "Transcript is too large to load in the app. Open it from disk instead.".into(),
        );
    }
    Ok(())
}

pub(crate) fn ensure_main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if crate::authorization::is_main_window(window.label()) {
        Ok(())
    } else {
        Err("This file action is only available from the main window.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{recording::StreamingRecording, session::SessionId};
    use crate::recording_access::{
        is_yap_media_or_transcript_path,
        metadata_is_reparse_point_for_test as metadata_is_reparse_point, openable_app_path_from,
        playable_recording_path, read_registered_playback_paths, register_playback_path_at,
        register_playback_path_at_from_owned_dir,
        register_recording_job_playback_path_at_from_owned_dir, registered_playback_path_at,
        restore_playback_path_at, restore_playback_path_at_with, write_registered_playback_paths,
        RecordingPlaybackRegistry, MAX_REGISTERED_PLAYBACK_PATHS,
        NATIVE_SELECTION_REGISTRY_VERSION,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    static TEMP_TEST_DIR_COUNTER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let sequence = TEMP_TEST_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "yap-{name}-{}-{}-{sequence}",
            std::process::id(),
            crate::live::recordings::unix_millis_now().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bounded_transcript_io_keeps_capacity_owned_until_timed_out_work_finishes() {
        let limiter = Arc::new(Semaphore::new(1));
        let first = tauri::async_runtime::block_on(run_bounded_transcript_io(
            Arc::clone(&limiter),
            Duration::from_millis(10),
            "Test read",
            || {
                std::thread::sleep(Duration::from_millis(100));
                Ok("late".into())
            },
        ));
        assert!(first.unwrap_err().contains("timed out"));
        assert_eq!(limiter.available_permits(), 0);

        let second_ran = Arc::new(AtomicBool::new(false));
        let second_ran_in_work = Arc::clone(&second_ran);
        let second = tauri::async_runtime::block_on(run_bounded_transcript_io(
            Arc::clone(&limiter),
            Duration::from_millis(10),
            "Test read",
            move || {
                second_ran_in_work.store(true, Ordering::SeqCst);
                Ok("unexpected".into())
            },
        ));
        assert!(second.unwrap_err().contains("filesystem capacity"));
        assert!(!second_ran.load(Ordering::SeqCst));

        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while limiter.available_permits() == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(limiter.available_permits(), 1);
    }

    #[test]
    fn bounded_transcript_io_returns_successful_work() {
        let result = tauri::async_runtime::block_on(run_bounded_transcript_io(
            Arc::new(Semaphore::new(1)),
            Duration::from_secs(1),
            "Test read",
            || Ok("ready".into()),
        ));

        assert_eq!(result.unwrap(), "ready");
    }

    #[test]
    fn read_text_file_rejects_non_transcripts() {
        assert!(read_text_file_at("recording.mp3".into()).is_err());
    }

    #[test]
    fn hidden_prune_authorizes_only_missing_primary_owned_transcripts() {
        let dir = temp_test_dir("hidden-prune-owned");
        let existing = dir.join("live-s-100.txt");
        let missing = dir.join("live-s-101.txt");
        std::fs::write(&existing, "still here").unwrap();
        let canonical_dir = dir.canonicalize().unwrap();

        let resolutions = resolve_owned_live_transcript_paths_from_dir(
            vec![
                existing.display().to_string(),
                missing.display().to_string(),
            ],
            &dir,
        )
        .unwrap();

        assert_eq!(resolutions.len(), 2);
        assert_eq!(
            resolutions[0].requested_path,
            existing.display().to_string()
        );
        assert_eq!(
            resolutions[0].canonical_path.as_deref(),
            Some(
                crate::live::recordings::stable_existing_path_string(
                    &canonical_dir.join("live-s-100.txt"),
                )
                .as_str(),
            )
        );
        assert!(!resolutions[0].missing);
        assert_eq!(
            resolutions[1],
            OwnedLiveTranscriptPathResolution {
                requested_path: missing.display().to_string(),
                canonical_path: Some(crate::live::recordings::stable_existing_path_string(
                    &canonical_dir.join("live-s-101.txt")
                )),
                missing: true,
            }
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn hidden_prune_resolves_legacy_case_alias_to_canonical_output() {
        let dir = temp_test_dir("hidden-prune-case-alias");
        let transcript = dir.join("live-s-108.txt");
        std::fs::write(&transcript, "still here").unwrap();
        let canonical_dir = dir.canonicalize().unwrap();
        let requested = dir
            .display()
            .to_string()
            .to_uppercase()
            .replace("LIVE-RECORDINGS", "live-recordings");
        let requested = std::path::PathBuf::from(requested).join("live-s-108.txt");

        let resolutions = resolve_owned_live_transcript_paths_from_dir(
            vec![requested.display().to_string()],
            &dir,
        )
        .unwrap();

        assert_eq!(resolutions.len(), 1);
        assert_eq!(
            resolutions[0].canonical_path.as_deref(),
            Some(
                crate::live::recordings::stable_existing_path_string(
                    &canonical_dir.join("live-s-108.txt"),
                )
                .as_str(),
            )
        );
        assert!(!resolutions[0].missing);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn hidden_prune_rejects_untrusted_or_non_primary_paths() {
        let dir = temp_test_dir("hidden-prune-untrusted");
        let external = temp_test_dir("hidden-prune-external");
        let nested = dir.join("nested");
        std::fs::create_dir_all(&nested).unwrap();

        let confirmed = resolve_owned_live_transcript_paths_from_dir(
            vec![
                external.join("live-s-102.txt").display().to_string(),
                nested.join("live-s-103.txt").display().to_string(),
                "live-s-104.txt".into(),
                dir.join("live-105.polished.txt").display().to_string(),
                dir.join("live-nope.txt").display().to_string(),
                dir.join("notes.txt").display().to_string(),
            ],
            &dir,
        )
        .unwrap();

        assert!(confirmed.is_empty());
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(external).ok();
    }

    #[test]
    fn hidden_prune_preserves_existing_non_file_and_missing_root() {
        let dir = temp_test_dir("hidden-prune-directory");
        let directory = dir.join("live-106.txt");
        std::fs::create_dir_all(&directory).unwrap();
        let missing_root = dir.join("missing-root");

        assert!(resolve_owned_live_transcript_paths_from_dir(
            vec![directory.display().to_string()],
            &dir,
        )
        .unwrap()
        .is_empty());
        assert!(resolve_owned_live_transcript_paths_from_dir(
            vec![missing_root.join("live-107.txt").display().to_string()],
            &missing_root,
        )
        .unwrap()
        .is_empty());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn hidden_prune_rejects_oversized_batches() {
        let dir = temp_test_dir("hidden-prune-bound");
        let candidates = (0..=MAX_HIDDEN_PRUNE_CANDIDATES)
            .map(|index| dir.join(format!("live-{index}.txt")).display().to_string())
            .collect();

        let error = resolve_owned_live_transcript_paths_from_dir(candidates, &dir).unwrap_err();

        assert!(error.contains("at most 200"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn read_text_preview_rejects_uncommitted_live_transcript() {
        let dir = temp_test_dir("preview-cap");
        let transcript = dir.join("live-100.txt");
        std::fs::write(&transcript, "abcdef").unwrap();

        assert!(read_text_preview_at_from_dir(transcript.display().to_string(), 3, &dir).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn canonical_transcript_read_and_preview_consume_the_validated_handle() {
        let dir = temp_test_dir("validated-transcript-handle");
        let session = SessionId::new("s-validated-transcript-handle").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let capture = recording.finalize().unwrap();
        crate::live::recordings::save_finalized_capture_to_dir_for_test(
            &dir,
            "verified text",
            capture,
        )
        .unwrap();
        let transcript = dir.join(format!("live-{session}.txt"));

        assert_eq!(
            read_text_file_at_from_dir(transcript.display().to_string(), &dir).unwrap(),
            "verified text\n"
        );
        assert_eq!(
            read_text_preview_at_from_dir(transcript.display().to_string(), 8, &dir).unwrap(),
            "verified"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn read_text_preview_rejects_uncommitted_multibyte_transcript() {
        let dir = temp_test_dir("preview-multibyte");
        let transcript = dir.join("live-105.txt");
        std::fs::write(&transcript, "abcdefg€").unwrap();

        assert!(read_text_preview_at_from_dir(transcript.display().to_string(), 1, &dir).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transcript_read_rejects_directory_after_canonicalization() {
        let dir = temp_test_dir("txt-dir");
        let transcript_dir = dir.join("live-101.txt");
        std::fs::create_dir_all(&transcript_dir).unwrap();

        let error =
            read_text_file_at_from_dir(transcript_dir.display().to_string(), &dir).unwrap_err();

        assert_eq!(error, "Only transcript text files can be read.");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn read_text_file_rejects_uncommitted_oversized_transcripts() {
        let dir = temp_test_dir("oversized-read");
        let transcript = dir.join("live-102.txt");
        std::fs::write(
            &transcript,
            vec![b'a'; (MAX_TRANSCRIPT_READ_BYTES as usize) + 1],
        )
        .unwrap();

        let error = read_text_file_at_from_dir(transcript.display().to_string(), &dir).unwrap_err();

        assert_eq!(
            error,
            "Only Yap-owned canonical live transcripts can be read."
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transcript_reads_reject_external_text_files() {
        let owned_dir = temp_test_dir("owned-live-read");
        let external_dir = temp_test_dir("external-transcript-read");
        let transcript = external_dir.join("live-103.txt");
        std::fs::write(&transcript, "secret").unwrap();

        assert_eq!(
            read_text_file_at_from_dir(transcript.display().to_string(), &owned_dir).unwrap_err(),
            "Only Yap-owned canonical live transcripts can be read."
        );
        assert_eq!(
            read_text_preview_at_from_dir(transcript.display().to_string(), 10, &owned_dir)
                .unwrap_err(),
            "Only Yap-owned canonical live transcripts can be read."
        );
        assert_eq!(
            write_polished_text_at_from_dir(
                transcript.display().to_string(),
                "safe".into(),
                &owned_dir,
            )
            .unwrap_err(),
            "Only Yap-owned canonical live transcripts can be polished."
        );
        std::fs::remove_dir_all(owned_dir).ok();
        std::fs::remove_dir_all(external_dir).ok();
    }

    #[test]
    fn transcript_actions_reject_resolved_non_transcript_files() {
        let dir = temp_test_dir("txt-symlink");
        let target_dir = dir.join("reparse-target");
        std::fs::create_dir_all(&target_dir).unwrap();
        let target = target_dir.join("secret.json");
        let link = dir.join("live-104.txt");
        std::fs::write(&target, "{}").unwrap();
        create_reparse_point(&target, &link).expect(
            "reparse fixture creation failed; tests require file symlinks or NTFS directory junctions",
        );
        let link_metadata = std::fs::symlink_metadata(&link).unwrap();
        assert!(
            link_metadata.file_type().is_symlink() || metadata_is_reparse_point(&link_metadata),
            "fixture must be a symlink or Windows reparse point"
        );

        assert_eq!(
            read_text_file_at_from_dir(link.display().to_string(), &dir).unwrap_err(),
            "Only transcript text files can be read."
        );
        assert_eq!(
            read_text_preview_at_from_dir(link.display().to_string(), 10, &dir).unwrap_err(),
            "Only transcript text files can be read."
        );
        assert_eq!(
            write_polished_text_at_from_dir(link.display().to_string(), "safe".into(), &dir)
                .unwrap_err(),
            "Only transcript text files can be polished."
        );
        remove_reparse_point(&link).unwrap();
        std::fs::remove_dir_all(dir).ok();
    }

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
            openable_app_path_from(transcript.display().to_string(), &registry, &owned_dir)
                .is_err()
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
        assert!(write_polished_text_at_from_dir(
            transcript.display().to_string(),
            "no".into(),
            &dir
        )
        .is_err());
        assert!(openable_app_path_from(transcript.display().to_string(), &registry, &dir).is_err());
        assert!(openable_app_path_from(audio.display().to_string(), &registry, &dir).is_err());
        assert!(register_playback_path_at_from_owned_dir(
            audio.display().to_string(),
            &registry,
            &dir
        )
        .is_err());
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

    #[test]
    fn playback_registry_restores_registered_recordings() {
        let dir = temp_test_dir("playback-registry");
        let registry = dir.join("registry.json");
        let media = dir.join("meeting.wav");
        std::fs::write(&media, b"RIFF").unwrap();

        let registered = register_playback_path_at(media.display().to_string(), &registry).unwrap();
        let restored = registered_playback_path_at(media.display().to_string(), &registry).unwrap();

        assert_eq!(restored, registered);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn playback_admission_returns_revocable_url_and_exact_metadata() {
        let dir = temp_test_dir("playback-admission-metadata");
        let media = dir.join("meeting.wav");
        std::fs::write(&media, b"RIFFdata").unwrap();
        let owner = crate::media_protocol::MediaOwner::new();

        let admission = mint_playback_admission(&media.canonicalize().unwrap(), &owner).unwrap();

        assert!(admission.playback_path.starts_with("http://127.0.0.1:"));
        assert!(!admission.playback_path.contains("meeting.wav"));
        assert_eq!(admission.byte_length, "8");
        assert!(admission.waveform_eligible);
        assert!(owner.release(&admission.playback_path));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn playback_restore_revalidates_native_authorization() {
        let dir = temp_test_dir("playback-admission-revalidate");
        let registry = dir.join("registry.json");
        let owned = dir.join("owned");
        let media = dir.join("meeting.wav");
        std::fs::create_dir_all(&owned).unwrap();
        std::fs::write(&media, b"RIFF").unwrap();
        register_playback_path_at_from_owned_dir(media.display().to_string(), &registry, &owned)
            .unwrap();
        std::fs::remove_file(&media).unwrap();

        let error =
            restore_playback_path_at(media.display().to_string(), &registry, &owned).unwrap_err();

        assert_eq!(error, "File no longer exists.");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn playback_restore_ignores_pre_picker_general_registry_authority() {
        let dir = temp_test_dir("playback-pre-picker-authority");
        let legacy_registry = dir.join("recording-playback-registry.json");
        let native_job_registry = dir.join("recording-job-playback-registry.json");
        let owned = dir.join("owned");
        let media = dir.join("meeting.wav");
        std::fs::create_dir_all(&owned).unwrap();
        std::fs::write(&media, b"RIFF").unwrap();
        register_playback_path_at_from_owned_dir(
            media.display().to_string(),
            &legacy_registry,
            &owned,
        )
        .unwrap();

        assert!(restore_playback_path_at(
            media.display().to_string(),
            &native_job_registry,
            &owned,
        )
        .is_err());

        register_recording_job_playback_path_at_from_owned_dir(
            media.display().to_string(),
            &native_job_registry,
            &owned,
        )
        .unwrap();
        assert_eq!(
            restore_playback_path_at(media.display().to_string(), &native_job_registry, &owned,)
                .unwrap(),
            media.canonicalize().unwrap()
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn owned_playback_restore_keeps_one_canonical_path_during_catalog_validation() {
        let dir = temp_test_dir("owned-playback-stable-canonical-path");
        let registry = dir.join("registry.json");
        let owned = dir.join("owned");
        let nested = owned.join("nested");
        let media = owned.join("live-selected.wav");
        let requested = nested.join("..").join("live-selected.wav");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(&media, b"RIFF").unwrap();
        let canonical_media = media.canonicalize().unwrap();
        let mut validations = 0;

        let restored = restore_playback_path_at_with(
            requested.display().to_string(),
            &registry,
            &owned,
            |requested, requested_owned, require_transcript| {
                validations += 1;
                assert_eq!(requested_owned, owned);
                assert!(!require_transcript);
                assert_eq!(requested, canonical_media.as_path());
                Ok(requested.to_path_buf())
            },
        )
        .unwrap();

        assert_eq!(restored, canonical_media);
        assert_eq!(validations, 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn playback_registry_rejects_unregistered_recordings() {
        let dir = temp_test_dir("playback-registry-unregistered");
        let registry = dir.join("registry.json");
        let media = dir.join("meeting.wav");
        std::fs::write(&media, b"RIFF").unwrap();

        let error =
            registered_playback_path_at(media.display().to_string(), &registry).unwrap_err();

        assert_eq!(error, "Recording file is not registered for playback.");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn playback_registry_recovers_from_invalid_json() {
        let dir = temp_test_dir("playback-registry-invalid");
        let registry = dir.join("registry.json");
        let media = dir.join("meeting.wav");
        std::fs::write(&registry, "not-json").unwrap();
        std::fs::write(&media, b"RIFF").unwrap();

        register_playback_path_at(media.display().to_string(), &registry).unwrap();
        let restored = registered_playback_path_at(media.display().to_string(), &registry).unwrap();

        assert_eq!(restored.file_name().unwrap(), "meeting.wav");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn playback_registry_rejects_unsupported_versions() {
        let dir = temp_test_dir("playback-registry-version");
        let registry = dir.join("registry.json");
        let media = dir.join("meeting.wav");
        std::fs::write(&registry, r#"{"version":99,"paths":[]}"#).unwrap();
        std::fs::write(&media, b"RIFF").unwrap();

        let error = register_playback_path_at(media.display().to_string(), &registry).unwrap_err();

        assert!(error.contains("Unsupported playback registry version"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn pre_native_picker_registry_cannot_restore_playback_authority() {
        let dir = temp_test_dir("playback-registry-pre-native-picker");
        let registry = dir.join("registry.json");
        let media = dir.join("meeting.wav");
        std::fs::write(&media, b"RIFF").unwrap();
        std::fs::write(
            &registry,
            format!(
                r#"{{"version":1,"paths":[{}]}}"#,
                serde_json::to_string(&media.display().to_string()).unwrap()
            ),
        )
        .unwrap();

        let error =
            registered_playback_path_at(media.display().to_string(), &registry).unwrap_err();
        assert_eq!(error, "Recording file is not registered for playback.");

        register_playback_path_at(media.display().to_string(), &registry).unwrap();
        let rewritten: RecordingPlaybackRegistry =
            serde_json::from_str(&std::fs::read_to_string(&registry).unwrap()).unwrap();
        assert_eq!(rewritten.version, NATIVE_SELECTION_REGISTRY_VERSION);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn playback_registry_does_not_evict_trusted_paths_at_capacity() {
        let dir = temp_test_dir("playback-registry-capacity");
        let registry = dir.join("registry.json");
        let media = dir.join("new.wav");
        let paths = (0..MAX_REGISTERED_PLAYBACK_PATHS)
            .map(|index| dir.join(format!("registered-{index}.wav")))
            .collect::<Vec<_>>();
        write_registered_playback_paths(&registry, &paths).unwrap();
        std::fs::write(&media, b"RIFF").unwrap();

        let error = register_playback_path_at(media.display().to_string(), &registry).unwrap_err();

        assert!(error.contains("playback registry is full"));
        assert_eq!(
            read_registered_playback_paths(&registry).unwrap().len(),
            paths.len()
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn playback_registry_serializes_concurrent_registrations() {
        let dir = temp_test_dir("playback-registry-concurrent");
        let registry = dir.join("registry.json");
        let paths = (0..20)
            .map(|index| {
                let path = dir.join(format!("meeting-{index}.wav"));
                std::fs::write(&path, b"RIFF").unwrap();
                path
            })
            .collect::<Vec<_>>();

        let threads = paths
            .iter()
            .cloned()
            .map(|path| {
                let registry = registry.clone();
                std::thread::spawn(move || {
                    register_playback_path_at(path.display().to_string(), &registry)
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            thread.join().unwrap().unwrap();
        }
        assert_eq!(read_registered_playback_paths(&registry).unwrap().len(), 20);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn polished_path_writes_sibling_file() {
        let path = polished_path(std::path::Path::new("C:/recordings/take.txt")).unwrap();
        assert_eq!(path.file_name().unwrap(), "take.polished.txt");
    }

    #[test]
    fn atomic_text_write_replaces_stale_partial_file() {
        let dir = temp_test_dir("atomic-polish-write");
        let output = dir.join("take.polished.txt");
        let partial = dir.join("take.polished.txt.part");
        std::fs::write(&partial, "stale").unwrap();

        write_text_atomically(&output, "polished").unwrap();

        assert_eq!(std::fs::read_to_string(&output).unwrap(), "polished");
        assert!(!partial.exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn atomic_text_write_replaces_existing_output() {
        let dir = temp_test_dir("atomic-polish-overwrite");
        let output = dir.join("take.polished.txt");
        std::fs::write(&output, "old").unwrap();

        write_text_atomically(&output, "new").unwrap();

        assert_eq!(std::fs::read_to_string(&output).unwrap(), "new");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn atomic_text_write_uses_unique_temps_for_concurrent_writes() {
        let dir = temp_test_dir("atomic-polish-concurrent");
        let output = dir.join("take.polished.txt");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let left_output = output.clone();
        let left_barrier = std::sync::Arc::clone(&barrier);
        let left = std::thread::spawn(move || {
            left_barrier.wait();
            write_text_atomically(&left_output, "left")
        });
        let right_output = output.clone();
        let right_barrier = std::sync::Arc::clone(&barrier);
        let right = std::thread::spawn(move || {
            right_barrier.wait();
            write_text_atomically(&right_output, "right")
        });

        left.join().unwrap().unwrap();
        right.join().unwrap().unwrap();

        let text = std::fs::read_to_string(&output).unwrap();
        assert!(text == "left" || text == "right");
        let leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "part")
            })
            .count();
        assert_eq!(leftovers, 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    fn create_reparse_point(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_reparse_point(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        let target_dir = target.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "target has no parent")
        })?;
        let output = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target_dir)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))
        }
    }

    #[cfg(unix)]
    fn remove_reparse_point(link: &std::path::Path) -> std::io::Result<()> {
        std::fs::remove_file(link)
    }

    #[cfg(windows)]
    fn remove_reparse_point(link: &std::path::Path) -> std::io::Result<()> {
        std::fs::remove_dir(link)
    }
}
