use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Arc;
use std::sync::{mpsc, Condvar, Mutex};
use std::thread::JoinHandle;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
#[cfg(windows)]
use windows::Win32::Foundation::HANDLE;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};

use crate::audio::coordinator::{BoundedReceiver, BoundedSink, SinkDegradeResult};
use crate::audio::frame::{AudioGap, GapCause, PreparedFrame, TrackConfigurationRevision};
use crate::audio::preprocess::f32_to_i16_le_bytes;
use crate::audio::session::{
    self, SessionId, SessionMetadata, SessionMode, SessionOrigin, TriggerMode,
};
use crate::audio::timeline::{
    ClockMappingRevision, RecordingInput, RecordingRevisionTransition, SessionClock,
};

const CAPTURE_SCHEMA_VERSION: u16 = 1;
const WAV_HEADER_BYTES: u64 = 44;
const PCM16_BYTES_PER_SAMPLE: u64 = 2;
const DEFAULT_SYNC_INTERVAL_SAMPLES: u64 = 16_000;
const MAX_SEQUENCE_GAP_DETAILS: usize = 1_024;
const MAX_TIMELINE_CONTROL_EVENTS: usize = 1_024;
const MAX_JOURNAL_BYTES: u64 = 512 * 1024;
const MAX_JOURNAL_RECORD_BYTES: u64 = 8 * 1024;
const MAX_JOURNAL_TERMINAL_BYTES: u64 = 256;

static DELETE_QUARANTINE_COUNTER: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
type SidecarPublishHook = Box<dyn FnOnce(&RecordingPaths) + Send>;
#[cfg(test)]
type PublicationHook =
    Box<dyn FnMut(PublicationArtifact, PublicationBarrier, &RecordingPaths) + Send>;

mod model;

pub(crate) use model::PublishedTranscriptReceipt;
pub use model::{
    CaptureCommitManifest, CaptureStatus, CommittedCapture, DamagedCommittedCapture,
    PartialCapture, PartialCaptureLineage, PartialRecoveryCommit, RecordingFinalizeResult,
    RecordingScan, RecoveredPartialCapture,
};
mod worker;

#[cfg(test)]
use worker::drain_recording_worker;
pub use worker::RecordingSinkHandle;
mod sidecar_validation;

use sidecar_validation::*;
mod journal_state;

use journal_state::*;
mod journal_delta;

use journal_delta::*;

mod stream_state;

use stream_state::default_session_metadata;
pub use stream_state::StreamingRecording;
mod artifact_identity;

pub(crate) use artifact_identity::{FileIdentity, QuarantinedArtifact, RegularArtifactIdentity};
use artifact_identity::{PublicationArtifact, PublicationBarrier, PublicationReceipt};

mod artifact_io;

use artifact_io::{
    file_identity, file_link_count, has_owned_partial_lineage, open_no_follow,
    open_regular_artifact_for_update, open_regular_path, read_open_file, same_file_identity,
    session_from_orphan_wav_artifact, session_from_private_artifact, validate_recoverable_wav,
};
pub(crate) use artifact_io::{
    read_and_hash_regular_artifact, read_regular_artifact, sha256_regular_artifact,
};

mod faults;

pub(crate) use faults::CommitFaultPoint;

mod reservation;

use reservation::RecordingPaths;
pub(crate) use reservation::{allocate_recording_session, RecordingReservation};
#[cfg(test)]
use reservation::{
    reserve_wav_part, reserve_wav_part_with_before_claim, ReservationHandleDropSignal,
};

mod stream_append;
mod stream_create;
mod stream_finalize;
mod stream_publish;

mod publication_io;

use publication_io::create_new;
#[cfg(test)]
pub(crate) use publication_io::publish_no_replace_with_after_link_for_test;
pub(crate) use publication_io::{publish_no_replace, remove_owned_staging};

mod journal_io;

pub(crate) use journal_io::parse_journal_for_session;
#[cfg(test)]
use journal_io::{parse_journal_append_log, read_journal_snapshot};
use journal_io::{
    read_journal_append_log, serialize_journal_record, write_journal_record, write_json_file_open,
    write_wav_header,
};

mod integrity;

use integrity::{
    manifest_from_published_commit, now_utc, read_manifest, receipt_from_published_partial_sidecar,
    receipt_from_published_sidecar, sha256_open_file, sync_parent_directory, validate_sha256,
};
pub use integrity::{sha256_file, validate_artifact_name};
pub(crate) use integrity::{sha256_open_regular_file, sync_recordings_parent};

mod scan;

pub use scan::scan_recordings;
pub(crate) use scan::{
    is_regular_artifact, open_regular_artifact, recover_partial_wav_with_identity,
};

mod artifact_admission;

use artifact_admission::remove_open_regular_artifact;
#[cfg(test)]
pub(crate) use artifact_admission::remove_regular_artifact_if_hash;
#[cfg(test)]
use artifact_admission::remove_regular_artifact_with_barrier_for_test;
pub(crate) use artifact_admission::{
    admit_expected_private_regular_artifact, admit_expected_regular_artifact,
    admit_regular_artifact, quarantine_regular_artifact, remove_regular_artifact,
    remove_regular_artifact_if_file_identity_and_hash,
    remove_regular_artifact_if_identity_and_hash, remove_verified_quarantined_artifact,
    restore_verified_quarantined_artifact, revalidate_regular_artifact_file_identity_and_hash,
    revalidate_regular_artifact_identity, verified_regular_artifact,
};

#[cfg(test)]
mod tests;
