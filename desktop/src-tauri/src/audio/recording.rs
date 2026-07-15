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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStatus {
    Complete,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureCommitManifest {
    pub schema_version: u16,
    pub session_id: SessionId,
    pub status: CaptureStatus,
    pub audio_file: String,
    pub audio_sha256: String,
    pub audio_bytes: u64,
    pub capture_sidecar_file: String,
    pub capture_sidecar_sha256: String,
    pub committed_at_utc: String,
    #[serde(default)]
    pub session_metadata: Option<SessionMetadata>,
}

impl CaptureCommitManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CAPTURE_SCHEMA_VERSION {
            return Err("unsupported capture commit schema".into());
        }
        if self.status != CaptureStatus::Complete {
            return Err("partial captures must not be published as committed history".into());
        }
        validate_artifact_name(&self.audio_file)?;
        validate_artifact_name(&self.capture_sidecar_file)?;
        if self.audio_file != format!("live-{}.wav", self.session_id)
            || self.capture_sidecar_file != format!("live-{}.capture.json", self.session_id)
        {
            return Err("capture manifest artifact names do not match the session".into());
        }
        validate_sha256(&self.audio_sha256)?;
        validate_sha256(&self.capture_sidecar_sha256)?;
        if self.audio_bytes < WAV_HEADER_BYTES {
            return Err("capture manifest audio is shorter than a WAV header".into());
        }
        OffsetDateTime::parse(&self.committed_at_utc, &Rfc3339)
            .map_err(|_| "capture manifest has an invalid commit timestamp")?;
        if let Some(metadata) = &self.session_metadata {
            validate_capture_metadata(metadata, &self.session_id)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedCapture {
    pub manifest: CaptureCommitManifest,
    pub directory: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialCapture {
    pub session_id: Option<SessionId>,
    pub directory: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredPartialCapture {
    pub session_id: SessionId,
    pub directory: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamagedCommittedCapture {
    pub session_id: SessionId,
    pub directory: PathBuf,
    pub reason: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecordingScan {
    pub complete: Vec<CommittedCapture>,
    pub partial: Vec<PartialCapture>,
    pub recovered_partial: Vec<RecoveredPartialCapture>,
    pub damaged: Vec<DamagedCommittedCapture>,
}

impl RecordingScan {
    pub fn is_empty(&self) -> bool {
        self.complete.is_empty()
            && self.partial.is_empty()
            && self.recovered_partial.is_empty()
            && self.damaged.is_empty()
    }

    pub fn len(&self) -> usize {
        self.complete.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialRecoveryCommit {
    pub schema_version: u16,
    pub session_id: SessionId,
    pub status: CaptureStatus,
    pub audio_file: String,
    pub audio_sha256: String,
    pub audio_bytes: u64,
    pub capture_sidecar_file: String,
    pub capture_sidecar_sha256: String,
    pub committed_at_utc: String,
}

impl PartialRecoveryCommit {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != CAPTURE_SCHEMA_VERSION
            || self.status != CaptureStatus::Partial
            || self.audio_file != format!("live-{}.wav", self.session_id)
            || self.capture_sidecar_file != format!("live-{}.capture.partial.json", self.session_id)
            || self.audio_bytes < WAV_HEADER_BYTES
        {
            return Err("partial recovery commit has an unsupported shape".into());
        }
        validate_artifact_name(&self.audio_file)?;
        validate_artifact_name(&self.capture_sidecar_file)?;
        validate_sha256(&self.audio_sha256)?;
        validate_sha256(&self.capture_sidecar_sha256)?;
        OffsetDateTime::parse(&self.committed_at_utc, &Rfc3339)
            .map_err(|_| "partial recovery commit has an invalid commit timestamp")?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialCaptureLineage {
    pub capture_sidecar_file: String,
    pub capture_sidecar_sha256: String,
}

#[derive(Debug, Clone)]
pub struct RecordingFinalizeResult {
    pub session_id: SessionId,
    pub status: CaptureStatus,
    pub committed: Option<CommittedCapture>,
    pub partial_lineage: Option<PartialCaptureLineage>,
    pub error: Option<String>,
    sidecar_receipt: Option<PublicationReceipt>,
}

#[derive(Debug, Clone)]
pub(crate) struct PublishedTranscriptReceipt {
    file_name: String,
    sha256: String,
    path: PathBuf,
    identity: FileIdentity,
}

impl PublishedTranscriptReceipt {
    pub(crate) fn from_verified_destination(
        destination: &Path,
        mut file: File,
    ) -> Result<Self, String> {
        let file_name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "published transcript has no valid file name".to_string())?
            .to_string();
        validate_artifact_name(&file_name)?;
        let sha256 = sha256_open_file(&mut file)?;
        let identity = file_identity(&file)?;
        drop(file);
        Ok(Self {
            file_name,
            sha256,
            path: destination.to_path_buf(),
            identity,
        })
    }

    pub(crate) fn file_name(&self) -> &str {
        &self.file_name
    }

    pub(crate) fn sha256(&self) -> &str {
        &self.sha256
    }

    pub(crate) fn revalidate(&self) -> Result<(), String> {
        let mut current = open_regular_path(&self.path)?;
        if self.identity != file_identity(&current)? {
            return Err("transcript path no longer names the verified destination".into());
        }
        if sha256_open_file(&mut current)? != self.sha256 {
            return Err("transcript path no longer matches the verified destination hash".into());
        }
        Ok(())
    }
}

impl PartialEq for RecordingFinalizeResult {
    fn eq(&self, other: &Self) -> bool {
        self.session_id == other.session_id
            && self.status == other.status
            && self.committed == other.committed
            && self.partial_lineage == other.partial_lineage
            && self.error == other.error
    }
}

impl Eq for RecordingFinalizeResult {}

impl RecordingFinalizeResult {
    pub fn capture_sidecar_sha256(&self) -> Option<&str> {
        self.committed
            .as_ref()
            .map(|capture| capture.manifest.capture_sidecar_sha256.as_str())
            .or_else(|| {
                self.partial_lineage
                    .as_ref()
                    .map(|lineage| lineage.capture_sidecar_sha256.as_str())
            })
    }

    pub(crate) fn revalidate_capture_sidecar(&self) -> Result<(), String> {
        self.sidecar_receipt
            .as_ref()
            .ok_or_else(|| {
                "Capture lineage is unavailable for the transcript revision".to_string()
            })?
            .revalidate()
    }
}

pub struct RecordingSinkHandle {
    sink: BoundedSink<RecordingInput>,
    session_id: SessionId,
    state: Mutex<RecordingSinkState>,
    completed: Condvar,
    #[cfg(test)]
    finalization_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

struct RecordingSinkState {
    worker: Option<JoinHandle<RecordingFinalizeResult>>,
    result: Option<Result<RecordingFinalizeResult, String>>,
    finalizing: bool,
}

impl RecordingSinkHandle {
    pub fn spawn(
        directory: PathBuf,
        session_id: SessionId,
        sink: BoundedSink<RecordingInput>,
        receiver: BoundedReceiver<RecordingInput>,
    ) -> Self {
        Self::spawn_inner(directory, session_id, sink, receiver, None)
    }

    pub(crate) fn spawn_reserved(
        reservation: RecordingReservation,
        sink: BoundedSink<RecordingInput>,
        receiver: BoundedReceiver<RecordingInput>,
    ) -> Self {
        let directory = reservation.paths.directory.clone();
        let session_id = reservation.session_id().clone();
        Self::spawn_inner(directory, session_id, sink, receiver, Some(reservation))
    }

    fn spawn_inner(
        directory: PathBuf,
        session_id: SessionId,
        sink: BoundedSink<RecordingInput>,
        receiver: BoundedReceiver<RecordingInput>,
        reservation: Option<RecordingReservation>,
    ) -> Self {
        let worker_session_id = session_id.clone();
        let worker_sink = sink.clone();
        let worker = std::thread::spawn(move || {
            run_recording_worker(
                directory,
                worker_session_id,
                receiver,
                reservation,
                worker_sink,
            )
        });
        Self::with_worker(sink, session_id, worker)
    }

    fn with_worker(
        sink: BoundedSink<RecordingInput>,
        session_id: SessionId,
        worker: JoinHandle<RecordingFinalizeResult>,
    ) -> Self {
        Self {
            sink,
            session_id,
            state: Mutex::new(RecordingSinkState {
                worker: Some(worker),
                result: None,
                finalizing: false,
            }),
            completed: Condvar::new(),
            #[cfg(test)]
            finalization_count: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    #[cfg(test)]
    pub(crate) fn spawn_with_fault_for_test(
        directory: PathBuf,
        session_id: SessionId,
        sink: BoundedSink<RecordingInput>,
        receiver: BoundedReceiver<RecordingInput>,
        fault: CommitFaultPoint,
        append_write_attempts: Arc<std::sync::atomic::AtomicUsize>,
        journal_write_attempts: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        let worker_session_id = session_id.clone();
        let worker_sink = sink.clone();
        let worker = std::thread::spawn(move || {
            let mut recording = match StreamingRecording::create_with_fault(
                &directory,
                worker_session_id.clone(),
                fault,
            ) {
                Ok(recording) => recording,
                Err(error) => return worker_creation_failure(worker_session_id, error),
            };
            recording.append_write_attempts = Some(append_write_attempts);
            recording.journal_write_attempts = Some(journal_write_attempts);
            recording.sync_interval_samples = 1;
            drain_recording_worker(recording, worker_session_id, receiver, worker_sink)
        });
        Self::with_worker(sink, session_id, worker)
    }

    #[cfg(test)]
    pub(crate) fn spawn_panicking_for_test(
        sink: BoundedSink<RecordingInput>,
        _receiver: BoundedReceiver<RecordingInput>,
        session_id: SessionId,
    ) -> Self {
        Self::with_worker(
            sink,
            session_id,
            std::thread::spawn(|| -> RecordingFinalizeResult {
                panic!("injected recording worker panic")
            }),
        )
    }

    #[cfg(test)]
    pub(crate) fn spawn_unavailable_for_test(
        sink: BoundedSink<RecordingInput>,
        session_id: SessionId,
    ) -> Self {
        Self {
            sink,
            session_id,
            state: Mutex::new(RecordingSinkState {
                worker: None,
                result: None,
                finalizing: false,
            }),
            completed: Condvar::new(),
            finalization_count: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    #[cfg(test)]
    pub(crate) fn spawn_with_finalization_counter_for_test(
        directory: PathBuf,
        session_id: SessionId,
        sink: BoundedSink<RecordingInput>,
        receiver: BoundedReceiver<RecordingInput>,
    ) -> (Self, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let handle = Self::spawn(directory, session_id, sink, receiver);
        let count = std::sync::Arc::clone(&handle.finalization_count);
        (handle, count)
    }

    pub fn sink(&self) -> BoundedSink<RecordingInput> {
        self.sink.clone()
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn finalize(&self) -> Result<RecordingFinalizeResult, String> {
        self.sink.close();
        let worker = loop {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "recording handle became unavailable")?;
            if let Some(result) = &state.result {
                return result.clone();
            }
            if !state.finalizing {
                state.finalizing = true;
                #[cfg(test)]
                self.finalization_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                break state.worker.take();
            }
            state = self
                .completed
                .wait(state)
                .map_err(|_| "recording handle became unavailable")?;
            drop(state);
        };
        let result = match worker {
            Some(worker) => worker
                .join()
                .map_err(|_| "recording worker panicked during finalization".to_string()),
            None => Err("recording worker is unavailable".to_string()),
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.sink.mark_published();
        state.finalizing = false;
        state.result = Some(result.clone());
        self.completed.notify_all();
        result
    }

    pub fn abort(&self, reason: impl Into<String>) -> Result<RecordingFinalizeResult, String> {
        match self.sink.degrade(&reason.into()) {
            SinkDegradeResult::Accepted => self.finalize(),
            SinkDegradeResult::CompletionInProgress => {
                Err("recording completion is already in progress".into())
            }
            SinkDegradeResult::Published => self.finalize(),
        }
    }
}

fn run_recording_worker(
    directory: PathBuf,
    session_id: SessionId,
    receiver: BoundedReceiver<RecordingInput>,
    reservation: Option<RecordingReservation>,
    sink: BoundedSink<RecordingInput>,
) -> RecordingFinalizeResult {
    let recording = match reservation {
        Some(reservation) => StreamingRecording::create_reserved(reservation),
        None => StreamingRecording::create(&directory, session_id.clone()),
    };
    let recording = match recording {
        Ok(recording) => recording,
        Err(error) => return worker_creation_failure(session_id, error),
    };
    drain_recording_worker(recording, session_id, receiver, sink)
}

fn worker_creation_failure(session_id: SessionId, error: String) -> RecordingFinalizeResult {
    RecordingFinalizeResult {
        session_id,
        status: CaptureStatus::Partial,
        committed: None,
        partial_lineage: None,
        error: Some(error),
        sidecar_receipt: None,
    }
}

fn drain_recording_worker(
    mut recording: StreamingRecording,
    session_id: SessionId,
    receiver: BoundedReceiver<RecordingInput>,
    sink: BoundedSink<RecordingInput>,
) -> RecordingFinalizeResult {
    let mut input_failed = false;
    loop {
        match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(input) => {
                if !input_failed {
                    if let Err(error) = recording.append_input(input) {
                        sink.degrade(&error);
                        recording.abort(error);
                        input_failed = true;
                    }
                    if recording.journal.sink_degraded {
                        sink.degrade("recording sequence discontinuity");
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if let Some(reason) = sink.begin_completion() {
        recording.abort(reason);
    }
    recording
        .finalize()
        .unwrap_or_else(|error| RecordingFinalizeResult {
            session_id,
            status: CaptureStatus::Partial,
            committed: None,
            partial_lineage: None,
            error: Some(error),
            sidecar_receipt: None,
        })
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureSidecar {
    schema_version: u16,
    session_id: SessionId,
    audio_file: String,
    audio_sha256: String,
    audio_bytes: u64,
    tracks: Vec<JournalTrack>,
    track_configurations: Vec<TrackConfigurationRevision>,
    clock_mappings: Vec<ClockMappingRevision>,
    timeline_gaps: Vec<AudioGap>,
    sequence_coverage: Vec<SequenceCoverage>,
    sequence_gaps: Vec<SequenceGap>,
    #[serde(default)]
    sequence_gap_overflow: Option<SequenceGapOverflow>,
    sink_degraded: bool,
    directory_sync_supported: bool,
    #[serde(default)]
    session_metadata: Option<SessionMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartialCaptureSidecar {
    schema_version: u16,
    session_id: SessionId,
    status: CaptureStatus,
}

impl CaptureSidecar {
    fn validate(&self, manifest: &CaptureCommitManifest) -> Result<(), String> {
        if self.schema_version != CAPTURE_SCHEMA_VERSION
            || self.session_id != manifest.session_id
            || self.audio_file != manifest.audio_file
            || self.audio_sha256 != manifest.audio_sha256
            || self.audio_bytes != manifest.audio_bytes
        {
            return Err("capture sidecar does not match the commit manifest".into());
        }
        if self.session_metadata != manifest.session_metadata {
            return Err("capture session metadata does not match the commit manifest".into());
        }
        if let Some(metadata) = &self.session_metadata {
            validate_capture_metadata(metadata, &self.session_id)?;
        }
        validate_audio_metadata_presence(self.audio_bytes, &self.tracks)?;
        validate_timeline_control_metadata(
            &self.session_id,
            &self.tracks,
            &self.track_configurations,
            &self.clock_mappings,
            &self.timeline_gaps,
        )?;
        validate_sequence_metadata(
            &self.tracks,
            &self.sequence_coverage,
            &self.sequence_gaps,
            self.sequence_gap_overflow.as_ref(),
            self.sink_degraded,
        )?;
        validate_artifact_name(&self.audio_file)?;
        validate_sha256(&self.audio_sha256)
    }
}

fn validate_audio_metadata_presence(
    audio_bytes: u64,
    tracks: &[JournalTrack],
) -> Result<(), String> {
    if audio_bytes > WAV_HEADER_BYTES && tracks.is_empty() {
        return Err("nonempty recording audio has no frame metadata".into());
    }
    Ok(())
}

fn validate_timeline_control_metadata<'a>(
    session_id: &SessionId,
    tracks: impl IntoIterator<Item = &'a JournalTrack>,
    track_configurations: &[TrackConfigurationRevision],
    clock_mappings: &[ClockMappingRevision],
    timeline_gaps: &[AudioGap],
) -> Result<(), String> {
    if track_configurations.len() > MAX_TIMELINE_CONTROL_EVENTS
        || clock_mappings.len() > MAX_TIMELINE_CONTROL_EVENTS
        || timeline_gaps.len() > MAX_TIMELINE_CONTROL_EVENTS
    {
        return Err("recording timeline metadata exceeds its fixed bound".into());
    }

    let mut recorded_tracks = BTreeSet::new();
    for track in tracks {
        if track.sample_rate_hz == 0
            || track.channels == 0
            || !recorded_tracks.insert(track.track_id.clone())
        {
            return Err("recording track metadata is invalid".into());
        }
    }

    let mut configurations = BTreeMap::<String, (u32, u64)>::new();
    let mut configuration_revisions = BTreeMap::<(String, u32), (u64, u32)>::new();
    for configuration in track_configurations {
        let track = configuration.track_id.as_str().to_string();
        if configuration.revision == 0 || configuration.sample_rate_hz == 0 {
            return Err("recording track configuration is invalid".into());
        }
        match configurations.get(&track) {
            Some((revision, effective_at_ms))
                if revision.checked_add(1) == Some(configuration.revision)
                    && configuration.effective_at_ms >= *effective_at_ms => {}
            None if configuration.revision == 1 => {}
            _ => return Err("recording track configuration revisions are not contiguous".into()),
        }
        configurations.insert(
            track.clone(),
            (configuration.revision, configuration.effective_at_ms),
        );
        configuration_revisions.insert(
            (track, configuration.revision),
            (configuration.effective_at_ms, configuration.sample_rate_hz),
        );
    }

    let mut mappings = BTreeMap::<String, (u32, u64, u64)>::new();
    let mut revision_clocks = BTreeMap::<String, Vec<(ClockMappingRevision, u32)>>::new();
    for mapping in clock_mappings {
        let track = mapping.track_id.as_str().to_string();
        if !configurations.contains_key(&track) || mapping.revision == 0 {
            return Err("recording clock mapping has no valid track configuration".into());
        }
        let Some((effective_at_ms, sample_rate_hz)) =
            configuration_revisions.get(&(track.clone(), mapping.revision))
        else {
            return Err("recording clock mapping has no matching configuration revision".into());
        };
        if *effective_at_ms != mapping.session_time_ms {
            return Err("recording revision transition timestamp does not match".into());
        }
        match mappings.get(&track) {
            Some((revision, source_position_frames, session_time_ms))
                if revision.checked_add(1) == Some(mapping.revision)
                    && mapping.source_position_frames >= *source_position_frames
                    && mapping.session_time_ms >= *session_time_ms => {}
            None if mapping.revision == 1 => {}
            _ => return Err("recording clock mapping revisions are not contiguous".into()),
        }
        mappings.insert(
            track.clone(),
            (
                mapping.revision,
                mapping.source_position_frames,
                mapping.session_time_ms,
            ),
        );
        revision_clocks
            .entry(track)
            .or_default()
            .push((mapping.clone(), *sample_rate_hz));
    }

    for track in &recorded_tracks {
        if !configurations.contains_key(track) || !mappings.contains_key(track) {
            return Err("recording track has no complete coordinator revision coverage".into());
        }
    }
    for (track, (configuration_revision, _)) in &configurations {
        if mappings.get(track).map(|(revision, _, _)| revision) != Some(configuration_revision) {
            return Err("recording track configuration has no matching clock mapping".into());
        }
    }

    let mut gaps = BTreeMap::<String, (u64, u64, u64, GapCause)>::new();
    for gap in timeline_gaps {
        let track = gap.track_id.as_str().to_string();
        if gap.session_id != *session_id
            || !configurations.contains_key(&track)
            || gap.duration_ms == 0
            || gap.dropped_frames == 0
        {
            return Err("recording timeline gap is invalid".into());
        }
        let end_ms = gap
            .start_ms
            .checked_add(u64::from(gap.duration_ms))
            .ok_or_else(|| "recording timeline gap end overflowed".to_string())?;
        let end_source = gap
            .source_position_frames
            .checked_add(gap.dropped_frames)
            .ok_or_else(|| "recording timeline gap source range overflowed".to_string())?;
        let revisions = revision_clocks
            .get(&track)
            .ok_or_else(|| "recording timeline gap has no clock revisions".to_string())?;
        let revision_index = revisions
            .iter()
            .rposition(|(mapping, _)| mapping.source_position_frames <= gap.source_position_frames)
            .ok_or_else(|| {
                "recording timeline gap precedes its first clock revision".to_string()
            })?;
        let (mapping, sample_rate_hz) = &revisions[revision_index];
        let (expected_start_ms, expected_duration_ms) =
            SessionClock::new(mapping.clone(), *sample_rate_hz)
                .and_then(|clock| clock.interval_ms(gap.source_position_frames, gap.dropped_frames))
                .map_err(|_| "recording timeline gap clock conversion failed".to_string())?;
        if gap.start_ms != expected_start_ms || gap.duration_ms != expected_duration_ms {
            return Err("recording timeline gap does not match its clock revision".into());
        }
        if revisions.get(revision_index + 1).is_some_and(|(next, _)| {
            end_source > next.source_position_frames || end_ms > next.session_time_ms
        }) {
            return Err("recording timeline gap crosses a clock revision".into());
        }
        if let Some((generation, previous_end_ms, previous_end_source, previous_cause)) =
            gaps.get(&track)
        {
            if gap.generation <= *generation
                || gap.start_ms < *previous_end_ms
                || gap.source_position_frames < *previous_end_source
            {
                return Err("recording timeline gaps are not monotonic".into());
            }
            if gap.start_ms == *previous_end_ms
                && gap.source_position_frames == *previous_end_source
                && gap.cause == *previous_cause
            {
                return Err("recording timeline contains an uncoalesced contiguous gap".into());
            }
        }
        gaps.insert(track, (gap.generation, end_ms, end_source, gap.cause));
    }
    Ok(())
}

fn validate_sequence_metadata(
    tracks: &[JournalTrack],
    sequence_coverage: &[SequenceCoverage],
    sequence_gaps: &[SequenceGap],
    sequence_gap_overflow: Option<&SequenceGapOverflow>,
    sink_degraded: bool,
) -> Result<(), String> {
    validate_initial_sequence_coverage(sequence_coverage)?;
    let track_ids = tracks
        .iter()
        .map(|track| track.track_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut coverage_by_track = BTreeMap::new();
    for coverage in sequence_coverage {
        if !track_ids.contains(coverage.track_id.as_str())
            || coverage.first_sequence > coverage.last_sequence
            || coverage_by_track
                .insert(coverage.track_id.as_str(), coverage)
                .is_some()
        {
            return Err("recording sequence coverage is invalid".into());
        }
    }
    if coverage_by_track.len() != track_ids.len() {
        return Err("recording track has no sequence coverage".into());
    }

    let mut previous_gap_end = BTreeMap::<&str, u64>::new();
    for gap in sequence_gaps {
        let coverage = coverage_by_track
            .get(gap.track_id.as_str())
            .ok_or_else(|| "recording sequence gap has no coverage".to_string())?;
        let gap_end = gap
            .first_sequence
            .checked_add(gap.dropped_frames)
            .filter(|end| gap.dropped_frames > 0 && *end <= coverage.last_sequence)
            .ok_or_else(|| "recording sequence gap is invalid".to_string())?;
        if gap.first_sequence <= coverage.first_sequence
            || previous_gap_end
                .get(gap.track_id.as_str())
                .is_some_and(|previous_end| gap.first_sequence < *previous_end)
        {
            return Err("recording sequence gaps are not ordered".into());
        }
        previous_gap_end.insert(gap.track_id.as_str(), gap_end);
    }

    if let Some(overflow) = sequence_gap_overflow {
        if overflow.detail_capacity != MAX_SEQUENCE_GAP_DETAILS as u32
            || sequence_gaps.len() != MAX_SEQUENCE_GAP_DETAILS
            || overflow.omitted_gap_count == 0
            || overflow.omitted_dropped_frames == 0
        {
            return Err("recording sequence-gap overflow is invalid".into());
        }
    }
    if (!sequence_gaps.is_empty() || sequence_gap_overflow.is_some()) && !sink_degraded {
        return Err("recording sequence degradation is inconsistent".into());
    }
    if sink_degraded || !sequence_gaps.is_empty() || sequence_gap_overflow.is_some() {
        return Err("degraded recording metadata cannot be complete".into());
    }
    Ok(())
}

fn validate_initial_sequence_coverage(
    sequence_coverage: &[SequenceCoverage],
) -> Result<(), String> {
    if sequence_coverage
        .iter()
        .any(|coverage| coverage.first_sequence != 0)
    {
        return Err("recording track sequence must start at zero".into());
    }
    Ok(())
}

fn validate_capture_metadata(
    metadata: &SessionMetadata,
    session_id: &SessionId,
) -> Result<(), String> {
    if metadata.session_id != *session_id {
        return Err("capture session metadata does not match the recording session".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalTrack {
    track_id: String,
    sample_rate_hz: u32,
    channels: u16,
    first_start_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SequenceCoverage {
    track_id: String,
    first_sequence: u64,
    last_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SequenceGap {
    track_id: String,
    first_sequence: u64,
    dropped_frames: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SequenceGapOverflow {
    detail_capacity: u32,
    omitted_gap_count: u64,
    omitted_dropped_frames: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureJournal {
    schema_version: u16,
    session_id: SessionId,
    tracks: BTreeMap<String, JournalTrack>,
    track_configurations: Vec<TrackConfigurationRevision>,
    clock_mappings: Vec<ClockMappingRevision>,
    timeline_gaps: Vec<AudioGap>,
    sequence_coverage: Vec<SequenceCoverage>,
    sequence_gaps: Vec<SequenceGap>,
    #[serde(default)]
    sequence_gap_overflow: Option<SequenceGapOverflow>,
    sink_degraded: bool,
}

impl CaptureJournal {
    fn new(session_id: SessionId) -> Self {
        Self {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id,
            tracks: BTreeMap::new(),
            track_configurations: Vec::new(),
            clock_mappings: Vec::new(),
            timeline_gaps: Vec::new(),
            sequence_coverage: Vec::new(),
            sequence_gaps: Vec::new(),
            sequence_gap_overflow: None,
            sink_degraded: false,
        }
    }

    fn observe_frame(
        &mut self,
        track_id: &str,
        sample_rate_hz: u32,
        channels: u16,
        sequence: u64,
        start_ms: u64,
    ) {
        self.tracks
            .entry(track_id.to_string())
            .or_insert(JournalTrack {
                track_id: track_id.to_string(),
                sample_rate_hz,
                channels,
                first_start_ms: start_ms,
            });
        let coverage_index = self
            .sequence_coverage
            .iter()
            .position(|coverage| coverage.track_id == track_id);
        match coverage_index {
            Some(index)
                if sequence
                    == self.sequence_coverage[index]
                        .last_sequence
                        .saturating_add(1) =>
            {
                self.sequence_coverage[index].last_sequence = sequence;
            }
            Some(index) if sequence > self.sequence_coverage[index].last_sequence => {
                let first_missing = self.sequence_coverage[index]
                    .last_sequence
                    .saturating_add(1);
                self.record_gap(track_id, first_missing, sequence);
                self.sequence_coverage[index].last_sequence = sequence;
            }
            Some(_) => self.sink_degraded = true,
            None => self.sequence_coverage.push(SequenceCoverage {
                track_id: track_id.to_string(),
                first_sequence: sequence,
                last_sequence: sequence,
            }),
        }
    }

    fn observe_revision_transition(
        &mut self,
        transition: RecordingRevisionTransition,
    ) -> Result<(), String> {
        let configuration = &transition.configuration;
        let mapping = &transition.clock_mapping;
        if self.track_configurations.len() >= MAX_TIMELINE_CONTROL_EVENTS
            || self.clock_mappings.len() >= MAX_TIMELINE_CONTROL_EVENTS
        {
            return Err("recording revision-transition metadata limit reached".into());
        }
        if configuration.track_id != mapping.track_id
            || configuration.revision != mapping.revision
            || configuration.effective_at_ms != mapping.session_time_ms
        {
            return Err("recording revision transition is inconsistent".into());
        }
        let previous = self
            .track_configurations
            .iter()
            .rev()
            .find(|previous| previous.track_id == configuration.track_id);
        match previous {
            Some(previous)
                if previous.revision.checked_add(1) == Some(configuration.revision)
                    && configuration.effective_at_ms >= previous.effective_at_ms
                    && configuration.sample_rate_hz > 0 => {}
            None if configuration.revision == 1 && configuration.sample_rate_hz > 0 => {}
            _ => return Err("recording track configuration is not monotonic".into()),
        }
        let previous = self
            .clock_mappings
            .iter()
            .rev()
            .find(|previous| previous.track_id == mapping.track_id);
        match previous {
            Some(previous)
                if previous.revision.checked_add(1) == Some(mapping.revision)
                    && mapping.source_position_frames >= previous.source_position_frames
                    && mapping.session_time_ms >= previous.session_time_ms => {}
            None if mapping.revision == 1 => {}
            _ => return Err("recording clock mapping is not monotonic".into()),
        }
        self.track_configurations.push(transition.configuration);
        self.clock_mappings.push(transition.clock_mapping);
        Ok(())
    }

    fn observe_gap(&mut self, gap: AudioGap) -> Result<(), String> {
        if gap.session_id != self.session_id
            || gap.duration_ms == 0
            || gap.dropped_frames == 0
            || !self
                .track_configurations
                .iter()
                .any(|configuration| configuration.track_id == gap.track_id)
            || gap.end_ms().is_none()
            || gap
                .source_position_frames
                .checked_add(gap.dropped_frames)
                .is_none()
        {
            return Err("recording timeline gap is invalid".into());
        }
        if let Some(index) = self.timeline_gaps.iter().position(|previous| {
            previous.session_id == gap.session_id
                && previous.track_id == gap.track_id
                && previous.cause == gap.cause
                && previous.start_ms == gap.start_ms
                && previous.source_position_frames == gap.source_position_frames
        }) {
            let previous = &self.timeline_gaps[index];
            if gap.generation <= previous.generation
                || gap.duration_ms < previous.duration_ms
                || gap.dropped_frames < previous.dropped_frames
                || self.timeline_gaps[index + 1..]
                    .iter()
                    .any(|later| later.track_id == gap.track_id)
            {
                return Err("recording timeline gap replacement regressed".into());
            }
            self.timeline_gaps[index] = gap;
            return Ok(());
        }
        if self.timeline_gaps.len() >= MAX_TIMELINE_CONTROL_EVENTS {
            return Err("recording timeline-gap metadata limit reached".into());
        }
        if let Some(previous) = self
            .timeline_gaps
            .iter()
            .rev()
            .find(|previous| previous.track_id == gap.track_id)
        {
            let previous_end_ms = previous
                .end_ms()
                .ok_or_else(|| "recording timeline gap end overflowed".to_string())?;
            let previous_end_source = previous
                .source_position_frames
                .checked_add(previous.dropped_frames)
                .ok_or_else(|| "recording timeline gap source range overflowed".to_string())?;
            if gap.generation <= previous.generation
                || gap.start_ms < previous_end_ms
                || gap.source_position_frames < previous_end_source
            {
                return Err("recording timeline gap is not monotonic".into());
            }
            if gap.cause == previous.cause
                && gap.start_ms == previous_end_ms
                && gap.source_position_frames == previous_end_source
            {
                return Err("recording timeline gap was not coalesced".into());
            }
        }
        self.timeline_gaps.push(gap);
        Ok(())
    }

    fn record_gap(&mut self, track_id: &str, first_sequence: u64, next_sequence: u64) {
        let dropped_frames = next_sequence.saturating_sub(first_sequence);
        if dropped_frames == 0 {
            return;
        }
        self.sink_degraded = true;
        if let Some(previous) = self.sequence_gaps.last_mut() {
            if previous.track_id == track_id
                && previous.first_sequence.checked_add(previous.dropped_frames)
                    == Some(first_sequence)
            {
                previous.dropped_frames = previous.dropped_frames.saturating_add(dropped_frames);
                return;
            }
        }
        if self.sequence_gaps.len() < MAX_SEQUENCE_GAP_DETAILS {
            self.sequence_gaps.push(SequenceGap {
                track_id: track_id.to_string(),
                first_sequence,
                dropped_frames,
            });
            return;
        }
        let overflow = self
            .sequence_gap_overflow
            .get_or_insert(SequenceGapOverflow {
                detail_capacity: MAX_SEQUENCE_GAP_DETAILS as u32,
                omitted_gap_count: 0,
                omitted_dropped_frames: 0,
            });
        overflow.omitted_gap_count = overflow.omitted_gap_count.saturating_add(1);
        overflow.omitted_dropped_frames = overflow
            .omitted_dropped_frames
            .saturating_add(dropped_frames);
    }

    #[cfg(test)]
    fn serialized_len(&self) -> usize {
        serde_json::to_vec(self).map_or(usize::MAX, |value| value.len())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalDelta {
    schema_version: u16,
    session_id: SessionId,
    tracks: Vec<JournalTrack>,
    revision_transitions: Vec<RecordingRevisionTransition>,
    timeline_gap_start_index: usize,
    timeline_gaps: Vec<AudioGap>,
    sequence_coverage: Vec<SequenceCoverage>,
    gap_start_index: usize,
    sequence_gaps: Vec<SequenceGap>,
    sequence_gap_overflow: Option<SequenceGapOverflow>,
    sink_degraded: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum JournalRecord {
    Header {
        journal: CaptureJournal,
    },
    Delta {
        delta: JournalDelta,
    },
    Overflow {
        session_id: SessionId,
        reason: String,
    },
}

#[derive(Debug, Clone)]
struct DurableJournalState {
    tracks: BTreeSet<String>,
    revision_transitions: usize,
    timeline_gaps: Vec<AudioGap>,
    sequence_coverage: BTreeMap<String, SequenceCoverage>,
    sequence_gaps: usize,
}

impl DurableJournalState {
    fn from_journal(journal: &CaptureJournal) -> Self {
        debug_assert_eq!(
            journal.track_configurations.len(),
            journal.clock_mappings.len()
        );
        Self {
            tracks: journal.tracks.keys().cloned().collect(),
            revision_transitions: journal.track_configurations.len(),
            timeline_gaps: journal.timeline_gaps.clone(),
            sequence_coverage: journal
                .sequence_coverage
                .iter()
                .map(|coverage| (coverage.track_id.clone(), coverage.clone()))
                .collect(),
            sequence_gaps: journal.sequence_gaps.len(),
        }
    }

    fn delta(&self, journal: &CaptureJournal) -> JournalDelta {
        let gap_start_index = self.sequence_gaps.saturating_sub(1);
        let timeline_gap_start_index =
            first_changed_index(&self.timeline_gaps, &journal.timeline_gaps);
        JournalDelta {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id: journal.session_id.clone(),
            tracks: journal
                .tracks
                .iter()
                .filter(|(track_id, _)| !self.tracks.contains(*track_id))
                .map(|(_, track)| track.clone())
                .collect(),
            revision_transitions: journal.track_configurations[self.revision_transitions..]
                .iter()
                .cloned()
                .zip(
                    journal.clock_mappings[self.revision_transitions..]
                        .iter()
                        .cloned(),
                )
                .map(
                    |(configuration, clock_mapping)| RecordingRevisionTransition {
                        configuration,
                        clock_mapping,
                    },
                )
                .collect(),
            timeline_gap_start_index,
            timeline_gaps: journal.timeline_gaps[timeline_gap_start_index..].to_vec(),
            sequence_coverage: journal
                .sequence_coverage
                .iter()
                .filter(|coverage| {
                    self.sequence_coverage.get(&coverage.track_id) != Some(*coverage)
                })
                .cloned()
                .collect(),
            gap_start_index,
            sequence_gaps: journal.sequence_gaps[gap_start_index..].to_vec(),
            sequence_gap_overflow: journal.sequence_gap_overflow.clone(),
            sink_degraded: journal.sink_degraded,
        }
    }
}

fn first_changed_index<T: PartialEq>(durable: &[T], current: &[T]) -> usize {
    durable
        .iter()
        .zip(current)
        .position(|(durable, current)| durable != current)
        .unwrap_or_else(|| durable.len().min(current.len()))
}

pub struct StreamingRecording {
    paths: RecordingPaths,
    session_metadata: SessionMetadata,
    audio: Option<File>,
    #[cfg(test)]
    _reservation_handle_drop_signal: Option<ReservationHandleDropSignal>,
    journal_file: Option<File>,
    journal: CaptureJournal,
    journal_durable: DurableJournalState,
    journal_bytes: u64,
    journal_growth_stopped: bool,
    journal_terminal_written: bool,
    data_bytes: u64,
    samples_since_sync: u64,
    sync_interval_samples: u64,
    data_limit: u64,
    failure: Option<String>,
    sidecar_receipt: Option<PublicationReceipt>,
    finalized: Option<RecordingFinalizeResult>,
    #[cfg(test)]
    fault: Option<CommitFaultPoint>,
    #[cfg(test)]
    after_sidecar_publish: Option<SidecarPublishHook>,
    #[cfg(test)]
    publication_hook: Option<PublicationHook>,
    #[cfg(test)]
    append_write_attempts: Option<Arc<std::sync::atomic::AtomicUsize>>,
    #[cfg(test)]
    journal_write_attempts: Option<Arc<std::sync::atomic::AtomicUsize>>,
}

fn default_session_metadata(session_id: SessionId) -> Result<SessionMetadata, String> {
    SessionMetadata::new(
        session_id,
        SessionMode::Dictation,
        SessionOrigin::LiveCapture,
        TriggerMode::PushToTalk,
        std::time::SystemTime::now(),
        None,
        None,
        None,
        Vec::new(),
        None,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationArtifact {
    Audio,
    CompleteSidecar,
    PartialSidecar,
    Commit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationBarrier {
    BeforeHardLink,
    AfterHardLink,
}

#[derive(Debug, Clone)]
struct PublicationReceipt {
    file_name: String,
    sha256: String,
    status: CaptureStatus,
    path: PathBuf,
    identity: FileIdentity,
}

impl PublicationReceipt {
    fn lineage(&self) -> PartialCaptureLineage {
        match self.status {
            CaptureStatus::Complete | CaptureStatus::Partial => {}
        }
        PartialCaptureLineage {
            capture_sidecar_file: self.file_name.clone(),
            capture_sidecar_sha256: self.sha256.clone(),
        }
    }

    fn revalidate(&self) -> Result<(), String> {
        let mut current = open_regular_path(&self.path)?;
        if self.identity != file_identity(&current)? {
            return Err("capture sidecar path no longer names the verified destination".into());
        }
        if sha256_open_file(&mut current)? != self.sha256 {
            return Err(
                "capture sidecar path no longer matches the verified destination hash".into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg(unix)]
pub(crate) struct FileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg(windows)]
pub(crate) struct FileIdentity {
    volume_serial: u32,
    file_index: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg(not(any(unix, windows)))]
pub(crate) struct FileIdentity;

#[derive(Debug)]
pub(crate) struct RegularArtifactIdentity {
    path: PathBuf,
    identity: FileIdentity,
    require_single_link: bool,
}

impl RegularArtifactIdentity {
    pub(crate) fn matches_artifact_name(&self, name: &str) -> bool {
        self.path.file_name().and_then(|value| value.to_str()) == Some(name)
    }

    fn open_current(&self) -> Result<File, String> {
        self.open_current_at(&self.path)
    }

    fn open_current_at(&self, path: &Path) -> Result<File, String> {
        let current = open_regular_path(path)?;
        if file_identity(&current)? != self.identity {
            return Err("recording artifact path no longer names the admitted file".into());
        }
        self.ensure_link_ownership(&current)?;
        Ok(current)
    }

    fn ensure_open_file(&self, file: &File) -> Result<(), String> {
        if file_identity(file)? != self.identity {
            return Err("recording artifact path no longer names the admitted file".into());
        }
        self.ensure_link_ownership(file)?;
        Ok(())
    }

    fn ensure_link_ownership(&self, file: &File) -> Result<(), String> {
        if self.require_single_link && file_link_count(file)? != 1 {
            return Err("private recording artifact has multiple filesystem links".into());
        }
        Ok(())
    }

    pub(crate) fn file_identity(&self) -> FileIdentity {
        self.identity
    }

    pub(crate) fn read_and_hash(&self) -> Result<(String, String), String> {
        let mut current = self.open_current()?;
        let text = read_open_file(&mut current)?;
        let hash = Sha256::digest(text.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok((text, hash))
    }

    pub(crate) fn sha256(&self) -> Result<String, String> {
        let mut current = self.open_current()?;
        sha256_open_file(&mut current)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct QuarantinedArtifact {
    path: PathBuf,
    sha256: String,
    identity: FileIdentity,
}

#[derive(Debug, Clone)]
struct RecordingPaths {
    directory: PathBuf,
    session_id: SessionId,
    wav_part: PathBuf,
    journal_part: PathBuf,
    wav: PathBuf,
    sidecar: PathBuf,
    sidecar_part: PathBuf,
    partial_sidecar: PathBuf,
    partial_sidecar_part: PathBuf,
    commit: PathBuf,
    commit_part: PathBuf,
}

impl RecordingPaths {
    fn new(directory: &Path, session_id: SessionId) -> Self {
        let prefix = format!("live-{session_id}");
        Self {
            directory: directory.to_path_buf(),
            session_id,
            wav_part: directory.join(format!("{prefix}.wav.part")),
            journal_part: directory.join(format!("{prefix}.capture.journal.part")),
            wav: directory.join(format!("{prefix}.wav")),
            sidecar: directory.join(format!("{prefix}.capture.json")),
            sidecar_part: directory.join(format!("{prefix}.capture.json.part")),
            partial_sidecar: directory.join(format!("{prefix}.capture.partial.json")),
            partial_sidecar_part: directory.join(format!("{prefix}.capture.partial.json.part")),
            commit: directory.join(format!("{prefix}.commit.json")),
            commit_part: directory.join(format!("{prefix}.commit.json.part")),
        }
    }

    fn wav_file_name(&self) -> String {
        self.wav.file_name().unwrap().to_string_lossy().into_owned()
    }

    fn sidecar_file_name(&self) -> String {
        self.sidecar
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    fn partial_sidecar_file_name(&self) -> String {
        self.partial_sidecar
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[cfg(test)]
    fn path_for_publication(
        &self,
        artifact: PublicationArtifact,
        barrier: PublicationBarrier,
    ) -> PathBuf {
        match (artifact, barrier) {
            (PublicationArtifact::CompleteSidecar, PublicationBarrier::BeforeHardLink) => {
                self.sidecar_part.clone()
            }
            (PublicationArtifact::CompleteSidecar, PublicationBarrier::AfterHardLink) => {
                self.sidecar.clone()
            }
            (PublicationArtifact::PartialSidecar, PublicationBarrier::BeforeHardLink) => {
                self.partial_sidecar_part.clone()
            }
            (PublicationArtifact::PartialSidecar, PublicationBarrier::AfterHardLink) => {
                self.partial_sidecar.clone()
            }
            (PublicationArtifact::Commit, PublicationBarrier::BeforeHardLink) => {
                self.commit_part.clone()
            }
            (PublicationArtifact::Commit, PublicationBarrier::AfterHardLink) => self.commit.clone(),
            (PublicationArtifact::Audio, PublicationBarrier::BeforeHardLink) => {
                self.wav_part.clone()
            }
            (PublicationArtifact::Audio, PublicationBarrier::AfterHardLink) => self.wav.clone(),
        }
    }
}

/// The durable claim passed directly from allocation into the recording worker.
/// Its audio handle is the only authority permitted to write the reserved WAV.
#[derive(Debug)]
pub(crate) struct RecordingReservation {
    paths: RecordingPaths,
    audio: File,
    identity: FileIdentity,
    session_metadata: SessionMetadata,
    #[cfg(test)]
    handle_drop_signal: Option<ReservationHandleDropSignal>,
}

impl RecordingReservation {
    pub(crate) fn session_id(&self) -> &SessionId {
        &self.paths.session_id
    }

    pub(crate) fn with_session_metadata(
        mut self,
        metadata: SessionMetadata,
    ) -> Result<Self, String> {
        validate_capture_metadata(&metadata, self.session_id())?;
        self.session_metadata = metadata;
        Ok(self)
    }

    #[cfg(test)]
    fn wav_part(&self) -> &Path {
        &self.paths.wav_part
    }

    #[cfg(test)]
    fn watch_handle_drop_for_test(
        &mut self,
        dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        self.handle_drop_signal = Some(ReservationHandleDropSignal(dropped));
    }
}

#[cfg(test)]
#[derive(Debug)]
struct ReservationHandleDropSignal(std::sync::Arc<std::sync::atomic::AtomicBool>);

#[cfg(test)]
impl Drop for ReservationHandleDropSignal {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Allocates a persistent recording identity by atomically reserving its first
/// on-disk artifact. Runtime-local counters are deliberately not part of this ID.
pub(crate) fn allocate_recording_session(directory: &Path) -> Result<RecordingReservation, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("Failed to create live recordings folder: {error}"))?;
    let mut reservation = None;
    session::allocate_recording(|session_id| {
        let claimed = reserve_wav_part(directory, session_id)?;
        reservation = Some(claimed);
        Ok(())
    })?;
    reservation.ok_or_else(|| "recording allocation returned without a reservation".to_string())
}

fn reserve_wav_part(
    directory: &Path,
    session_id: &SessionId,
) -> std::io::Result<RecordingReservation> {
    reserve_wav_part_with_before_claim(directory, session_id, || {})
}

fn reserve_wav_part_with_before_claim<F>(
    directory: &Path,
    session_id: &SessionId,
    before_claim: F,
) -> std::io::Result<RecordingReservation>
where
    F: FnOnce(),
{
    let paths = RecordingPaths::new(directory, session_id.clone());
    let prefix = format!("live-{session_id}.");
    let conflicting_artifact = fs::read_dir(directory)?.any(|entry| {
        let Ok(entry) = entry else {
            return true;
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.starts_with(&prefix)
    });
    if conflicting_artifact {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "recording artifact prefix already exists",
        ));
    }
    before_claim();
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&paths.wav_part)?;
    file.sync_all()?;
    let identity = file_identity(&file).map_err(std::io::Error::other)?;
    let session_metadata =
        default_session_metadata(session_id.clone()).map_err(std::io::Error::other)?;
    Ok(RecordingReservation {
        paths,
        audio: file,
        identity,
        session_metadata,
        #[cfg(test)]
        handle_drop_signal: None,
    })
}

impl StreamingRecording {
    pub fn create(directory: &Path, session_id: SessionId) -> Result<Self, String> {
        Self::create_inner(directory, session_id, None)
    }

    #[cfg(test)]
    pub(crate) fn create_with_session_metadata(
        directory: &Path,
        metadata: SessionMetadata,
    ) -> Result<Self, String> {
        let session_id = metadata.session_id.clone();
        fs::create_dir_all(directory)
            .map_err(|error| format!("Failed to create live recordings folder: {error}"))?;
        let paths = RecordingPaths::new(directory, session_id);
        let audio = create_new(&paths.wav_part, "recording audio")?;
        Self::create_from_open_audio(paths, audio, None, None, metadata)
    }

    pub(crate) fn create_reserved(reservation: RecordingReservation) -> Result<Self, String> {
        let RecordingReservation {
            paths,
            audio,
            identity,
            session_metadata,
            #[cfg(test)]
            handle_drop_signal,
        } = reservation;
        if file_identity(&audio)? != identity {
            return Err(
                "reserved recording audio handle identity changed before worker adoption".into(),
            );
        }
        Self::create_from_open_audio(
            paths,
            audio,
            #[cfg(test)]
            handle_drop_signal,
            None,
            session_metadata,
        )
    }

    #[cfg(test)]
    pub(crate) fn create_with_fault(
        directory: &Path,
        session_id: SessionId,
        fault: CommitFaultPoint,
    ) -> Result<Self, String> {
        Self::create_inner(directory, session_id, Some(fault))
    }

    #[cfg(test)]
    fn create_with_sidecar_hook<F>(
        directory: &Path,
        session_id: SessionId,
        hook: F,
    ) -> Result<Self, String>
    where
        F: FnOnce(&RecordingPaths) + Send + 'static,
    {
        let mut recording = Self::create_inner(directory, session_id, None)?;
        recording.after_sidecar_publish = Some(Box::new(hook));
        Ok(recording)
    }

    #[cfg(test)]
    fn create_with_publication_hook<F>(
        directory: &Path,
        session_id: SessionId,
        fault: Option<CommitFaultPoint>,
        hook: F,
    ) -> Result<Self, String>
    where
        F: FnMut(PublicationArtifact, PublicationBarrier, &RecordingPaths) + Send + 'static,
    {
        let mut recording = Self::create_inner(directory, session_id, fault)?;
        recording.publication_hook = Some(Box::new(hook));
        Ok(recording)
    }

    fn create_inner(
        directory: &Path,
        session_id: SessionId,
        #[cfg(test)] fault: Option<CommitFaultPoint>,
        #[cfg(not(test))] _fault: Option<()>,
    ) -> Result<Self, String> {
        fs::create_dir_all(directory)
            .map_err(|error| format!("Failed to create live recordings folder: {error}"))?;
        let paths = RecordingPaths::new(directory, session_id.clone());
        let audio = create_new(&paths.wav_part, "recording audio")?;
        let session_metadata = default_session_metadata(session_id)?;
        Self::create_from_open_audio(
            paths,
            audio,
            #[cfg(test)]
            None,
            #[cfg(test)]
            fault,
            #[cfg(not(test))]
            _fault,
            session_metadata,
        )
    }

    fn create_from_open_audio(
        paths: RecordingPaths,
        mut audio: File,
        #[cfg(test)] reservation_handle_drop_signal: Option<ReservationHandleDropSignal>,
        #[cfg(test)] fault: Option<CommitFaultPoint>,
        #[cfg(not(test))] _fault: Option<()>,
        session_metadata: SessionMetadata,
    ) -> Result<Self, String> {
        if audio
            .metadata()
            .map_err(|error| format!("Failed to inspect recording audio: {error}"))?
            .len()
            != 0
        {
            return Err("Reserved recording audio was unexpectedly non-empty".into());
        }
        write_wav_header(&mut audio, 0)?;
        audio
            .sync_data()
            .map_err(|error| format!("Failed to initialize live audio: {error}"))?;
        let mut journal_file = create_new(&paths.journal_part, "recording journal")?;
        let journal = CaptureJournal::new(paths.session_id.clone());
        let journal_bytes = write_journal_record(
            &mut journal_file,
            &JournalRecord::Header {
                journal: journal.clone(),
            },
        )?;
        journal_file
            .sync_data()
            .map_err(|error| format!("Failed to initialize recording journal: {error}"))?;
        Ok(Self {
            paths,
            session_metadata,
            audio: Some(audio),
            #[cfg(test)]
            _reservation_handle_drop_signal: reservation_handle_drop_signal,
            journal_file: Some(journal_file),
            journal_durable: DurableJournalState::from_journal(&journal),
            journal,
            journal_bytes,
            journal_growth_stopped: false,
            journal_terminal_written: false,
            data_bytes: 0,
            samples_since_sync: 0,
            sync_interval_samples: DEFAULT_SYNC_INTERVAL_SAMPLES,
            data_limit: u64::from(u32::MAX),
            failure: None,
            sidecar_receipt: None,
            finalized: None,
            #[cfg(test)]
            fault,
            #[cfg(test)]
            after_sidecar_publish: None,
            #[cfg(test)]
            publication_hook: None,
            #[cfg(test)]
            append_write_attempts: None,
            #[cfg(test)]
            journal_write_attempts: None,
        })
    }

    fn append_prepared(&mut self, frame: &PreparedFrame) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if frame.metadata.session_id != self.journal.session_id {
            return self.fail("recording prepared frame session does not match".into());
        }
        if frame.metadata.sequence != 0
            && !self
                .journal
                .sequence_coverage
                .iter()
                .any(|coverage| coverage.track_id == frame.metadata.track_id.as_str())
        {
            return self.fail("recording track sequence must start at zero".into());
        }
        self.observe_frame_metadata(
            frame.metadata.track_id.as_str(),
            frame.metadata.sample_rate_hz,
            frame.metadata.channels,
            frame.metadata.sequence,
            frame.metadata.start_ms,
            frame.metadata.duration_ms,
        );
        self.write_pcm16(&f32_to_i16_le_bytes(&frame.samples))
    }

    fn append_input(&mut self, input: RecordingInput) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        match input {
            RecordingInput::PreparedFrame(frame) => self.append_prepared(&frame),
            RecordingInput::RevisionTransition(transition) => {
                self.journal.observe_revision_transition(transition)?;
                self.persist_journal()
            }
            RecordingInput::Gap(gap) => {
                self.journal.observe_gap(gap)?;
                self.persist_journal()
            }
        }
    }

    fn write_pcm16(&mut self, pcm: &[u8]) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if !pcm.len().is_multiple_of(2) {
            return self.fail("PCM16 append has an odd byte length".into());
        }
        let added =
            u64::try_from(pcm.len()).map_err(|_| "PCM16 append is too large".to_string())?;
        if self
            .data_bytes
            .checked_add(added)
            .is_none_or(|total| total > self.data_limit)
        {
            return self.fail("Live recording exceeds the WAV 32-bit data-length limit".into());
        }
        #[cfg(test)]
        if let Some(attempts) = &self.append_write_attempts {
            attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        if let Err(error) = self.hit_fault(CommitFaultPoint::Append) {
            return self.fail(error);
        }
        let write_result = self
            .audio
            .as_mut()
            .ok_or_else(|| "Live recording is already finalized".to_string())
            .and_then(|audio| {
                audio
                    .write_all(pcm)
                    .map_err(|error| format!("Failed to append live audio: {error}"))
            });
        if let Err(error) = write_result {
            return self.fail(error);
        }
        self.data_bytes += added;
        self.samples_since_sync += added / PCM16_BYTES_PER_SAMPLE;
        if self.samples_since_sync >= self.sync_interval_samples {
            if let Err(error) = self.hit_fault(CommitFaultPoint::PeriodicFlush) {
                return self.fail(error);
            }
            let sync_result = self
                .audio
                .as_mut()
                .expect("recording audio was checked before append")
                .sync_data();
            if let Err(error) = sync_result {
                return self.fail(format!("Failed to flush live audio: {error}"));
            }
            if let Err(error) = self.persist_journal() {
                return self.fail(error);
            }
            self.samples_since_sync = 0;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn append_pcm16(&mut self, pcm: &[u8]) -> Result<(), String> {
        const TEST_TRACK: &str = "test-raw-pcm";

        if !pcm.is_empty() && pcm.len().is_multiple_of(2) {
            let metadata_is_empty = self.journal.tracks.is_empty()
                && self.journal.track_configurations.is_empty()
                && self.journal.clock_mappings.is_empty()
                && self.journal.sequence_coverage.is_empty();
            if metadata_is_empty {
                let track = crate::audio::session::TrackId::new(TEST_TRACK).unwrap();
                self.journal.observe_revision_transition(
                    RecordingRevisionTransition::new(
                        TrackConfigurationRevision::new(track.clone(), 1, 0, 16_000).unwrap(),
                        ClockMappingRevision::new(track, 1, 0, 0).unwrap(),
                    )
                    .unwrap(),
                )?;
            }
            if self
                .journal
                .track_configurations
                .iter()
                .any(|configuration| configuration.track_id.as_str() == TEST_TRACK)
            {
                let sequence = self
                    .journal
                    .sequence_coverage
                    .iter()
                    .find(|coverage| coverage.track_id == TEST_TRACK)
                    .map_or(0, |coverage| coverage.last_sequence.saturating_add(1));
                self.journal
                    .observe_frame(TEST_TRACK, 16_000, 1, sequence, sequence);
            }
        }
        self.write_pcm16(pcm)
    }

    fn observe_frame_metadata(
        &mut self,
        track_id: &str,
        sample_rate_hz: u32,
        channels: u16,
        sequence: u64,
        start_ms: u64,
        _duration_ms: u32,
    ) {
        self.journal
            .observe_frame(track_id, sample_rate_hz, channels, sequence, start_ms);
    }

    pub fn finalize(&mut self) -> Result<RecordingFinalizeResult, String> {
        if let Some(result) = &self.finalized {
            return Ok(result.clone());
        }
        if self.journal.sink_degraded && self.failure.is_none() {
            self.abort("recording sequence metadata is degraded".into());
        }
        if self.failure.is_some() {
            return Ok(self.partial_result());
        }

        let manifest = match self.finalize_inner() {
            Ok(manifest) => manifest,
            Err(error) => {
                self.failure = Some(error);
                return Ok(self.partial_result());
            }
        };
        let result = RecordingFinalizeResult {
            session_id: self.paths.session_id.clone(),
            status: CaptureStatus::Complete,
            committed: Some(CommittedCapture {
                manifest,
                directory: self.paths.directory.clone(),
            }),
            partial_lineage: None,
            error: None,
            sidecar_receipt: self.sidecar_receipt.clone(),
        };
        self.finalized = Some(result.clone());
        Ok(result)
    }

    fn finalize_inner(&mut self) -> Result<CaptureCommitManifest, String> {
        let mut audio = self
            .audio
            .take()
            .ok_or_else(|| "Live recording audio is unavailable".to_string())?;
        self.hit_fault(CommitFaultPoint::WavHeaderPatch)?;
        write_wav_header(&mut audio, self.data_bytes)?;
        self.hit_fault(CommitFaultPoint::AudioSync)?;
        audio
            .sync_all()
            .map_err(|error| format!("Failed to sync finalized live audio: {error}"))?;

        self.persist_journal()?;

        self.hit_fault(CommitFaultPoint::FinalArtifactRename)?;
        let wav_part = self.paths.wav_part.clone();
        let wav = self.paths.wav.clone();
        let mut published_audio = self.publish_owned(
            &wav_part,
            &wav,
            &audio,
            "finalize live audio",
            PublicationArtifact::Audio,
            CommitFaultPoint::AudioStagingCleanup,
        )?;
        let audio_bytes = published_audio
            .metadata()
            .map_err(|error| format!("Failed to inspect finalized live audio: {error}"))?
            .len();
        let audio_sha256 = sha256_open_file(&mut published_audio)?;
        drop(published_audio);
        drop(audio);

        let sidecar = CaptureSidecar {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id: self.paths.session_id.clone(),
            audio_file: self.paths.wav_file_name(),
            audio_sha256,
            audio_bytes,
            tracks: self.journal.tracks.values().cloned().collect(),
            track_configurations: self.journal.track_configurations.clone(),
            clock_mappings: self.journal.clock_mappings.clone(),
            timeline_gaps: self.journal.timeline_gaps.clone(),
            sequence_coverage: self.journal.sequence_coverage.clone(),
            sequence_gaps: self.journal.sequence_gaps.clone(),
            sequence_gap_overflow: self.journal.sequence_gap_overflow.clone(),
            sink_degraded: self.journal.sink_degraded,
            directory_sync_supported: sync_parent_directory(&self.paths.directory),
            session_metadata: Some(self.session_metadata.clone()),
        };
        validate_audio_metadata_presence(sidecar.audio_bytes, &sidecar.tracks)?;
        validate_timeline_control_metadata(
            &sidecar.session_id,
            &sidecar.tracks,
            &sidecar.track_configurations,
            &sidecar.clock_mappings,
            &sidecar.timeline_gaps,
        )?;
        validate_sequence_metadata(
            &sidecar.tracks,
            &sidecar.sequence_coverage,
            &sidecar.sequence_gaps,
            sidecar.sequence_gap_overflow.as_ref(),
            sidecar.sink_degraded,
        )?;
        let sidecar_file =
            write_json_file_open(&self.paths.sidecar_part, &sidecar, "capture sidecar")?;
        self.hit_fault(CommitFaultPoint::SidecarSync)?;
        sidecar_file
            .sync_all()
            .map_err(|error| format!("Failed to sync capture sidecar: {error}"))?;
        let sidecar_part = self.paths.sidecar_part.clone();
        let sidecar_path = self.paths.sidecar.clone();
        let published_sidecar = self.publish_owned(
            &sidecar_part,
            &sidecar_path,
            &sidecar_file,
            "publish capture sidecar",
            PublicationArtifact::CompleteSidecar,
            CommitFaultPoint::SidecarStagingCleanup,
        )?;
        let sidecar_receipt = receipt_from_published_sidecar(
            published_sidecar,
            self.paths.sidecar_file_name(),
            self.paths.sidecar.clone(),
            &sidecar,
        )?;
        drop(sidecar_file);
        self.sidecar_receipt = Some(sidecar_receipt.clone());
        #[cfg(test)]
        if let Some(hook) = self.after_sidecar_publish.take() {
            hook(&self.paths);
        }
        self.revalidate_sidecar_receipt()?;

        let manifest = CaptureCommitManifest {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id: self.paths.session_id.clone(),
            status: CaptureStatus::Complete,
            audio_file: self.paths.wav_file_name(),
            audio_sha256: sidecar.audio_sha256,
            audio_bytes,
            capture_sidecar_file: self.paths.sidecar_file_name(),
            capture_sidecar_sha256: sidecar_receipt.sha256,
            committed_at_utc: now_utc()?,
            session_metadata: Some(self.session_metadata.clone()),
        };
        manifest.validate()?;
        let commit_file =
            write_json_file_open(&self.paths.commit_part, &manifest, "capture commit")?;
        self.hit_fault(CommitFaultPoint::CommitSync)?;
        commit_file
            .sync_all()
            .map_err(|error| format!("Failed to sync capture commit: {error}"))?;
        self.hit_fault(CommitFaultPoint::CommitRename)?;
        self.revalidate_sidecar_receipt()?;
        let commit_part = self.paths.commit_part.clone();
        let commit = self.paths.commit.clone();
        let mut published_commit = self.publish_owned(
            &commit_part,
            &commit,
            &commit_file,
            "publish capture commit",
            PublicationArtifact::Commit,
            CommitFaultPoint::CommitStagingCleanup,
        )?;
        let manifest = manifest_from_published_commit(&mut published_commit, &manifest)?;
        self.revalidate_sidecar_receipt()?;
        self.remove_owned_journal_after_commit();
        let _ = sync_parent_directory(&self.paths.directory);
        Ok(manifest)
    }

    // The journal is recovery state. Keep its original handle until publication so a
    // pathname replacement cannot cause us to remove somebody else's file.
    fn remove_owned_journal_after_commit(&mut self) {
        let Some(journal) = self.journal_file.take() else {
            return;
        };
        let name = self
            .paths
            .journal_part
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let result = if name.is_empty() {
            Err("recording journal has no valid file name".to_string())
        } else {
            remove_open_regular_artifact(&self.paths.directory, name, &journal, || {})
        };
        drop(journal);
        if let Err(error) = result {
            crate::stt::log_yap(&format!(
                "Published capture commit, but journal cleanup is pending: {error}"
            ));
        }
    }

    fn partial_result(&mut self) -> RecordingFinalizeResult {
        let partial_lineage = self.publish_partial_lineage();
        let error = match (self.failure.clone(), partial_lineage.as_ref()) {
            (Some(error), Ok(_)) => Some(error),
            (Some(error), Err(lineage_error)) => Some(format!(
                "{error}; failed to publish partial capture lineage: {lineage_error}"
            )),
            (None, Err(lineage_error)) => Some(format!(
                "Failed to publish partial capture lineage: {lineage_error}"
            )),
            (None, Ok(_)) => None,
        };
        let result = RecordingFinalizeResult {
            session_id: self.paths.session_id.clone(),
            status: CaptureStatus::Partial,
            committed: None,
            partial_lineage: partial_lineage.ok(),
            error,
            sidecar_receipt: self.sidecar_receipt.clone(),
        };
        self.finalized = Some(result.clone());
        result
    }

    fn publish_partial_lineage(&mut self) -> Result<PartialCaptureLineage, String> {
        if let Some(receipt) = &self.sidecar_receipt {
            if receipt.revalidate().is_ok() {
                return Ok(receipt.lineage());
            }
            self.sidecar_receipt = None;
        }

        let sidecar = PartialCaptureSidecar {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id: self.paths.session_id.clone(),
            status: CaptureStatus::Partial,
        };
        let partial_sidecar_file = write_json_file_open(
            &self.paths.partial_sidecar_part,
            &sidecar,
            "partial capture sidecar",
        )?;
        partial_sidecar_file
            .sync_all()
            .map_err(|error| format!("Failed to sync partial capture sidecar: {error}"))?;
        let partial_sidecar_part = self.paths.partial_sidecar_part.clone();
        let partial_sidecar = self.paths.partial_sidecar.clone();
        let published_sidecar = self.publish_owned(
            &partial_sidecar_part,
            &partial_sidecar,
            &partial_sidecar_file,
            "publish partial capture sidecar",
            PublicationArtifact::PartialSidecar,
            CommitFaultPoint::SidecarStagingCleanup,
        )?;
        let receipt = receipt_from_published_partial_sidecar(
            published_sidecar,
            self.paths.partial_sidecar_file_name(),
            self.paths.partial_sidecar.clone(),
            &sidecar,
        )?;
        let _ = sync_parent_directory(&self.paths.directory);
        self.sidecar_receipt = Some(receipt.clone());
        Ok(receipt.lineage())
    }

    fn abort(&mut self, reason: String) {
        self.failure.get_or_insert(reason);
    }

    fn fail<T>(&mut self, error: String) -> Result<T, String> {
        Err(self.failure.get_or_insert(error).clone())
    }

    fn revalidate_sidecar_receipt(&self) -> Result<(), String> {
        self.sidecar_receipt
            .as_ref()
            .ok_or_else(|| "capture sidecar receipt is unavailable".to_string())?
            .revalidate()
    }

    fn hit_fault(&self, point: CommitFaultPoint) -> Result<(), String> {
        #[cfg(test)]
        if self.fault == Some(point) {
            return Err(format!("injected recording fault at {point:?}"));
        }
        let _ = point;
        Ok(())
    }

    fn persist_journal(&mut self) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if self.journal_growth_stopped {
            return self.fail("recording journal durability is unavailable".into());
        }
        let record = JournalRecord::Delta {
            delta: self.journal_durable.delta(&self.journal),
        };
        let bytes = match serialize_journal_record(&record) {
            Ok(bytes) => bytes,
            Err(error) => return self.fail(error),
        };
        if bytes.len() as u64 > MAX_JOURNAL_RECORD_BYTES
            || self
                .journal_bytes
                .saturating_add(bytes.len() as u64)
                .saturating_add(MAX_JOURNAL_TERMINAL_BYTES)
                > MAX_JOURNAL_BYTES
        {
            return self.stop_journal_growth("journal size limit reached");
        }
        #[cfg(test)]
        if let Some(attempts) = &self.journal_write_attempts {
            attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        if let Err(error) = self.hit_fault(CommitFaultPoint::JournalAppend) {
            return self.fail(error);
        }
        let write_result = self
            .journal_file
            .as_mut()
            .ok_or_else(|| "recording journal handle is unavailable".to_string())
            .and_then(|file| {
                file.write_all(&bytes)
                    .map_err(|error| format!("Failed to append recording journal: {error}"))
            });
        if let Err(error) = write_result {
            return self.fail(error);
        }
        if let Err(error) = self.hit_fault(CommitFaultPoint::JournalSync) {
            return self.fail(error);
        }
        let sync_result = self
            .journal_file
            .as_mut()
            .expect("recording journal was checked before sync")
            .sync_data()
            .map_err(|error| format!("Failed to sync recording journal: {error}"));
        if let Err(error) = sync_result {
            return self.fail(error);
        }
        self.journal_bytes = self.journal_bytes.saturating_add(bytes.len() as u64);
        self.journal_durable = DurableJournalState::from_journal(&self.journal);
        Ok(())
    }

    fn stop_journal_growth(&mut self, reason: &str) -> Result<(), String> {
        self.journal.sink_degraded = true;
        if !self.journal_terminal_written {
            let bytes = match serialize_journal_record(&JournalRecord::Overflow {
                session_id: self.paths.session_id.clone(),
                reason: reason.to_string(),
            }) {
                Ok(bytes) => bytes,
                Err(error) => return self.fail(error),
            };
            if self.journal_bytes.saturating_add(bytes.len() as u64) <= MAX_JOURNAL_BYTES {
                #[cfg(test)]
                if let Some(attempts) = &self.journal_write_attempts {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                if let Err(error) = self.hit_fault(CommitFaultPoint::JournalAppend) {
                    return self.fail(error);
                }
                let write_result = self
                    .journal_file
                    .as_mut()
                    .ok_or_else(|| "recording journal handle is unavailable".to_string())
                    .and_then(|file| {
                        file.write_all(&bytes).map_err(|error| {
                            format!("Failed to append recording journal overflow: {error}")
                        })
                    });
                if let Err(error) = write_result {
                    return self.fail(error);
                }
                if let Err(error) = self.hit_fault(CommitFaultPoint::JournalSync) {
                    return self.fail(error);
                }
                let sync_result = self
                    .journal_file
                    .as_mut()
                    .expect("recording journal was checked before overflow sync")
                    .sync_data()
                    .map_err(|error| format!("Failed to sync recording journal overflow: {error}"));
                if let Err(error) = sync_result {
                    return self.fail(error);
                }
                self.journal_bytes = self.journal_bytes.saturating_add(bytes.len() as u64);
                self.journal_terminal_written = true;
            }
        }
        self.journal_growth_stopped = true;
        self.fail(format!("recording journal durability stopped: {reason}"))
    }

    fn publish_owned(
        &mut self,
        source: &Path,
        destination: &Path,
        owned_staging: &File,
        label: &str,
        artifact: PublicationArtifact,
        cleanup_fault: CommitFaultPoint,
    ) -> Result<File, String> {
        self.publication_barrier(artifact, PublicationBarrier::BeforeHardLink);
        let opened_staging = open_regular_path(source)?;
        if !same_file_identity(owned_staging, &opened_staging)? {
            return Err(format!(
                "Refused to {label}: staging path no longer names the owned file"
            ));
        }
        drop(opened_staging);

        fs::hard_link(source, destination)
            .map_err(|error| format!("Failed to {label}: {error}"))?;
        self.publication_barrier(artifact, PublicationBarrier::AfterHardLink);

        let destination_file = open_regular_path(destination)?;
        if !same_file_identity(owned_staging, &destination_file)? {
            return Err(format!(
                "Refused to {label}: published destination does not name the owned file"
            ));
        }

        let cleanup_warning = match open_regular_path(source) {
            Ok(current_staging) if same_file_identity(owned_staging, &current_staging)? => {
                #[cfg(test)]
                if self.fault == Some(cleanup_fault) {
                    Some(format!(
                        "Published {label}, but staging cleanup is pending: injected post-link cleanup failure at {cleanup_fault:?}"
                    ))
                } else {
                    fs::remove_file(source)
                        .err()
                        .map(|error| format!("Published {label}, but staging cleanup is pending: {error}"))
                }
                #[cfg(not(test))]
                {
                    let _ = cleanup_fault;
                    fs::remove_file(source)
                        .err()
                        .map(|error| format!("Published {label}, but staging cleanup is pending: {error}"))
                }
            }
            Ok(_) => Some(format!(
                "Published {label}, but staging cleanup is pending: staging path no longer names the owned file"
            )),
            Err(error) => Some(format!(
                "Published {label}, but staging cleanup is pending: {error}"
            )),
        };
        if let Some(warning) = cleanup_warning {
            crate::stt::log_yap(&warning);
        }
        Ok(destination_file)
    }

    fn publication_barrier(&mut self, artifact: PublicationArtifact, barrier: PublicationBarrier) {
        #[cfg(test)]
        if let Some(mut hook) = self.publication_hook.take() {
            hook(artifact, barrier, &self.paths);
            self.publication_hook = Some(hook);
        }
        let _ = (artifact, barrier);
    }

    #[cfg(test)]
    fn set_data_limit_for_test(&mut self, data_limit: u64) {
        self.data_limit = data_limit;
    }

    #[cfg(test)]
    fn journal_for_test(&self) -> &CaptureJournal {
        &self.journal
    }

    #[cfg(test)]
    pub(crate) fn journal_path_for_test(&self) -> &Path {
        &self.paths.journal_part
    }

    #[cfg(test)]
    fn persist_journal_for_test(&mut self) -> Result<(), String> {
        self.persist_journal()
    }

    #[cfg(test)]
    fn journal_growth_stopped_for_test(&self) -> bool {
        self.journal_growth_stopped
    }
}

fn create_new(path: &Path, label: &str) -> Result<File, String> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .read(true)
        .open(path)
        .map_err(|error| format!("Failed to create {label}: {error}"))
}

pub(crate) fn publish_no_replace(
    source: &Path,
    destination: &Path,
    owned_staging: &File,
    label: &str,
) -> Result<File, String> {
    publish_no_replace_with_after_link(source, destination, owned_staging, label, || {})
}

#[cfg(test)]
pub(crate) fn publish_no_replace_with_after_link_for_test<F>(
    source: &Path,
    destination: &Path,
    owned_staging: &File,
    label: &str,
    after_link: F,
) -> Result<File, String>
where
    F: FnOnce(),
{
    publish_no_replace_with_after_link(source, destination, owned_staging, label, after_link)
}

fn publish_no_replace_with_after_link<F>(
    source: &Path,
    destination: &Path,
    owned_staging: &File,
    label: &str,
    after_link: F,
) -> Result<File, String>
where
    F: FnOnce(),
{
    let opened_staging = open_regular_path(source)?;
    if !same_file_identity(owned_staging, &opened_staging)? {
        return Err(format!(
            "Refused to {label}: staging path no longer names the owned file"
        ));
    }
    drop(opened_staging);
    fs::hard_link(source, destination).map_err(|error| format!("Failed to {label}: {error}"))?;
    after_link();
    let destination_file = open_regular_path(destination)?;
    if !same_file_identity(owned_staging, &destination_file)? {
        return Err(format!(
            "Refused to {label}: published destination does not name the owned file"
        ));
    }
    remove_owned_staging(source, owned_staging, label);
    Ok(destination_file)
}

pub(crate) fn remove_owned_staging(source: &Path, owned_staging: &File, label: &str) {
    let cleanup_warning = match open_regular_path(source) {
        Ok(current_staging) => match same_file_identity(owned_staging, &current_staging) {
            Ok(true) => fs::remove_file(source)
                .err()
                .map(|error| format!("Published {label}, but staging cleanup is pending: {error}")),
            Ok(false) => Some(format!(
                "Published {label}, but staging cleanup is pending: staging path no longer names the owned file"
            )),
            Err(error) => Some(format!("Published {label}, but staging cleanup is pending: {error}")),
        },
        Err(error) => Some(format!("Published {label}, but staging cleanup is pending: {error}")),
    };
    if let Some(warning) = cleanup_warning {
        crate::stt::log_yap(&warning);
    }
}

fn write_wav_header(file: &mut File, data_bytes: u64) -> Result<(), String> {
    let data_bytes = u32::try_from(data_bytes)
        .map_err(|_| "Live recording exceeds the WAV 32-bit data-length limit".to_string())?;
    let riff_bytes = 36u32
        .checked_add(data_bytes)
        .ok_or_else(|| "Live recording exceeds the WAV 32-bit data-length limit".to_string())?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("Failed to seek live audio: {error}"))?;
    file.write_all(b"RIFF")
        .and_then(|_| file.write_all(&riff_bytes.to_le_bytes()))
        .and_then(|_| file.write_all(b"WAVEfmt "))
        .and_then(|_| file.write_all(&16u32.to_le_bytes()))
        .and_then(|_| file.write_all(&1u16.to_le_bytes()))
        .and_then(|_| file.write_all(&1u16.to_le_bytes()))
        .and_then(|_| file.write_all(&16_000u32.to_le_bytes()))
        .and_then(|_| file.write_all(&32_000u32.to_le_bytes()))
        .and_then(|_| file.write_all(&2u16.to_le_bytes()))
        .and_then(|_| file.write_all(&16u16.to_le_bytes()))
        .and_then(|_| file.write_all(b"data"))
        .and_then(|_| file.write_all(&data_bytes.to_le_bytes()))
        .map_err(|error| format!("Failed to write live audio header: {error}"))?;
    file.seek(SeekFrom::End(0))
        .map_err(|error| format!("Failed to seek live audio data: {error}"))?;
    Ok(())
}

fn serialize_journal_record(record: &JournalRecord) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(record)
        .map_err(|error| format!("Failed to serialize recording journal: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_journal_record(file: &mut File, record: &JournalRecord) -> Result<u64, String> {
    let bytes = serialize_journal_record(record)?;
    file.write_all(&bytes)
        .map_err(|error| format!("Failed to write recording journal: {error}"))?;
    Ok(bytes.len() as u64)
}

fn read_journal_append_log(directory: &Path, name: &str) -> Result<CaptureJournal, String> {
    let mut file = open_regular_artifact(directory, name)?;
    let text = read_open_file(&mut file)?;
    parse_journal_append_log(&text)
}

fn parse_journal_append_log(text: &str) -> Result<CaptureJournal, String> {
    if let Ok(snapshot) = serde_json::from_str::<CaptureJournal>(text) {
        validate_timeline_control_metadata(
            &snapshot.session_id,
            snapshot.tracks.values(),
            &snapshot.track_configurations,
            &snapshot.clock_mappings,
            &snapshot.timeline_gaps,
        )?;
        validate_initial_sequence_coverage(&snapshot.sequence_coverage)?;
        return Ok(snapshot);
    }
    let mut journal = None;
    let lines = text.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let record = match serde_json::from_str::<JournalRecord>(line) {
            Ok(record) => record,
            Err(_) if index + 1 == lines.len() && !text.ends_with('\n') => break,
            Err(error) => return Err(format!("Failed to parse recording journal: {error}")),
        };
        match record {
            JournalRecord::Header { journal: header } => {
                if journal.is_some() {
                    return Err("recording journal has multiple headers".into());
                }
                journal = Some(header);
            }
            JournalRecord::Delta { delta } => {
                let Some(recovered) = journal.as_mut() else {
                    return Err("recording journal delta has no header".into());
                };
                apply_journal_delta(recovered, delta)?;
            }
            JournalRecord::Overflow { session_id, .. } => {
                let Some(recovered) = journal.as_ref() else {
                    return Err("recording journal overflow has no header".into());
                };
                if recovered.session_id != session_id {
                    return Err("recording journal overflow session does not match".into());
                }
            }
        }
    }
    let journal = journal.ok_or_else(|| "recording journal has no valid header".to_string())?;
    validate_timeline_control_metadata(
        &journal.session_id,
        journal.tracks.values(),
        &journal.track_configurations,
        &journal.clock_mappings,
        &journal.timeline_gaps,
    )?;
    validate_initial_sequence_coverage(&journal.sequence_coverage)?;
    Ok(journal)
}

pub(crate) fn parse_journal_for_session(
    text: &str,
    session_id: &SessionId,
) -> Result<bool, String> {
    Ok(parse_journal_append_log(text)?.session_id == *session_id)
}

fn apply_journal_delta(journal: &mut CaptureJournal, delta: JournalDelta) -> Result<(), String> {
    if delta.schema_version != CAPTURE_SCHEMA_VERSION || delta.session_id != journal.session_id {
        return Err("recording journal delta does not match the session".into());
    }
    validate_initial_sequence_coverage(&delta.sequence_coverage)?;
    for track in delta.tracks {
        journal.tracks.insert(track.track_id.clone(), track);
    }
    for transition in delta.revision_transitions {
        journal.observe_revision_transition(transition)?;
    }
    if delta.timeline_gap_start_index > journal.timeline_gaps.len() {
        return Err("recording journal timeline-gap delta is out of order".into());
    }
    journal
        .timeline_gaps
        .truncate(delta.timeline_gap_start_index);
    journal.timeline_gaps.extend(delta.timeline_gaps);
    for coverage in delta.sequence_coverage {
        if let Some(existing) = journal
            .sequence_coverage
            .iter_mut()
            .find(|existing| existing.track_id == coverage.track_id)
        {
            *existing = coverage;
        } else {
            journal.sequence_coverage.push(coverage);
        }
    }
    if delta.gap_start_index != journal.sequence_gaps.len().saturating_sub(1) {
        return Err("recording journal gap delta is out of order".into());
    }
    journal.sequence_gaps.truncate(delta.gap_start_index);
    journal.sequence_gaps.extend(delta.sequence_gaps);
    journal.sequence_gap_overflow = delta.sequence_gap_overflow;
    journal.sink_degraded |= delta.sink_degraded;
    Ok(())
}

#[cfg(test)]
fn read_journal_snapshot(path: &Path) -> Result<CaptureJournal, String> {
    let directory = path
        .parent()
        .ok_or_else(|| "recording journal has no parent directory".to_string())?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "recording journal has no file name".to_string())?;
    read_journal_append_log(directory, name)
}

fn write_json_file_open<T: serde::Serialize>(
    path: &Path,
    value: &T,
    label: &str,
) -> Result<File, String> {
    let mut file = create_new(path, label)?;
    serde_json::to_writer(&mut file, value)
        .map_err(|error| format!("Failed to write {label}: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("Failed to write {label}: {error}"))?;
    Ok(file)
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("Failed to hash recording artifact: {error}"))?;
    sha256_open_file(&mut file)
}

fn sha256_open_file(file: &mut File) -> Result<String, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("Failed to hash recording artifact: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Failed to hash recording artifact: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(crate) fn sha256_open_regular_file(file: &mut File) -> Result<String, String> {
    sha256_open_file(file)
}

fn receipt_from_published_sidecar(
    mut file: File,
    file_name: String,
    path: PathBuf,
    expected: &CaptureSidecar,
) -> Result<PublicationReceipt, String> {
    validate_artifact_name(&file_name)?;
    let sha256 = sha256_open_file(&mut file)?;
    let text = read_open_file(&mut file)?;
    let published: CaptureSidecar = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse published capture sidecar: {error}"))?;
    if &published != expected {
        return Err("published capture sidecar does not match the owned sidecar".into());
    }
    Ok(PublicationReceipt {
        file_name,
        sha256,
        status: CaptureStatus::Complete,
        path,
        identity: file_identity(&file)?,
    })
}

fn receipt_from_published_partial_sidecar(
    mut file: File,
    file_name: String,
    path: PathBuf,
    expected: &PartialCaptureSidecar,
) -> Result<PublicationReceipt, String> {
    validate_artifact_name(&file_name)?;
    let sha256 = sha256_open_file(&mut file)?;
    let text = read_open_file(&mut file)?;
    let published: PartialCaptureSidecar = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse published partial capture sidecar: {error}"))?;
    if &published != expected {
        return Err("published partial capture sidecar does not match the owned sidecar".into());
    }
    Ok(PublicationReceipt {
        file_name,
        sha256,
        status: CaptureStatus::Partial,
        path,
        identity: file_identity(&file)?,
    })
}

fn manifest_from_published_commit(
    file: &mut File,
    expected: &CaptureCommitManifest,
) -> Result<CaptureCommitManifest, String> {
    let _commit_sha256 = sha256_open_file(file)?;
    let text = read_open_file(file)?;
    let published: CaptureCommitManifest = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse published capture commit: {error}"))?;
    published.validate()?;
    if &published != expected {
        return Err("published capture commit does not match the owned commit".into());
    }
    Ok(published)
}

fn now_utc() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| "Failed to format recording commit time".to_string())
}

// Windows cannot portably sync a directory handle through std. The false result is persisted so
// callers retain the residual power-loss window instead of mistaking it for durable metadata.
fn sync_parent_directory(directory: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        let _ = directory;
        false
    }
    #[cfg(not(target_os = "windows"))]
    {
        File::open(directory)
            .and_then(|file| file.sync_all())
            .is_ok()
    }
}

pub(crate) fn sync_recordings_parent(directory: &Path) -> bool {
    sync_parent_directory(directory)
}

pub fn validate_artifact_name(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path.components().count() != 1
        || path.file_name().and_then(|name| name.to_str()) != Some(value)
        || value.contains(':')
    {
        return Err("recording artifact names must be same-directory basenames".into());
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), String> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("recording artifact hash is invalid".into())
    }
}

fn read_manifest(directory: &Path, name: &str) -> Result<CaptureCommitManifest, String> {
    let text = read_regular_artifact(directory, name)
        .map_err(|error| format!("Failed to read capture commit: {error}"))?;
    let manifest: CaptureCommitManifest = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse capture commit: {error}"))?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn scan_recordings(directory: &Path) -> Result<RecordingScan, String> {
    if !directory.exists() {
        return Ok(RecordingScan::default());
    }
    let mut scan = RecordingScan::default();
    let mut partial_ids = BTreeSet::new();
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("Failed to read live recordings: {error}"))?
    {
        let entry = entry.map_err(|error| format!("Failed to read live recording: {error}"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_regular_artifact(&directory.join(name)) {
            continue;
        }
        if let Some(session) = session_from_private_artifact(name) {
            if name.ends_with(".capture.journal.part") {
                let _ = read_journal_append_log(directory, name);
            }
            partial_ids.insert(session.as_str().to_string());
            continue;
        }
        if let Some(session) = session_from_orphan_wav_artifact(name) {
            if has_owned_partial_lineage(directory, &session) {
                partial_ids.insert(session.as_str().to_string());
            }
            continue;
        }
        let Some(session) = session_from_commit_artifact(name) else {
            continue;
        };
        match read_manifest(directory, name)
            .and_then(|manifest| validate_committed_capture(directory, manifest))
        {
            Ok(committed) => scan.complete.push(committed),
            Err(complete_error) => {
                match read_recovered_partial_capture(directory, name, &session) {
                    Ok(()) => scan.recovered_partial.push(RecoveredPartialCapture {
                        session_id: session,
                        directory: directory.to_path_buf(),
                    }),
                    Err(_) => scan.damaged.push(DamagedCommittedCapture {
                        session_id: session,
                        directory: directory.to_path_buf(),
                        reason: bounded_scan_reason(&complete_error),
                    }),
                }
            }
        }
    }
    for session in partial_ids {
        let session_id =
            SessionId::new(session).expect("private artifact parser validates session IDs");
        if !scan
            .complete
            .iter()
            .any(|capture| capture.manifest.session_id == session_id)
            && !scan
                .recovered_partial
                .iter()
                .any(|capture| capture.session_id == session_id)
            && !scan
                .damaged
                .iter()
                .any(|capture| capture.session_id == session_id)
        {
            scan.partial.push(PartialCapture {
                session_id: Some(session_id),
                directory: directory.to_path_buf(),
            });
        }
    }
    Ok(scan)
}

fn session_from_commit_artifact(name: &str) -> Option<SessionId> {
    name.strip_prefix("live-")
        .and_then(|value| value.strip_suffix(".commit.json"))
        .and_then(|session| SessionId::new(session.to_string()).ok())
}

fn bounded_scan_reason(error: &str) -> String {
    let detail = error.chars().take(160).collect::<String>();
    format!("Damaged complete capture commit: {detail}")
}

fn read_recovered_partial_capture(
    directory: &Path,
    name: &str,
    expected_session: &SessionId,
) -> Result<(), String> {
    let text = read_regular_artifact(directory, name)
        .map_err(|error| format!("Failed to read partial recovery commit: {error}"))?;
    let commit: PartialRecoveryCommit = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse partial recovery commit: {error}"))?;
    commit.validate()?;
    if &commit.session_id != expected_session {
        return Err("partial recovery commit session does not match its file name".into());
    }
    let mut audio = open_regular_artifact(directory, &commit.audio_file)?;
    let mut sidecar = open_regular_artifact(directory, &commit.capture_sidecar_file)?;
    if audio
        .metadata()
        .map_err(|error| format!("Failed to inspect recovered partial audio: {error}"))?
        .len()
        != commit.audio_bytes
        || sha256_open_file(&mut audio)? != commit.audio_sha256
        || sha256_open_file(&mut sidecar)? != commit.capture_sidecar_sha256
    {
        return Err("partial recovery artifact hash does not match the commit".into());
    }
    let sidecar = read_open_file(&mut sidecar)
        .map_err(|error| format!("Failed to read partial recovery sidecar: {error}"))?;
    let value: serde_json::Value = serde_json::from_str(&sidecar)
        .map_err(|error| format!("Failed to parse partial recovery sidecar: {error}"))?;
    if value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
        || value.get("sessionId").and_then(serde_json::Value::as_str)
            != Some(expected_session.as_str())
        || value.get("status").and_then(serde_json::Value::as_str) != Some("partial")
    {
        return Err("partial recovery sidecar does not match the commit".into());
    }
    Ok(())
}

fn validate_committed_capture(
    directory: &Path,
    manifest: CaptureCommitManifest,
) -> Result<CommittedCapture, String> {
    manifest.validate()?;
    let mut audio = open_regular_artifact(directory, &manifest.audio_file)?;
    let mut sidecar = open_regular_artifact(directory, &manifest.capture_sidecar_file)?;
    if audio
        .metadata()
        .map_err(|error| format!("Failed to inspect committed audio: {error}"))?
        .len()
        != manifest.audio_bytes
    {
        return Err("committed audio size does not match the manifest".into());
    }
    if sha256_open_file(&mut audio)? != manifest.audio_sha256
        || sha256_open_file(&mut sidecar)? != manifest.capture_sidecar_sha256
    {
        return Err("committed recording artifact hash does not match the manifest".into());
    }
    let sidecar_text = read_open_file(&mut sidecar)
        .map_err(|error| format!("Failed to read capture sidecar: {error}"))?;
    let sidecar: CaptureSidecar = serde_json::from_str(&sidecar_text)
        .map_err(|error| format!("Failed to parse capture sidecar: {error}"))?;
    sidecar.validate(&manifest)?;
    Ok(CommittedCapture {
        manifest,
        directory: directory.to_path_buf(),
    })
}

pub(crate) fn is_regular_artifact(path: &Path) -> bool {
    let Some(directory) = path.parent() else {
        return false;
    };
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    open_regular_artifact(directory, name).is_ok()
}

pub(crate) fn open_regular_artifact(directory: &Path, name: &str) -> Result<File, String> {
    validate_artifact_name(name)?;
    let path = directory.join(name);
    let file = open_no_follow(&path)
        .map_err(|error| format!("Failed to open recording artifact: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Failed to inspect recording artifact: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("recording artifact is not a regular file".into());
    }
    #[cfg(windows)]
    if metadata.file_attributes() & 0x400 != 0 {
        return Err("recording artifact is a reparse point".into());
    }
    Ok(file)
}

pub(crate) fn recover_partial_wav_with_identity(
    directory: &Path,
    session_id: &SessionId,
    expected: &RegularArtifactIdentity,
) -> Result<(String, u64, String), String> {
    recover_partial_wav_with_admitted_identity(directory, session_id, Some(expected))
}

fn recover_partial_wav_with_admitted_identity(
    directory: &Path,
    session_id: &SessionId,
    expected: Option<&RegularArtifactIdentity>,
) -> Result<(String, u64, String), String> {
    let source_name = format!("live-{session_id}.wav.part");
    let destination_name = format!("live-{session_id}.wav");
    let source = directory.join(&source_name);
    let destination = directory.join(&destination_name);
    if open_regular_artifact(directory, &source_name).is_err() {
        let mut orphan = open_regular_artifact(directory, &destination_name)?;
        if let Some(expected) = expected {
            if !expected.matches_artifact_name(&destination_name) {
                return Err(
                    "admitted recovery artifact no longer matches the current session".into(),
                );
            }
            expected.ensure_open_file(&orphan)?;
        }
        validate_recoverable_wav(&mut orphan, "recovered live audio")?;
        let bytes = orphan
            .metadata()
            .map_err(|error| format!("Failed to inspect recovered live audio: {error}"))?
            .len();
        let hash = sha256_open_file(&mut orphan)?;
        return Ok((destination_name, bytes, hash));
    }

    let mut audio = open_regular_artifact_for_update(directory, &source_name)?;
    if let Some(expected) = expected {
        if !expected.matches_artifact_name(&source_name) {
            return Err("admitted recovery artifact no longer matches the current session".into());
        }
        expected.ensure_open_file(&audio)?;
    }
    let (data_bytes, _) = validate_recoverable_wav(&mut audio, "partial live audio")?;
    write_wav_header(&mut audio, data_bytes)?;
    audio
        .sync_all()
        .map_err(|error| format!("Failed to sync recovered live audio: {error}"))?;
    let mut published = publish_no_replace(
        &source,
        &destination,
        &audio,
        "publish recovered live audio",
    )?;
    let published_bytes = published
        .metadata()
        .map_err(|error| format!("Failed to inspect recovered live audio: {error}"))?
        .len();
    let hash = sha256_open_file(&mut published)?;
    Ok((destination_name, published_bytes, hash))
}

pub(crate) fn admit_expected_regular_artifact(
    actual_path: &Path,
    expected_path: &Path,
) -> Result<RegularArtifactIdentity, String> {
    admit_expected_regular_artifact_with_link_policy(actual_path, expected_path, false)
}

pub(crate) fn admit_regular_artifact(path: &Path) -> Result<RegularArtifactIdentity, String> {
    let file = open_regular_path(path)?;
    Ok(RegularArtifactIdentity {
        path: path.to_path_buf(),
        identity: file_identity(&file)?,
        require_single_link: false,
    })
}

pub(crate) fn admit_expected_private_regular_artifact(
    actual_path: &Path,
    expected_path: &Path,
) -> Result<RegularArtifactIdentity, String> {
    admit_expected_regular_artifact_with_link_policy(actual_path, expected_path, true)
}

fn admit_expected_regular_artifact_with_link_policy(
    actual_path: &Path,
    expected_path: &Path,
    require_single_link: bool,
) -> Result<RegularArtifactIdentity, String> {
    let actual = open_regular_path(actual_path)?;
    let expected = open_regular_path(expected_path)?;
    if !same_file_identity(&actual, &expected)? {
        return Err(
            "Live recording identity is no longer current. Refresh history and try again.".into(),
        );
    }
    let admitted = RegularArtifactIdentity {
        path: actual_path.to_path_buf(),
        identity: file_identity(&actual)?,
        require_single_link,
    };
    admitted.ensure_link_ownership(&actual)?;
    Ok(admitted)
}

pub(crate) fn remove_regular_artifact(directory: &Path, name: &str) -> Result<(), String> {
    let owned = open_regular_artifact(directory, name)?;
    remove_open_regular_artifact(directory, name, &owned, || {})
}

pub(crate) fn quarantine_regular_artifact(
    directory: &Path,
    name: &str,
) -> Result<QuarantinedArtifact, String> {
    let mut owned = open_regular_artifact(directory, name)?;
    let sha256 = sha256_open_file(&mut owned)?;
    let identity = file_identity(&owned)?;
    let path = quarantine_open_regular_artifact(directory, name, &owned)?;
    Ok(QuarantinedArtifact {
        path,
        sha256,
        identity,
    })
}

pub(crate) fn verified_regular_artifact(
    directory: &Path,
    name: &str,
) -> Result<QuarantinedArtifact, String> {
    let mut owned = open_regular_artifact(directory, name)?;
    let sha256 = sha256_open_file(&mut owned)?;
    let identity = file_identity(&owned)?;
    Ok(QuarantinedArtifact {
        path: directory.join(name),
        sha256,
        identity,
    })
}

pub(crate) fn remove_verified_quarantined_artifact(
    artifact: &QuarantinedArtifact,
) -> Result<(), String> {
    let mut current = open_regular_path(&artifact.path)?;
    if file_identity(&current)? != artifact.identity
        || sha256_open_file(&mut current)? != artifact.sha256
    {
        return Err(
            "quarantined recording artifact no longer matches its verified identity or hash".into(),
        );
    }
    let directory = artifact
        .path
        .parent()
        .ok_or_else(|| "quarantined recording artifact has no parent directory".to_string())?;
    let name = artifact
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "quarantined recording artifact has no valid file name".to_string())?;
    remove_open_regular_artifact(directory, name, &current, || {})
}

pub(crate) fn restore_verified_quarantined_artifact(
    artifact: &QuarantinedArtifact,
    destination: &Path,
) -> Result<(), String> {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "recording artifact has no valid restore destination".to_string())?;
    validate_artifact_name(name)?;
    if destination.exists() {
        return Err("recording artifact restore destination is already occupied".into());
    }
    let mut current = open_regular_path(&artifact.path)?;
    if file_identity(&current)? != artifact.identity
        || sha256_open_file(&mut current)? != artifact.sha256
    {
        return Err(
            "quarantined recording artifact no longer matches its verified identity or hash".into(),
        );
    }
    fs::hard_link(&artifact.path, destination)
        .map_err(|error| format!("Failed to restore quarantined recording artifact: {error}"))?;
    let restored = open_regular_path(destination)?;
    if file_identity(&restored)? != artifact.identity {
        let _ = fs::remove_file(destination);
        return Err("restored recording artifact no longer matches its verified identity".into());
    }
    drop(restored);
    drop(current);
    remove_verified_quarantined_artifact(artifact)
}

#[cfg(test)]
pub(crate) fn remove_regular_artifact_if_hash(
    directory: &Path,
    name: &str,
    expected_sha256: &str,
) -> Result<(), String> {
    let mut owned = open_regular_artifact(directory, name)?;
    if sha256_open_file(&mut owned)? != expected_sha256 {
        return Err("recording artifact no longer matches its validated hash".into());
    }
    remove_open_regular_artifact(directory, name, &owned, || {})
}

pub(crate) fn revalidate_regular_artifact_identity(
    expected: &RegularArtifactIdentity,
) -> Result<(), String> {
    expected.open_current().map(drop)
}

pub(crate) fn remove_regular_artifact_if_identity_and_hash(
    directory: &Path,
    name: &str,
    expected: &RegularArtifactIdentity,
    expected_sha256: &str,
) -> Result<(), String> {
    let path = directory.join(name);
    if !expected.matches_artifact_name(name) {
        return Err("admitted recording artifact no longer matches the deletion target".into());
    }
    let mut owned = expected.open_current_at(&path)?;
    if sha256_open_file(&mut owned)? != expected_sha256 {
        return Err("recording artifact no longer matches its validated hash".into());
    }
    remove_open_regular_artifact(directory, name, &owned, || {})
}

pub(crate) fn revalidate_regular_artifact_file_identity_and_hash(
    directory: &Path,
    name: &str,
    expected: &FileIdentity,
    expected_sha256: &str,
) -> Result<(), String> {
    let mut owned = open_regular_artifact(directory, name)?;
    if file_identity(&owned)? != *expected {
        return Err("recording artifact no longer matches its admitted identity".into());
    }
    if sha256_open_file(&mut owned)? != expected_sha256 {
        return Err("recording artifact no longer matches its validated hash".into());
    }
    Ok(())
}

pub(crate) fn remove_regular_artifact_if_file_identity_and_hash(
    directory: &Path,
    name: &str,
    expected: &FileIdentity,
    expected_sha256: &str,
) -> Result<(), String> {
    let mut owned = open_regular_artifact(directory, name)?;
    if file_identity(&owned)? != *expected {
        return Err("recording artifact no longer matches its admitted identity".into());
    }
    if sha256_open_file(&mut owned)? != expected_sha256 {
        return Err("recording artifact no longer matches its validated hash".into());
    }
    remove_open_regular_artifact(directory, name, &owned, || {})
}

#[cfg(test)]
fn remove_regular_artifact_with_barrier_for_test<F>(
    directory: &Path,
    name: &str,
    barrier: F,
) -> Result<(), String>
where
    F: FnOnce(&Path),
{
    let owned = open_regular_artifact(directory, name)?;
    remove_open_regular_artifact(directory, name, &owned, || barrier(&directory.join(name)))
}

fn remove_open_regular_artifact<F>(
    directory: &Path,
    name: &str,
    owned: &File,
    before_quarantine: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    before_quarantine();
    let quarantine = quarantine_open_regular_artifact(directory, name, owned)?;
    fs::remove_file(&quarantine)
        .map_err(|error| format!("Failed to remove quarantined recording artifact: {error}"))
}

fn quarantine_open_regular_artifact(
    directory: &Path,
    name: &str,
    owned: &File,
) -> Result<PathBuf, String> {
    validate_artifact_name(name)?;
    let path = directory.join(name);
    let quarantine = unique_delete_quarantine_path(directory, name)?;
    fs::rename(&path, &quarantine).map_err(|error| {
        format!("Failed to quarantine recording artifact for deletion: {error}")
    })?;
    let quarantined = match open_regular_path(&quarantine) {
        Ok(file) => file,
        Err(error) => {
            return Err(format!(
                "Failed to verify quarantined recording artifact: {error}"
            ))
        }
    };
    if !same_file_identity(owned, &quarantined)? {
        let _ = restore_quarantined_artifact(&quarantine, &path);
        return Err("recording artifact path no longer names the verified file".into());
    }
    drop(quarantined);
    Ok(quarantine)
}

fn unique_delete_quarantine_path(directory: &Path, name: &str) -> Result<PathBuf, String> {
    for _ in 0..128 {
        let nonce = DELETE_QUARANTINE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let candidate = directory.join(format!(".{name}.delete-{}-{nonce}", std::process::id()));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("Failed to allocate a private recording deletion quarantine path".into())
}

fn restore_quarantined_artifact(quarantine: &Path, path: &Path) -> Result<(), String> {
    std::fs::hard_link(quarantine, path)
        .map_err(|error| format!("Failed to restore quarantined recording artifact: {error}"))?;
    std::fs::remove_file(quarantine).map_err(|error| {
        format!("Failed to finish restoring quarantined recording artifact: {error}")
    })
}

fn validate_recoverable_wav(file: &mut File, label: &str) -> Result<(u64, bool), String> {
    let length = file
        .metadata()
        .map_err(|error| format!("Failed to inspect {label}: {error}"))?
        .len();
    if length < WAV_HEADER_BYTES {
        return Err(format!("{label} is shorter than a WAV header"));
    }
    let mut header = [0_u8; WAV_HEADER_BYTES as usize];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut header))
        .map_err(|error| format!("Failed to read {label} header: {error}"))?;
    let read_u16 = |offset: usize| u16::from_le_bytes([header[offset], header[offset + 1]]);
    let read_u32 = |offset: usize| {
        u32::from_le_bytes([
            header[offset],
            header[offset + 1],
            header[offset + 2],
            header[offset + 3],
        ])
    };
    if &header[0..4] != b"RIFF"
        || &header[8..12] != b"WAVE"
        || &header[12..16] != b"fmt "
        || read_u32(16) != 16
        || read_u16(20) != 1
        || read_u16(22) != 1
        || read_u32(24) != 16_000
        || read_u32(28) != 32_000
        || read_u16(32) != 2
        || read_u16(34) != 16
        || &header[36..40] != b"data"
    {
        return Err(format!(
            "{label} is not Yap PCM mono 16 kHz 16-bit WAV audio"
        ));
    }
    let data_bytes = length - WAV_HEADER_BYTES;
    if !data_bytes.is_multiple_of(PCM16_BYTES_PER_SAMPLE) {
        return Err(format!("{label} has an unaligned PCM data length"));
    }
    let riff_bytes = u64::from(read_u32(4));
    let declared_data_bytes = u64::from(read_u32(40));
    let placeholder = riff_bytes == 36 && declared_data_bytes == 0;
    let finalized = riff_bytes == 36 + data_bytes && declared_data_bytes == data_bytes;
    if !placeholder && !finalized {
        return Err(format!(
            "{label} header lengths do not match its opened file length"
        ));
    }
    Ok((data_bytes, placeholder))
}

pub(crate) fn sha256_regular_artifact(directory: &Path, name: &str) -> Result<String, String> {
    let mut file = open_regular_artifact(directory, name)?;
    sha256_open_file(&mut file)
}

fn open_regular_path(path: &Path) -> Result<File, String> {
    let directory = path
        .parent()
        .ok_or_else(|| "recording artifact has no parent directory".to_string())?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "recording artifact has no valid file name".to_string())?;
    open_regular_artifact(directory, name)
}

fn open_regular_artifact_for_update(directory: &Path, name: &str) -> Result<File, String> {
    validate_artifact_name(name)?;
    let file = open_no_follow_update(&directory.join(name))
        .map_err(|error| format!("Failed to open recording artifact for update: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Failed to inspect recording artifact: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("recording artifact is not a regular file".into());
    }
    #[cfg(windows)]
    if metadata.file_attributes() & 0x400 != 0 {
        return Err("recording artifact is a reparse point".into());
    }
    Ok(file)
}

#[cfg(unix)]
fn same_file_identity(left: &File, right: &File) -> Result<bool, String> {
    Ok(file_identity(left)? == file_identity(right)?)
}

#[cfg(windows)]
fn same_file_identity(left: &File, right: &File) -> Result<bool, String> {
    Ok(file_identity(left)? == file_identity(right)?)
}

#[cfg(not(any(unix, windows)))]
fn same_file_identity(_left: &File, _right: &File) -> Result<bool, String> {
    Err("recording publication ownership is unsupported on this platform".into())
}

#[cfg(unix)]
fn file_identity(file: &File) -> Result<FileIdentity, String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file
        .metadata()
        .map_err(|error| format!("Failed to inspect recording file identity: {error}"))?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(unix)]
fn file_link_count(file: &File) -> Result<u64, String> {
    use std::os::unix::fs::MetadataExt;

    file.metadata()
        .map(|metadata| metadata.nlink())
        .map_err(|error| format!("Failed to inspect recording file link count: {error}"))
}

#[cfg(windows)]
fn file_link_count(file: &File) -> Result<u64, String> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            &mut information as *mut BY_HANDLE_FILE_INFORMATION,
        )
    }
    .map_err(|error| format!("Failed to inspect recording file link count: {error}"))?;
    Ok(u64::from(information.nNumberOfLinks))
}

#[cfg(not(any(unix, windows)))]
fn file_link_count(_file: &File) -> Result<u64, String> {
    Err("recording link ownership is unsupported on this platform".into())
}

#[cfg(windows)]
fn file_identity(file: &File) -> Result<FileIdentity, String> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            &mut information as *mut BY_HANDLE_FILE_INFORMATION,
        )
    }
    .map_err(|error| format!("Failed to inspect recording file identity: {error}"))?;
    Ok(FileIdentity {
        volume_serial: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

#[cfg(not(any(unix, windows)))]
fn file_identity(_file: &File) -> Result<FileIdentity, String> {
    Err("recording publication ownership is unsupported on this platform".into())
}

pub(crate) fn read_regular_artifact(directory: &Path, name: &str) -> Result<String, String> {
    let mut file = open_regular_artifact(directory, name)?;
    read_open_file(&mut file)
}

pub(crate) fn read_and_hash_regular_artifact(
    directory: &Path,
    name: &str,
) -> Result<(String, String), String> {
    let mut file = open_regular_artifact(directory, name)?;
    let text = read_open_file(&mut file)?;
    let hash = Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((text, hash))
}

fn read_open_file(file: &mut File) -> Result<String, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("Failed to read recording artifact: {error}"))?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|error| format!("Failed to read recording artifact: {error}"))?;
    Ok(text)
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(unix)]
fn open_no_follow_update(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(windows)]
fn open_no_follow_update(path: &Path) -> std::io::Result<File> {
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_no_follow_update(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).open(path)
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

fn session_from_private_artifact(name: &str) -> Option<SessionId> {
    let session = name.strip_prefix("live-")?;
    [
        ".wav.part",
        ".capture.journal.part",
        ".capture.json.part",
        ".capture.partial.json",
        ".capture.partial.json.part",
        ".commit.json.part",
    ]
    .into_iter()
    .find_map(|suffix| session.strip_suffix(suffix))
    .and_then(|session| SessionId::new(session).ok())
}

fn session_from_orphan_wav_artifact(name: &str) -> Option<SessionId> {
    let session = name.strip_prefix("live-")?.strip_suffix(".wav")?;
    SessionId::new(session)
        .ok()
        .filter(SessionId::is_current_writer_id)
}

fn has_owned_partial_lineage(directory: &Path, session_id: &SessionId) -> bool {
    let name = format!("live-{session_id}.capture.partial.json");
    let Ok(text) = read_regular_artifact(directory, &name) else {
        return false;
    };
    serde_json::from_str::<PartialCaptureSidecar>(&text)
        .map(|sidecar| {
            sidecar.schema_version == CAPTURE_SCHEMA_VERSION
                && sidecar.session_id == *session_id
                && sidecar.status == CaptureStatus::Partial
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitFaultPoint {
    Append,
    PeriodicFlush,
    JournalAppend,
    JournalSync,
    WavHeaderPatch,
    AudioSync,
    SidecarSync,
    FinalArtifactRename,
    CommitSync,
    CommitRename,
    AudioStagingCleanup,
    SidecarStagingCleanup,
    CommitStagingCleanup,
}

impl CommitFaultPoint {
    #[cfg(test)]
    const ALL: [Self; 10] = [
        Self::Append,
        Self::PeriodicFlush,
        Self::JournalAppend,
        Self::JournalSync,
        Self::WavHeaderPatch,
        Self::AudioSync,
        Self::SidecarSync,
        Self::FinalArtifactRename,
        Self::CommitSync,
        Self::CommitRename,
    ];
}

#[cfg(test)]
mod tests;
