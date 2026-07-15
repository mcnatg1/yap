mod registry;

use registry::{
    canonical_path_is_inside_owned_live_directory,
    registered_canonical_recording_path_at_with_limit, MAX_RECORDING_JOB_PLAYBACK_PATHS,
};

pub(crate) use registry::{
    authorize_openable_app_path, playable_recording_path,
    reconcile_recording_job_playback_paths_at, recording_job_playback_registry_path,
    recording_job_selection_registry_path, register_recording_job_playback_path_at_from_owned_dir,
    remove_recording_job_playback_path_at, restore_playback_path_at,
};

#[cfg(test)]
pub(crate) use registry::{
    is_yap_media_or_transcript_path, openable_app_path_from, openable_app_path_from_registries,
    read_registered_playback_paths, register_general_playback_path_at_for_test,
    register_playback_path_at, register_playback_path_at_from_owned_dir,
    registered_playback_path_at, restore_playback_path_at_with, write_registered_playback_paths,
    RecordingPlaybackRegistry, MAX_REGISTERED_PLAYBACK_PATHS, NATIVE_SELECTION_REGISTRY_VERSION,
};

pub(crate) const MAX_DECODED_WAVEFORM_BYTES: u64 = 32 * 1024 * 1024;

pub(crate) struct RecordingJobSourceAdmission {
    pub(crate) canonical_path: std::path::PathBuf,
    pub(crate) playback_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValidatedRecordingJobSource {
    pub(crate) canonical_path: std::path::PathBuf,
    pub(crate) fingerprint: crate::media_protocol::MediaSourceFingerprint,
}

#[derive(Debug)]
pub(crate) enum RecordingJobSourceError {
    Missing,
    Unsafe(String),
}

pub(crate) fn validate_recording_job_source_at(
    path: &std::path::Path,
    owned_dir: &std::path::Path,
) -> Result<ValidatedRecordingJobSource, RecordingJobSourceError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            RecordingJobSourceError::Missing
        } else {
            RecordingJobSourceError::Unsafe(format!("Failed to inspect recording source: {error}"))
        }
    })?;
    if !metadata.file_type().is_file() || metadata_is_reparse_point(&metadata) {
        return Err(RecordingJobSourceError::Unsafe(
            "Recording source must be a regular file and not a reparse point.".into(),
        ));
    }
    let canonical_path = playable_recording_path(path.display().to_string())
        .map_err(RecordingJobSourceError::Unsafe)?;
    let canonical_metadata = std::fs::symlink_metadata(&canonical_path)
        .map_err(|error| RecordingJobSourceError::Unsafe(error.to_string()))?;
    if !canonical_metadata.file_type().is_file() || metadata_is_reparse_point(&canonical_metadata) {
        return Err(RecordingJobSourceError::Unsafe(
            "Recording source must be a regular file and not a reparse point.".into(),
        ));
    }
    if canonical_path_is_inside_owned_live_directory(&canonical_path, owned_dir) {
        crate::live::recordings::canonical_committed_live_path_from_dir(
            &canonical_path,
            owned_dir,
            false,
        )
        .map_err(RecordingJobSourceError::Unsafe)?;
    }
    let fingerprint = crate::media_protocol::inspect_media_source(&canonical_path)
        .map_err(RecordingJobSourceError::Unsafe)?;
    Ok(ValidatedRecordingJobSource {
        canonical_path,
        fingerprint,
    })
}

pub(crate) fn register_native_selected_recording_job_source_at(
    source: &ValidatedRecordingJobSource,
    registry_path: &std::path::Path,
    owned_dir: &std::path::Path,
) -> Result<(), RecordingJobSourceError> {
    let canonical_path = register_recording_job_playback_path_at_from_owned_dir(
        source.canonical_path.display().to_string(),
        registry_path,
        owned_dir,
    )
    .map_err(RecordingJobSourceError::Unsafe)?;
    if canonical_path != source.canonical_path {
        return Err(RecordingJobSourceError::Unsafe(
            "Recording source changed while playback was being authorized.".into(),
        ));
    }
    Ok(())
}

pub(crate) fn authorize_registered_recording_job_source_at(
    source: &ValidatedRecordingJobSource,
    owner: &crate::media_protocol::MediaOwner,
    selection_registry_path: &std::path::Path,
    playback_registry_path: &std::path::Path,
    owned_dir: &std::path::Path,
) -> Result<RecordingJobSourceAdmission, RecordingJobSourceError> {
    let canonical_path =
        if canonical_path_is_inside_owned_live_directory(&source.canonical_path, owned_dir) {
            crate::live::recordings::canonical_committed_live_path_from_dir(
                &source.canonical_path,
                owned_dir,
                false,
            )
            .map_err(RecordingJobSourceError::Unsafe)?
        } else {
            registered_canonical_recording_path_at_with_limit(
                &source.canonical_path,
                selection_registry_path,
                MAX_RECORDING_JOB_PLAYBACK_PATHS,
            )
            .map_err(RecordingJobSourceError::Unsafe)?
        };
    if canonical_path != source.canonical_path {
        return Err(RecordingJobSourceError::Unsafe(
            "Recording source changed while playback authority was being restored.".into(),
        ));
    }
    let admission = owner
        .admit_unchanged(
            &canonical_path,
            &source.fingerprint,
            MAX_DECODED_WAVEFORM_BYTES,
        )
        .map_err(RecordingJobSourceError::Unsafe)?;
    let active_path = register_recording_job_playback_path_at_from_owned_dir(
        canonical_path.display().to_string(),
        playback_registry_path,
        owned_dir,
    )
    .map_err(|error| {
        owner.release(&admission.url);
        RecordingJobSourceError::Unsafe(error)
    })?;
    if active_path != canonical_path {
        owner.release(&admission.url);
        return Err(RecordingJobSourceError::Unsafe(
            "Recording source changed while active playback authority was being restored.".into(),
        ));
    }
    Ok(RecordingJobSourceAdmission {
        canonical_path,
        playback_path: admission.url,
    })
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(test)]
pub(crate) fn metadata_is_reparse_point_for_test(metadata: &std::fs::Metadata) -> bool {
    metadata_is_reparse_point(metadata)
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}
