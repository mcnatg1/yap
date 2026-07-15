use serde::{Deserialize, Serialize};

mod admission;
mod path_policy;
mod persistence;

#[cfg(test)]
pub(crate) const MAX_REGISTERED_PLAYBACK_PATHS: usize = 500;
pub(super) const MAX_RECORDING_JOB_PLAYBACK_PATHS: usize = 200;
pub(crate) const NATIVE_SELECTION_REGISTRY_VERSION: u32 = 2;

#[derive(Deserialize, Serialize)]
pub(crate) struct RecordingPlaybackRegistry {
    pub(crate) version: u32,
    pub(crate) paths: Vec<String>,
}

pub(super) use admission::registered_canonical_recording_path_at_with_limit;
pub(crate) use admission::{authorize_openable_app_path, restore_playback_path_at};
#[cfg(test)]
pub(crate) use admission::{
    openable_app_path_from, openable_app_path_from_registries, registered_playback_path_at,
    restore_playback_path_at_with,
};

pub(super) use path_policy::canonical_path_is_inside_owned_live_directory;
#[cfg(test)]
pub(crate) use path_policy::is_yap_media_or_transcript_path;
pub(crate) use path_policy::playable_recording_path;

#[cfg(test)]
pub(crate) use persistence::{
    read_registered_playback_paths, register_general_playback_path_at_for_test,
    register_playback_path_at, register_playback_path_at_from_owned_dir,
    write_registered_playback_paths,
};
pub(crate) use persistence::{
    reconcile_recording_job_playback_paths_at, recording_job_playback_registry_path,
    recording_job_selection_registry_path, register_recording_job_playback_path_at_from_owned_dir,
    remove_recording_job_playback_path_at,
};
