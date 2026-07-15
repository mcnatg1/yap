use std::{
    io::Read,
    sync::{Arc, OnceLock},
    time::Duration,
};

use serde::Serialize;
use tokio::{sync::Semaphore, time::timeout_at};

use crate::atomic_text::write as write_text_atomically;

pub(super) const MAX_TRANSCRIPT_READ_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const MAX_HIDDEN_PRUNE_CANDIDATES: usize = 200;
const MAX_CONCURRENT_TRANSCRIPT_READS: usize = 4;
const TRANSCRIPT_READ_TIMEOUT: Duration = Duration::from_secs(8);
const TRANSCRIPT_WRITE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedLiveTranscriptPathResolution {
    pub(super) requested_path: String,
    pub(super) canonical_path: Option<String>,
    pub(super) missing: bool,
}

#[tauri::command]
pub fn resolve_owned_live_transcript_paths(
    window: tauri::WebviewWindow,
    output_paths: Vec<String>,
) -> Result<Vec<OwnedLiveTranscriptPathResolution>, String> {
    super::ensure_main_window(&window)?;
    resolve_owned_live_transcript_paths_from_dir(
        output_paths,
        &crate::live::recordings::recordings_dir(),
    )
}

pub(super) fn resolve_owned_live_transcript_paths_from_dir(
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
    super::ensure_main_window(&window)?;
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
    super::ensure_main_window(&window)?;
    let max_chars = max_chars.unwrap_or(600);
    run_bounded_transcript_io(
        transcript_read_limiter(),
        TRANSCRIPT_READ_TIMEOUT,
        "Transcript preview",
        move || read_text_preview_at(path, max_chars),
    )
    .await
}

pub(super) fn read_text_file_at(path: String) -> Result<String, String> {
    read_text_file_at_from_dir(path.clone(), &crate::live::recordings::recordings_dir())
        .or_else(|_| crate::jobs::read_published_remote_transcript(std::path::Path::new(&path)))
}

pub(super) fn read_text_file_at_from_dir(
    path: String,
    owned_dir: &std::path::Path,
) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);
    let mut file = owned_live_transcript_file_from_dir(&path, "read", owned_dir)?;
    reject_oversized_transcript_file(&file)?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|err| format!("Failed to read transcript: {err}"))?;
    Ok(text)
}

pub(super) fn read_text_preview_at(path: String, max_chars: usize) -> Result<String, String> {
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

pub(super) fn read_text_preview_at_from_dir(
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
    super::ensure_main_window(&window)?;
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

pub(super) async fn run_bounded_transcript_io<F>(
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

pub(super) fn write_polished_text_at(path: String, text: String) -> Result<String, String> {
    write_polished_text_at_from_dir(path, text, &crate::live::recordings::recordings_dir())
}

pub(super) fn write_polished_text_at_from_dir(
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

pub(super) fn polished_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| "Transcript path has no file name.".to_string())?;

    Ok(path.with_file_name(format!("{stem}.polished.txt")))
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

pub(super) fn owned_live_transcript_path_from_dir(
    path: &std::path::Path,
    action: &str,
    owned_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let path = canonical_transcript_path(path, action)?;
    crate::live::recordings::canonical_committed_live_path_from_dir(&path, owned_dir, true)
        .map_err(|_| format!("Only Yap-owned canonical live transcripts can be {action}."))
}

pub(super) fn owned_live_transcript_file_from_dir(
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
