use std::path::Path;
use std::time::Duration;

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::audio::recording::{self, RecordingFinalizeResult};
use crate::audio::results::ResultStatus;
use crate::live;

mod artifacts;
mod catalog;
mod deletion;
mod mutation_ownership;
mod recovery;
mod retention;
mod save;
mod transcripts;

use artifacts::committed_session_output_path;
pub(crate) use artifacts::{
    canonical_committed_live_path_from_dir, open_committed_live_transcript_from_dir,
};
pub(crate) use catalog::{list_history_sources, recordings_dir};
#[cfg(test)]
use catalog::{
    list_history_sources_from_dir_at_with_queue_observer, list_session_catalog_from_dir,
    list_session_catalog_from_dir_at_with_queue_observer, list_session_files_from_dir,
    list_session_files_from_dir_at, recordings_dir_from,
};
use deletion::delete_saved_live_session_in_dir;
#[cfg(test)]
use deletion::*;

use mutation_ownership::{
    session_mutation_ownership, session_mutation_ownership_with_queue_observer,
};

#[cfg(test)]
use recovery::*;
use recovery::{delete_recoverable_live_session_in_dir, recover_live_session_in_dir};
#[cfg(test)]
pub(crate) use save::save_finalized_capture_to_dir_for_test;
pub use save::save_session_files;
pub(crate) use save::save_stop_result;
#[cfg(test)]
use save::*;

pub(crate) use transcripts::{
    completed_transcript_text, is_primary_live_transcript_path, is_transcript_path,
    stable_existing_path_string, transcript_text, unix_millis_now,
};
#[cfg(test)]
use transcripts::{
    has_valid_transcript_revision, partial_text_path, stable_path_string, transcript_revision_path,
    write_transcript_revision_with_barrier, TranscriptRevisionPublicationBarrier,
};
use transcripts::{
    system_time_to_unix_millis, transcript_artifact_names, write_new_text_file,
    write_new_text_file_with, write_transcript_revision,
};

const AUDIO_SAVE_FAILED_WARNING: &str = "Live audio could not be saved. Transcript was saved.";
const TRANSCRIPT_DEGRADED_WARNING: &str = "Live transcript may be incomplete. Audio was saved.";
const PARTIAL_RECOVERY_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedLiveSession {
    pub session_id: String,
    pub name: String,
    pub source_path: String,
    pub output_path: String,
    pub created_at_ms: u64,
    pub warning: Option<String>,
    pub capture_commit_path: Option<String>,
    pub recovery_state: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedLiveSessionCatalog {
    pub sessions: Vec<SavedLiveSession>,
    pub maintenance_warnings: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableLiveSession {
    pub session_id: String,
    pub name: String,
    pub audio_partial_path: Option<String>,
    pub journal_partial_path: Option<String>,
    pub reason: String,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct LiveHistorySourceCatalog {
    pub(crate) saved: SavedLiveSessionCatalog,
    pub(crate) recoverable: Vec<RecoverableLiveSession>,
}

pub fn recover_live_session(
    session_id: String,
    expected_artifact_path: String,
) -> Result<SavedLiveSession, String> {
    recover_live_session_in_dir(&recordings_dir(), session_id, expected_artifact_path)
}

pub fn delete_recoverable_live_session(
    session_id: String,
    expected_artifact_path: String,
) -> Result<(), String> {
    delete_recoverable_live_session_in_dir(&recordings_dir(), session_id, expected_artifact_path)
}

pub fn delete_saved_live_session(
    session_id: String,
    expected_output_path: String,
    expected_capture_commit_path: String,
) -> Result<(), String> {
    delete_saved_live_session_in_dir(
        &recordings_dir(),
        session_id,
        expected_output_path,
        expected_capture_commit_path,
    )
}

fn committed_at_ms(value: &str) -> u64 {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .and_then(|timestamp| u64::try_from(timestamp.unix_timestamp_nanos() / 1_000_000).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
