use serde::Serialize;

pub(crate) mod transcripts;

#[cfg(test)]
use crate::atomic_text::write as write_text_atomically;
#[cfg(test)]
use std::{sync::Arc, time::Duration};
#[cfg(test)]
use tokio::sync::Semaphore;
#[cfg(test)]
use transcripts::{
    polished_path, read_text_file_at, read_text_file_at_from_dir, read_text_preview_at_from_dir,
    resolve_owned_live_transcript_paths_from_dir, run_bounded_transcript_io,
    write_polished_text_at_from_dir, OwnedLiveTranscriptPathResolution,
    MAX_HIDDEN_PRUNE_CANDIDATES, MAX_TRANSCRIPT_READ_BYTES,
};

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

pub(crate) fn ensure_main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if crate::authorization::is_main_window(window.label()) {
        Ok(())
    } else {
        Err("This file action is only available from the main window.".into())
    }
}

#[cfg(test)]
mod tests;
