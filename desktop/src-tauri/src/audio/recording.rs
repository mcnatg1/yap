use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};
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

use crate::audio::coordinator::{BoundedReceiver, BoundedSink};
use crate::audio::frame::PreparedFrame;
use crate::audio::preprocess::f32_to_i16_le_bytes;
use crate::audio::session::{self, SessionId};

const CAPTURE_SCHEMA_VERSION: u16 = 1;
const WAV_HEADER_BYTES: u64 = 44;
const PCM16_BYTES_PER_SAMPLE: u64 = 2;
const DEFAULT_SYNC_INTERVAL_SAMPLES: u64 = 16_000;
const MAX_SEQUENCE_GAP_DETAILS: usize = 1_024;
const MAX_JOURNAL_BYTES: u64 = 512 * 1024;
const MAX_JOURNAL_RECORD_BYTES: u64 = 8 * 1024;
const MAX_JOURNAL_TERMINAL_BYTES: u64 = 256;

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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecordingScan {
    pub complete: Vec<CommittedCapture>,
    pub partial: Vec<PartialCapture>,
}

impl RecordingScan {
    pub fn is_empty(&self) -> bool {
        self.complete.is_empty() && self.partial.is_empty()
    }

    pub fn len(&self) -> usize {
        self.complete.len()
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
    sink: BoundedSink<PreparedFrame>,
    session_id: SessionId,
    abort_reason: Arc<Mutex<Option<String>>>,
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
        sink: BoundedSink<PreparedFrame>,
        receiver: BoundedReceiver<PreparedFrame>,
    ) -> Self {
        Self::spawn_inner(directory, session_id, sink, receiver, false)
    }

    pub(crate) fn spawn_reserved(
        directory: PathBuf,
        session_id: SessionId,
        sink: BoundedSink<PreparedFrame>,
        receiver: BoundedReceiver<PreparedFrame>,
    ) -> Self {
        Self::spawn_inner(directory, session_id, sink, receiver, true)
    }

    fn spawn_inner(
        directory: PathBuf,
        session_id: SessionId,
        sink: BoundedSink<PreparedFrame>,
        receiver: BoundedReceiver<PreparedFrame>,
        reserved_wav_part: bool,
    ) -> Self {
        let worker_session_id = session_id.clone();
        let abort_reason = Arc::new(Mutex::new(None));
        let worker_abort_reason = Arc::clone(&abort_reason);
        let worker = std::thread::spawn(move || {
            run_recording_worker(
                directory,
                worker_session_id,
                receiver,
                reserved_wav_part,
                worker_abort_reason,
            )
        });
        Self::with_worker(sink, session_id, worker, abort_reason)
    }

    fn with_worker(
        sink: BoundedSink<PreparedFrame>,
        session_id: SessionId,
        worker: JoinHandle<RecordingFinalizeResult>,
        abort_reason: Arc<Mutex<Option<String>>>,
    ) -> Self {
        Self {
            sink,
            session_id,
            abort_reason,
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
        sink: BoundedSink<PreparedFrame>,
        receiver: BoundedReceiver<PreparedFrame>,
        fault: CommitFaultPoint,
        append_write_attempts: Arc<std::sync::atomic::AtomicUsize>,
        journal_write_attempts: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        let worker_session_id = session_id.clone();
        let abort_reason = Arc::new(Mutex::new(None));
        let worker_abort_reason = Arc::clone(&abort_reason);
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
            drain_recording_worker(recording, worker_session_id, receiver, worker_abort_reason)
        });
        Self::with_worker(sink, session_id, worker, abort_reason)
    }

    #[cfg(test)]
    pub(crate) fn spawn_panicking_for_test(
        sink: BoundedSink<PreparedFrame>,
        _receiver: BoundedReceiver<PreparedFrame>,
        session_id: SessionId,
    ) -> Self {
        Self::with_worker(
            sink,
            session_id,
            std::thread::spawn(|| -> RecordingFinalizeResult {
                panic!("injected recording worker panic")
            }),
            Arc::new(Mutex::new(None)),
        )
    }

    #[cfg(test)]
    pub(crate) fn spawn_unavailable_for_test(
        sink: BoundedSink<PreparedFrame>,
        session_id: SessionId,
    ) -> Self {
        Self {
            sink,
            session_id,
            abort_reason: Arc::new(Mutex::new(None)),
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
        sink: BoundedSink<PreparedFrame>,
        receiver: BoundedReceiver<PreparedFrame>,
    ) -> (Self, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let handle = Self::spawn(directory, session_id, sink, receiver);
        let count = std::sync::Arc::clone(&handle.finalization_count);
        (handle, count)
    }

    pub fn sink(&self) -> BoundedSink<PreparedFrame> {
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
        state.finalizing = false;
        state.result = Some(result.clone());
        self.completed.notify_all();
        result
    }

    pub fn abort(&self, reason: impl Into<String>) -> Result<RecordingFinalizeResult, String> {
        let mut abort_reason = self
            .abort_reason
            .lock()
            .map_err(|_| "recording abort state became unavailable")?;
        abort_reason.get_or_insert_with(|| reason.into());
        drop(abort_reason);
        self.finalize()
    }
}

fn run_recording_worker(
    directory: PathBuf,
    session_id: SessionId,
    receiver: BoundedReceiver<PreparedFrame>,
    reserved_wav_part: bool,
    abort_reason: Arc<Mutex<Option<String>>>,
) -> RecordingFinalizeResult {
    let recording = match if reserved_wav_part {
        StreamingRecording::create_reserved(&directory, session_id.clone())
    } else {
        StreamingRecording::create(&directory, session_id.clone())
    } {
        Ok(recording) => recording,
        Err(error) => return worker_creation_failure(session_id, error),
    };
    drain_recording_worker(recording, session_id, receiver, abort_reason)
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
    receiver: BoundedReceiver<PreparedFrame>,
    abort_reason: Arc<Mutex<Option<String>>>,
) -> RecordingFinalizeResult {
    let mut terminal_error = None;
    loop {
        match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(frame) => {
                if terminal_error.is_none() {
                    if let Err(error) = recording.append_prepared(&frame) {
                        terminal_error = Some(error);
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if let Ok(mut abort_reason) = abort_reason.lock() {
        if let Some(reason) = abort_reason.take() {
            recording.abort(reason);
        }
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
    clock_mappings: Vec<JournalClockMapping>,
    sequence_coverage: Vec<SequenceCoverage>,
    sequence_gaps: Vec<SequenceGap>,
    #[serde(default)]
    sequence_gap_overflow: Option<SequenceGapOverflow>,
    sink_degraded: bool,
    directory_sync_supported: bool,
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
        validate_artifact_name(&self.audio_file)?;
        validate_sha256(&self.audio_sha256)
    }
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
struct JournalClockMapping {
    track_id: String,
    sequence: u64,
    session_time_ms: u64,
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
    clock_mappings: Vec<JournalClockMapping>,
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
            clock_mappings: Vec::new(),
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
        if !self
            .clock_mappings
            .iter()
            .any(|mapping| mapping.track_id == track_id)
        {
            self.clock_mappings.push(JournalClockMapping {
                track_id: track_id.to_string(),
                sequence,
                session_time_ms: start_ms,
            });
        }

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
    clock_mappings: Vec<JournalClockMapping>,
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
    clock_mappings: usize,
    sequence_coverage: BTreeMap<String, SequenceCoverage>,
    sequence_gaps: usize,
}

impl DurableJournalState {
    fn from_journal(journal: &CaptureJournal) -> Self {
        Self {
            tracks: journal.tracks.keys().cloned().collect(),
            clock_mappings: journal.clock_mappings.len(),
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
        JournalDelta {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id: journal.session_id.clone(),
            tracks: journal
                .tracks
                .iter()
                .filter(|(track_id, _)| !self.tracks.contains(*track_id))
                .map(|(_, track)| track.clone())
                .collect(),
            clock_mappings: journal.clock_mappings[self.clock_mappings..].to_vec(),
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

pub struct StreamingRecording {
    paths: RecordingPaths,
    audio: Option<File>,
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
    file: Arc<File>,
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
        if !same_file_identity(&self.file, &current)? {
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

/// Allocates a persistent recording identity by atomically reserving its first
/// on-disk artifact. Runtime-local counters are deliberately not part of this ID.
pub(crate) fn allocate_recording_session(directory: &Path) -> Result<SessionId, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("Failed to create live recordings folder: {error}"))?;
    session::allocate_recording(|session_id| reserve_wav_part(directory, session_id))
}

fn reserve_wav_part(directory: &Path, session_id: &SessionId) -> std::io::Result<()> {
    reserve_wav_part_with_before_claim(directory, session_id, || {})
}

fn reserve_wav_part_with_before_claim<F>(
    directory: &Path,
    session_id: &SessionId,
    before_claim: F,
) -> std::io::Result<()>
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
        .write(true)
        .open(&paths.wav_part)?;
    file.sync_all()?;
    Ok(())
}

impl StreamingRecording {
    pub fn create(directory: &Path, session_id: SessionId) -> Result<Self, String> {
        Self::create_inner(directory, session_id, false, None)
    }

    pub(crate) fn create_reserved(directory: &Path, session_id: SessionId) -> Result<Self, String> {
        Self::create_inner(directory, session_id, true, None)
    }

    #[cfg(test)]
    pub(crate) fn create_with_fault(
        directory: &Path,
        session_id: SessionId,
        fault: CommitFaultPoint,
    ) -> Result<Self, String> {
        Self::create_inner(directory, session_id, false, Some(fault))
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
        let mut recording = Self::create_inner(directory, session_id, false, None)?;
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
        let mut recording = Self::create_inner(directory, session_id, false, fault)?;
        recording.publication_hook = Some(Box::new(hook));
        Ok(recording)
    }

    fn create_inner(
        directory: &Path,
        session_id: SessionId,
        reserved_wav_part: bool,
        #[cfg(test)] fault: Option<CommitFaultPoint>,
        #[cfg(not(test))] _fault: Option<()>,
    ) -> Result<Self, String> {
        fs::create_dir_all(directory)
            .map_err(|error| format!("Failed to create live recordings folder: {error}"))?;
        let paths = RecordingPaths::new(directory, session_id.clone());
        let mut audio = if reserved_wav_part {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&paths.wav_part)
                .map_err(|error| format!("Failed to open reserved recording audio: {error}"))?
        } else {
            create_new(&paths.wav_part, "recording audio")?
        };
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
        let journal = CaptureJournal::new(session_id);
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
            audio: Some(audio),
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

    pub fn append_prepared(&mut self, frame: &PreparedFrame) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        self.observe_frame_metadata(
            frame.metadata.track_id.as_str(),
            frame.metadata.sample_rate_hz,
            frame.metadata.channels,
            frame.metadata.sequence,
            frame.metadata.start_ms,
            frame.metadata.duration_ms,
        );
        self.append_pcm16(&f32_to_i16_le_bytes(&frame.samples))
    }

    pub fn append_pcm16(&mut self, pcm: &[u8]) -> Result<(), String> {
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

    pub fn observe_frame_metadata(
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
        drop(self.journal_file.take());

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
            clock_mappings: self.journal.clock_mappings.clone(),
            sequence_coverage: self.journal.sequence_coverage.clone(),
            sequence_gaps: self.journal.sequence_gaps.clone(),
            sequence_gap_overflow: self.journal.sequence_gap_overflow.clone(),
            sink_degraded: self.journal.sink_degraded,
            directory_sync_supported: sync_parent_directory(&self.paths.directory),
        };
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
        let _ = sync_parent_directory(&self.paths.directory);
        Ok(manifest)
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
        if self.journal_growth_stopped {
            return Ok(());
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
        Ok(())
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
    journal.ok_or_else(|| "recording journal has no valid header".into())
}

fn apply_journal_delta(journal: &mut CaptureJournal, delta: JournalDelta) -> Result<(), String> {
    if delta.schema_version != CAPTURE_SCHEMA_VERSION || delta.session_id != journal.session_id {
        return Err("recording journal delta does not match the session".into());
    }
    for track in delta.tracks {
        journal.tracks.insert(track.track_id.clone(), track);
    }
    journal.clock_mappings.extend(delta.clock_mappings);
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
    if delta.gap_start_index > journal.sequence_gaps.len() {
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

pub(crate) fn sha256_regular_artifact(directory: &Path, name: &str) -> Result<String, String> {
    let mut file = open_regular_artifact(directory, name)?;
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
        file: Arc::new(file),
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
        file: Arc::new(file),
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
        if !name.starts_with("live-") || !name.ends_with(".commit.json") {
            continue;
        }
        match read_manifest(directory, name)
            .and_then(|manifest| validate_committed_capture(directory, manifest))
        {
            Ok(committed) => scan.complete.push(committed),
            Err(_) => {
                if let Some(session) = session_from_commit_name(name) {
                    partial_ids.insert(session.as_str().to_string());
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
        {
            scan.partial.push(PartialCapture {
                session_id: Some(session_id),
                directory: directory.to_path_buf(),
            });
        }
    }
    Ok(scan)
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

#[cfg(unix)]
fn same_file_identity(left: &File, right: &File) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;

    let left = left
        .metadata()
        .map_err(|error| format!("Failed to inspect owned recording artifact: {error}"))?;
    let right = right
        .metadata()
        .map_err(|error| format!("Failed to inspect published recording artifact: {error}"))?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(windows)]
fn same_file_identity(left: &File, right: &File) -> Result<bool, String> {
    fn identity(file: &File) -> Result<(u32, u64), String> {
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                &mut information as *mut BY_HANDLE_FILE_INFORMATION,
            )
        }
        .map_err(|error| format!("Failed to inspect recording file identity: {error}"))?;
        let file_index =
            (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
        Ok((information.dwVolumeSerialNumber, file_index))
    }

    Ok(identity(left)? == identity(right)?)
}

#[cfg(not(any(unix, windows)))]
fn same_file_identity(_left: &File, _right: &File) -> Result<bool, String> {
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

fn session_from_commit_name(name: &str) -> Option<SessionId> {
    let session = name.strip_prefix("live-")?.strip_suffix(".commit.json")?;
    SessionId::new(session).ok()
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
mod tests {
    use super::*;
    use crate::audio::coordinator::{bounded_sink, SinkKind};
    use crate::audio::frame::AudioFrame;
    use crate::audio::session::SessionId;
    use crate::audio::session::TrackId;
    use std::sync::Arc;

    #[test]
    fn streamed_pcm_finalizes_only_after_a_commit_manifest() {
        let dir = tempfile_dir("commit-last");
        let session = SessionId::new("s-commit-last").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0, 2, 0]).unwrap();

        let pending = scan_recordings(&dir).unwrap();
        assert!(pending.complete.is_empty());
        assert_eq!(pending.partial.len(), 1);

        let completed = recording.finalize().unwrap();
        assert_eq!(
            completed.status,
            CaptureStatus::Complete,
            "{:?}",
            completed.error
        );
        assert_eq!(scan_recordings(&dir).unwrap().len(), 1);
        assert!(dir.join(format!("live-{session}.commit.json")).is_file());
    }

    #[test]
    fn every_commit_fault_leaves_an_explicit_partial_candidate_not_a_complete_session() {
        for point in CommitFaultPoint::ALL {
            let dir = tempfile_dir(&format!("fault-{point:?}"));
            let session = SessionId::new("s-fault").unwrap();
            let mut recording =
                StreamingRecording::create_with_fault(&dir, session, point).unwrap();
            if point == CommitFaultPoint::PeriodicFlush {
                recording.sync_interval_samples = 1;
            }
            let _ = recording.append_pcm16(&[1, 0, 2, 0]);
            let _ = recording.finalize();

            let scanned = scan_recordings(&dir).unwrap();
            assert!(
                scanned.complete.is_empty(),
                "{point:?} published a complete recording"
            );
            assert_eq!(
                scanned.partial.len(),
                1,
                "{point:?} hid the partial recovery candidate"
            );
        }
    }

    #[test]
    fn partial_finalization_publishes_a_hashed_partial_capture_lineage() {
        let dir = tempfile_dir("partial-lineage");
        let session = SessionId::new("s-partial-lineage").unwrap();
        let mut recording = StreamingRecording::create_with_fault(
            &dir,
            session.clone(),
            CommitFaultPoint::AudioSync,
        )
        .unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();

        let result = recording.finalize().unwrap();

        let lineage = result.partial_lineage.expect("partial capture lineage");
        assert_eq!(result.status, CaptureStatus::Partial);
        assert_eq!(
            lineage.capture_sidecar_sha256,
            sha256_file(&dir.join(&lineage.capture_sidecar_file)).unwrap()
        );
        let sidecar: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.join(&lineage.capture_sidecar_file)).unwrap(),
        )
        .unwrap();
        assert_eq!(sidecar["sessionId"], session.as_str());
        assert_eq!(sidecar["status"], "partial");
        let scanned = scan_recordings(&dir).unwrap();
        assert!(scanned.complete.is_empty());
        assert_eq!(scanned.partial.len(), 1);
    }

    #[test]
    fn partial_lineage_uses_the_owned_partial_receipt_not_a_colliding_complete_sidecar() {
        let dir = tempfile_dir("partial-receipt-collision");
        let session = SessionId::new("s-partial-receipt-collision").unwrap();
        let paths = RecordingPaths::new(&dir, session.clone());
        fs::write(&paths.sidecar, b"attacker complete sidecar").unwrap();
        let mut recording = StreamingRecording::create(&dir, session).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();

        let result = recording.finalize().unwrap();

        let lineage = result.partial_lineage.expect("owned partial receipt");
        assert_eq!(
            lineage.capture_sidecar_file,
            paths.partial_sidecar_file_name()
        );
        assert_ne!(
            lineage.capture_sidecar_sha256,
            sha256_file(&paths.sidecar).unwrap()
        );
        assert_eq!(
            fs::read(&paths.sidecar).unwrap(),
            b"attacker complete sidecar"
        );
    }

    #[test]
    fn publication_replacement_barriers_fail_closed_without_deleting_unowned_artifacts() {
        for artifact in [
            PublicationArtifact::CompleteSidecar,
            PublicationArtifact::PartialSidecar,
            PublicationArtifact::Commit,
        ] {
            for barrier in [
                PublicationBarrier::BeforeHardLink,
                PublicationBarrier::AfterHardLink,
            ] {
                let dir = tempfile_dir(&format!("publication-{artifact:?}-{barrier:?}"));
                let session = SessionId::new("s-publication-replacement").unwrap();
                let unowned_path = Arc::new(Mutex::new(None));
                let hook_path = Arc::clone(&unowned_path);
                let mut recording = StreamingRecording::create_with_publication_hook(
                    &dir,
                    session.clone(),
                    if artifact == PublicationArtifact::PartialSidecar {
                        Some(CommitFaultPoint::AudioSync)
                    } else {
                        None
                    },
                    move |published, reached, paths| {
                        if published != artifact || reached != barrier {
                            return;
                        }
                        let target = paths.path_for_publication(artifact, barrier);
                        let displaced = target
                            .with_extension(format!("displaced-{:?}-{:?}", artifact, barrier));
                        fs::rename(&target, &displaced).unwrap();
                        fs::write(&target, b"unowned replacement").unwrap();
                        *hook_path.lock().unwrap() = Some(target);
                    },
                )
                .unwrap();
                recording.append_pcm16(&[1, 0]).unwrap();

                let result = recording.finalize().unwrap();
                let replacement = unowned_path
                    .lock()
                    .unwrap()
                    .clone()
                    .expect("replacement barrier ran");

                assert_eq!(
                    result.status,
                    CaptureStatus::Partial,
                    "{artifact:?} {barrier:?}"
                );
                assert!(result.committed.is_none(), "{artifact:?} {barrier:?}");
                assert_eq!(fs::read(replacement).unwrap(), b"unowned replacement");
                assert!(scan_recordings(&dir).unwrap().complete.is_empty());
            }
        }
    }

    #[test]
    fn worker_caches_the_first_append_failure_while_draining_later_frames() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let dir = tempfile_dir("worker-terminal-append-failure");
        let session = SessionId::new("s-worker-terminal-append-failure").unwrap();
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 64);
        let attempts = Arc::new(AtomicUsize::new(0));
        let journal_attempts = Arc::new(AtomicUsize::new(0));
        let handle = Arc::new(RecordingSinkHandle::spawn_with_fault_for_test(
            dir.clone(),
            session.clone(),
            sink,
            receiver,
            CommitFaultPoint::Append,
            Arc::clone(&attempts),
            journal_attempts,
        ));

        handle.sink().try_send(prepared_frame(&session)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while attempts.load(Ordering::SeqCst) == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "first append was not attempted"
            );
            std::thread::yield_now();
        }
        for _ in 0..1_000 {
            let _ = handle.sink().try_send(prepared_frame(&session));
        }

        let (result_tx, result_rx) = mpsc::channel();
        let finalize_handle = Arc::clone(&handle);
        std::thread::spawn(move || {
            result_tx.send(finalize_handle.finalize()).unwrap();
        });
        let result = result_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("worker must finish after draining the bounded receiver")
            .unwrap();
        let repeated = handle.finalize().unwrap();

        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(result, repeated);
        assert_eq!(
            result.error.as_deref(),
            Some("injected recording fault at Append")
        );
        assert_eq!(result.status, CaptureStatus::Partial);
        assert!(dir.join(format!("live-{session}.wav.part")).is_file());
        assert!(!dir.join(format!("live-{session}.commit.json")).exists());
        assert!(scan_recordings(&dir).unwrap().complete.is_empty());
        assert_eq!(scan_recordings(&dir).unwrap().partial.len(), 1);
    }

    #[test]
    fn journal_persistence_failures_are_terminal_while_the_worker_drains_later_frames() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        for point in [
            CommitFaultPoint::JournalAppend,
            CommitFaultPoint::JournalSync,
        ] {
            let dir = tempfile_dir(&format!("worker-terminal-{point:?}"));
            let session = SessionId::new("s-worker-terminal-journal-failure").unwrap();
            let (sink, receiver) = bounded_sink(SinkKind::Recording, 64);
            let attempts = Arc::new(AtomicUsize::new(0));
            let journal_attempts = Arc::new(AtomicUsize::new(0));
            let handle = Arc::new(RecordingSinkHandle::spawn_with_fault_for_test(
                dir.clone(),
                session.clone(),
                sink,
                receiver,
                point,
                Arc::clone(&attempts),
                Arc::clone(&journal_attempts),
            ));

            handle.sink().try_send(prepared_frame(&session)).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            while attempts.load(Ordering::SeqCst) == 0 {
                assert!(
                    std::time::Instant::now() < deadline,
                    "first append was not attempted for {point:?}"
                );
                std::thread::yield_now();
            }
            for _ in 0..1_000 {
                let _ = handle.sink().try_send(prepared_frame(&session));
            }

            let (result_tx, result_rx) = mpsc::channel();
            let finalize_handle = Arc::clone(&handle);
            std::thread::spawn(move || {
                result_tx.send(finalize_handle.finalize()).unwrap();
            });
            let result = result_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("worker must finish after draining the bounded receiver")
                .unwrap();
            let repeated = handle.finalize().unwrap();

            assert_eq!(attempts.load(Ordering::SeqCst), 1, "{point:?}");
            assert_eq!(journal_attempts.load(Ordering::SeqCst), 1, "{point:?}");
            assert_eq!(result, repeated, "{point:?}");
            assert_eq!(
                result.error,
                Some(format!("injected recording fault at {point:?}")),
                "{point:?}"
            );
            assert_eq!(result.status, CaptureStatus::Partial, "{point:?}");
            assert!(dir.join(format!("live-{session}.wav.part")).is_file());
            assert!(!dir.join(format!("live-{session}.commit.json")).exists());
            assert!(scan_recordings(&dir).unwrap().complete.is_empty());
            assert_eq!(scan_recordings(&dir).unwrap().partial.len(), 1);
        }
    }

    #[test]
    fn rejects_pcm_that_exceeds_wav_u32_data_length_without_wrapping() {
        let dir = tempfile_dir("wav-limit");
        let mut recording =
            StreamingRecording::create(&dir, SessionId::new("s-limit").unwrap()).unwrap();
        recording.set_data_limit_for_test(2);

        assert!(recording.append_pcm16(&[1, 0, 2, 0]).is_err());
        assert_eq!(recording.finalize().unwrap().status, CaptureStatus::Partial);
    }

    #[test]
    fn journal_coalesces_four_hours_of_contiguous_sequence_coverage() {
        let dir = tempfile_dir("journal-bounded");
        let session = SessionId::new("s-journal").unwrap();
        let mut recording = StreamingRecording::create(&dir, session).unwrap();
        for sequence in 0..(4 * 60 * 60 * 10) {
            recording.observe_frame_metadata("live-microphone", 16_000, 1, sequence, 0, 100);
        }

        let journal = recording.journal_for_test();
        assert_eq!(journal.sequence_coverage.len(), 1);
        assert!(journal.serialized_len() < 8_192);
    }

    #[test]
    fn validates_manifest_names_as_same_directory_basename_only() {
        assert!(validate_artifact_name("live-s-a.wav").is_ok());
        for name in [
            "../live-s-a.wav",
            "C:\\live-s-a.wav",
            "/live-s-a.wav",
            "nested/live.wav",
        ] {
            assert!(validate_artifact_name(name).is_err(), "{name}");
        }
    }

    #[test]
    fn finalization_is_idempotent() {
        let dir = tempfile_dir("idempotent");
        let mut recording =
            StreamingRecording::create(&dir, SessionId::new("s-idempotent").unwrap()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let first = recording.finalize().unwrap();
        let second = recording.finalize().unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn sink_handle_finalizes_idempotently_for_concurrent_callers() {
        let dir = tempfile_dir("handle-idempotent");
        let session = SessionId::new("s-handle").unwrap();
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
        let handle = Arc::new(RecordingSinkHandle::spawn(
            dir,
            session.clone(),
            sink,
            receiver,
        ));
        handle.sink().try_send(prepared_frame(&session)).unwrap();

        let left_handle = Arc::clone(&handle);
        let left = std::thread::spawn(move || left_handle.finalize().unwrap());
        let right_handle = Arc::clone(&handle);
        let right = std::thread::spawn(move || right_handle.finalize().unwrap());

        let left = left.join().unwrap();
        let right = right.join().unwrap();
        assert_eq!(left, right);
        assert_eq!(left.status, CaptureStatus::Complete);
    }

    #[test]
    fn sink_handle_caches_worker_panic_for_racing_and_repeated_callers() {
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
        let handle = Arc::new(RecordingSinkHandle::spawn_panicking_for_test(
            sink,
            receiver,
            SessionId::new("s-panicking-recording").unwrap(),
        ));
        let barrier = Arc::new(std::sync::Barrier::new(3));

        let left_handle = Arc::clone(&handle);
        let left_barrier = Arc::clone(&barrier);
        let left = std::thread::spawn(move || {
            left_barrier.wait();
            left_handle.finalize()
        });
        let right_handle = Arc::clone(&handle);
        let right_barrier = Arc::clone(&barrier);
        let right = std::thread::spawn(move || {
            right_barrier.wait();
            right_handle.finalize()
        });

        barrier.wait();
        let left = left.join().unwrap();
        let right = right.join().unwrap();
        let repeated = handle.finalize();

        assert_eq!(left, right);
        assert_eq!(left, repeated);
        assert_eq!(
            left.unwrap_err(),
            "recording worker panicked during finalization"
        );
    }

    #[test]
    fn journal_create_failure_leaves_wav_part_as_a_recovery_candidate() {
        let dir = tempfile_dir("journal-create-failure");
        let session = SessionId::new("s-journal-create-failure").unwrap();
        std::fs::write(
            dir.join(format!("live-{session}.capture.journal.part")),
            "occupied",
        )
        .unwrap();

        assert!(StreamingRecording::create(&dir, session.clone()).is_err());

        let scan = scan_recordings(&dir).unwrap();
        assert!(scan.complete.is_empty());
        assert_eq!(scan.partial.len(), 1);
        assert_eq!(scan.partial[0].session_id.as_ref(), Some(&session));
    }

    #[test]
    fn journal_append_keeps_stale_unowned_files_untouched() {
        let dir = tempfile_dir("journal-private-temp");
        let session = SessionId::new("s-journal-private-temp").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        let stale = dir.join(format!("live-{session}.capture.journal.part.next"));
        std::fs::write(&stale, b"stale private snapshot").unwrap();

        recording.persist_journal_for_test().unwrap();

        assert_eq!(std::fs::read(&stale).unwrap(), b"stale private snapshot");
        drop(recording);
        let scan = scan_recordings(&dir).unwrap();
        assert!(scan.complete.is_empty());
        assert_eq!(scan.partial.len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn persistent_allocation_survives_runtime_restarts_without_reusing_numeric_names() {
        let dir = tempfile_dir("persistent-allocation-restart");
        let first = allocate_recording_session(&dir).unwrap();
        let mut first_recording = StreamingRecording::create_reserved(&dir, first.clone()).unwrap();
        first_recording.append_pcm16(&[1, 0]).unwrap();
        first_recording.finalize().unwrap();

        let second = allocate_recording_session(&dir).unwrap();

        assert_ne!(first, second);
        assert!(dir.join(format!("live-{first}.commit.json")).is_file());
        assert!(dir.join(format!("live-{second}.wav.part")).is_file());
    }

    #[test]
    fn reservation_rejects_every_preexisting_artifact_for_the_same_session() {
        let session = SessionId::new("s-existing-artifact").unwrap();
        for suffix in [
            ".wav",
            ".txt",
            ".capture.json",
            ".capture.partial.json",
            ".commit.json",
            ".transcript.r1.json",
            ".capture.journal.part",
        ] {
            let dir = tempfile_dir(&format!("preexisting-{}", suffix.replace('.', "-")));
            std::fs::write(dir.join(format!("live-{session}{suffix}")), b"existing").unwrap();

            let error = reserve_wav_part(&dir, &session).unwrap_err();

            assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists, "{suffix}");
            assert!(!dir.join(format!("live-{session}.wav.part")).exists());
        }
    }

    #[test]
    fn reservation_collision_after_preflight_never_removes_the_competing_claim() {
        let dir = tempfile_dir("reservation-race");
        let session = SessionId::new("s-reservation-race").unwrap();
        let claimed = RecordingPaths::new(&dir, session.clone()).wav_part;
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let (claimed_tx, claimed_rx) = std::sync::mpsc::channel();
        let racer_dir = dir.clone();
        let racer_session = session.clone();
        let racer_barrier = Arc::clone(&barrier);
        let racer = std::thread::spawn(move || {
            racer_barrier.wait();
            std::fs::write(
                RecordingPaths::new(&racer_dir, racer_session).wav_part,
                b"racer reservation",
            )
            .unwrap();
            claimed_tx.send(()).unwrap();
        });

        let error = reserve_wav_part_with_before_claim(&dir, &session, || {
            barrier.wait();
            claimed_rx.recv().unwrap();
        })
        .unwrap_err();
        racer.join().unwrap();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(claimed).unwrap(), b"racer reservation");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn concurrent_persistent_allocations_reserve_distinct_wav_parts() {
        let dir = tempfile_dir("concurrent-persistent-allocation");
        let barrier = Arc::new(std::sync::Barrier::new(5));
        let mut workers = Vec::new();
        for _ in 0..4 {
            let directory = dir.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                allocate_recording_session(&directory).unwrap()
            }));
        }
        barrier.wait();
        let sessions = workers
            .into_iter()
            .map(|worker| worker.join().unwrap().to_string())
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(sessions.len(), 4);
        for session in sessions {
            assert!(dir.join(format!("live-{session}.wav.part")).is_file());
        }
    }

    #[test]
    fn collision_safe_publication_never_overwrites_an_existing_artifact() {
        let dir = tempfile_dir("no-overwrite-publication");
        for suffix in [
            ".wav",
            ".capture.json",
            ".capture.partial.json",
            ".commit.json",
            ".txt",
            ".transcript.r1.json",
        ] {
            let source = dir.join(format!("staged{suffix}.part"));
            let destination = dir.join(format!("live-s-safe{suffix}"));
            std::fs::write(&source, b"new").unwrap();
            std::fs::write(&destination, b"old").unwrap();
            let owned = File::open(&source).unwrap();

            assert!(publish_no_replace(&source, &destination, &owned, "test publish").is_err());
            assert_eq!(std::fs::read(&destination).unwrap(), b"old");
            assert_eq!(std::fs::read(&source).unwrap(), b"new");
        }
    }

    #[test]
    fn hard_link_cleanup_debt_keeps_publication_complete_and_staging_private() {
        for (fault, staging_suffix) in [
            (CommitFaultPoint::AudioStagingCleanup, ".wav.part"),
            (CommitFaultPoint::CommitStagingCleanup, ".commit.json.part"),
        ] {
            let dir = tempfile_dir(&format!("post-link-cleanup-{fault:?}"));
            let session = SessionId::new("s-post-link-cleanup").unwrap();
            let mut recording =
                StreamingRecording::create_with_fault(&dir, session.clone(), fault).unwrap();
            recording.append_pcm16(&[1, 0]).unwrap();

            let result = recording.finalize().unwrap();
            assert_eq!(result.status, CaptureStatus::Complete, "{fault:?}");
            assert!(result.committed.is_some(), "{fault:?}");
            let scan = scan_recordings(&dir).unwrap();
            assert_eq!(scan.complete.len(), 1, "{fault:?}");
            assert!(scan.partial.is_empty(), "{fault:?}");
            let staging = dir.join(format!("live-{session}{staging_suffix}"));
            assert!(staging.is_file(), "{fault:?}");
            let owned = File::open(&staging).unwrap();
            assert!(
                publish_no_replace(
                    &staging,
                    &dir.join(format!("live-{session}.commit.json")),
                    &owned,
                    "retry"
                )
                .is_err(),
                "{fault:?} must never overwrite the published destination"
            );
            std::fs::remove_dir_all(dir).ok();
        }
    }

    #[test]
    fn lone_wav_part_is_a_partial_candidate_without_inventing_metadata() {
        let dir = tempfile_dir("lone-wav-part");
        let session = SessionId::new("s-lone-wav-part").unwrap();
        std::fs::write(dir.join(format!("live-{session}.wav.part")), b"RIFF").unwrap();

        let scan = scan_recordings(&dir).unwrap();

        assert!(scan.complete.is_empty());
        assert_eq!(scan.partial.len(), 1);
        assert_eq!(scan.partial[0].session_id.as_ref(), Some(&session));
        assert_eq!(scan.partial[0].directory, dir);
    }

    #[test]
    fn scanner_ignores_malformed_or_unknown_partial_artifacts() {
        let dir = tempfile_dir("malformed-partials");
        for name in [
            "live-.wav.part",
            "live-not a session.wav.part",
            "live-s-known.wav.partial",
            "capture.wav.part",
        ] {
            std::fs::write(dir.join(name), b"partial").unwrap();
        }

        assert!(scan_recordings(&dir).unwrap().is_empty());
    }

    #[test]
    fn alternating_four_hour_gaps_have_bounded_journal_memory_and_snapshot_size() {
        const FOUR_HOURS_AT_TEN_HZ: u64 = 4 * 60 * 60 * 10;
        const MAX_JOURNAL_BYTES: u64 = 128 * 1024;

        let dir = tempfile_dir("journal-alternating-gaps");
        let session = SessionId::new("s-journal-alternating-gaps").unwrap();
        let mut recording = StreamingRecording::create(&dir, session).unwrap();
        for sequence in (0..FOUR_HOURS_AT_TEN_HZ * 2).step_by(2) {
            recording.observe_frame_metadata("live-microphone", 16_000, 1, sequence, 0, 100);
            if sequence % 20_000 == 0 {
                recording.persist_journal_for_test().unwrap();
            }
        }
        recording.persist_journal_for_test().unwrap();

        assert_eq!(
            recording.journal_for_test().sequence_gaps.len(),
            MAX_SEQUENCE_GAP_DETAILS
        );
        assert!(recording.journal_for_test().sequence_gap_overflow.is_some());
        assert!(
            std::fs::metadata(&recording.paths.journal_part)
                .unwrap()
                .len()
                <= MAX_JOURNAL_BYTES
        );
        let recovered = read_journal_snapshot(&recording.paths.journal_part).unwrap();
        assert!(!recovered.sequence_coverage.is_empty());
        assert!(
            recovered.sequence_coverage[0].last_sequence <= FOUR_HOURS_AT_TEN_HZ * 2 - 2,
            "a bounded append journal may retain a valid prefix once it reaches its terminal marker"
        );
    }

    #[test]
    fn journal_recovers_the_valid_append_prefix_after_a_torn_tail() {
        let dir = tempfile_dir("journal-torn-tail");
        let session = SessionId::new("s-journal-torn-tail").unwrap();
        let mut recording = StreamingRecording::create(&dir, session).unwrap();
        recording.observe_frame_metadata("live-microphone", 16_000, 1, 4, 0, 100);
        recording.persist_journal_for_test().unwrap();
        let journal = recording.paths.journal_part.clone();
        drop(recording);
        OpenOptions::new()
            .append(true)
            .open(&journal)
            .unwrap()
            .write_all(b"{\"delta\":")
            .unwrap();

        let recovered = read_journal_snapshot(&journal).unwrap();

        assert_eq!(recovered.sequence_coverage[0].last_sequence, 4);
    }

    #[test]
    fn journal_never_replaces_an_adversarial_path_after_creation() {
        let dir = tempfile_dir("journal-path-replacement");
        let session = SessionId::new("s-journal-path-replacement").unwrap();
        let mut recording = StreamingRecording::create(&dir, session).unwrap();
        let journal = recording.paths.journal_part.clone();
        let displaced = dir.join("displaced-journal");
        fs::rename(&journal, &displaced).unwrap();
        fs::write(&journal, b"attacker replacement").unwrap();
        recording.observe_frame_metadata("live-microphone", 16_000, 1, 1, 0, 100);

        recording.persist_journal_for_test().unwrap();

        assert_eq!(fs::read(&journal).unwrap(), b"attacker replacement");
        assert!(fs::metadata(&displaced).unwrap().len() > 0);
    }

    #[test]
    fn four_hour_journal_growth_stops_at_the_explicit_hard_limit() {
        let dir = tempfile_dir("journal-hard-limit");
        let session = SessionId::new("s-journal-hard-limit").unwrap();
        let mut recording = StreamingRecording::create(&dir, session).unwrap();
        for second in 0..(4 * 60 * 60) {
            recording.observe_frame_metadata(
                "live-microphone",
                16_000,
                1,
                second * 2,
                second * 1_000,
                100,
            );
            recording.persist_journal_for_test().unwrap();
        }

        let journal = recording.paths.journal_part.clone();
        let bounded = fs::metadata(&journal).unwrap().len();
        recording.persist_journal_for_test().unwrap();

        assert!(bounded <= MAX_JOURNAL_BYTES);
        assert_eq!(fs::metadata(journal).unwrap().len(), bounded);
        assert!(recording.journal_growth_stopped_for_test());
    }

    #[test]
    fn repeated_journal_write_failure_returns_the_cached_terminal_error() {
        let dir = tempfile_dir("journal-write-failure");
        let session = SessionId::new("s-journal-write-failure").unwrap();
        let mut recording =
            StreamingRecording::create_with_fault(&dir, session, CommitFaultPoint::JournalAppend)
                .unwrap();
        let journal = recording.paths.journal_part.clone();
        let initial_len = fs::metadata(&journal).unwrap().len();

        let first = recording.persist_journal_for_test().unwrap_err();
        let second = recording.persist_journal_for_test().unwrap_err();

        assert_eq!(first, second);
        assert_eq!(fs::metadata(journal).unwrap().len(), initial_len);
    }

    #[test]
    fn sidecar_replacement_between_receipt_and_commit_fails_closed() {
        let dir = tempfile_dir("partial-receipt-reparse-replacement");
        let session = SessionId::new("s-partial-receipt-reparse-replacement").unwrap();
        let outside = dir.join("outside-sidecar.json");
        fs::write(&outside, b"attacker sidecar").unwrap();
        let mut recording =
            StreamingRecording::create_with_sidecar_hook(&dir, session.clone(), move |paths| {
                if let Err(error) = fs::remove_file(&paths.sidecar) {
                    panic!("failed to remove owned sidecar in test: {error}");
                }
                if let Err(error) = create_file_symlink_for_test(&outside, &paths.sidecar) {
                    if error.kind() == std::io::ErrorKind::PermissionDenied
                        || error.raw_os_error() == Some(1314)
                    {
                        fs::write(&paths.sidecar, b"attacker replacement").unwrap();
                        return;
                    }
                    panic!("failed to replace sidecar with reparse point: {error}");
                }
            })
            .unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();

        let result = recording.finalize().unwrap();
        let lineage = result
            .partial_lineage
            .expect("replacement-safe partial lineage");

        assert_eq!(result.status, CaptureStatus::Partial);
        assert!(result.committed.is_none());
        assert_eq!(
            lineage.capture_sidecar_file,
            format!("live-{session}.capture.partial.json")
        );
        assert!(
            !dir.join(format!("live-{session}.commit.json")).exists(),
            "a divergent sidecar must never receive a complete commit"
        );
        assert!(scan_recordings(&dir).unwrap().complete.is_empty());
    }

    #[test]
    fn sidecar_replacement_during_commit_publication_fails_closed() {
        for barrier in [
            PublicationBarrier::BeforeHardLink,
            PublicationBarrier::AfterHardLink,
        ] {
            let dir = tempfile_dir(&format!("sidecar-commit-{barrier:?}"));
            let session = SessionId::new("s-sidecar-commit-replacement").unwrap();
            let mut recording = StreamingRecording::create_with_publication_hook(
                &dir,
                session.clone(),
                None,
                move |artifact, reached, paths| {
                    if artifact != PublicationArtifact::Commit || reached != barrier {
                        return;
                    }
                    let displaced = paths
                        .sidecar
                        .with_extension(format!("{barrier:?}.displaced"));
                    fs::rename(&paths.sidecar, displaced).unwrap();
                    fs::write(&paths.sidecar, b"attacker sidecar").unwrap();
                },
            )
            .unwrap();
            recording.append_pcm16(&[1, 0]).unwrap();

            let result = recording.finalize().unwrap();

            assert_eq!(result.status, CaptureStatus::Partial, "{barrier:?}");
            assert!(result.committed.is_none(), "{barrier:?}");
            assert!(
                scan_recordings(&dir).unwrap().complete.is_empty(),
                "{barrier:?}"
            );
        }
    }

    #[test]
    fn nofollow_handle_keeps_the_original_bytes_across_path_replacement() {
        let dir = tempfile_dir("nofollow-replacement");
        let name = "safe.json";
        let path = dir.join(name);
        fs::write(&path, b"owned bytes").unwrap();
        let mut file = open_regular_artifact(&dir, name).unwrap();
        let displaced = dir.join("displaced-safe.json");
        fs::rename(&path, &displaced).unwrap();
        fs::write(&path, b"attacker bytes").unwrap();
        let mut bytes = String::new();

        file.read_to_string(&mut bytes).unwrap();

        assert_eq!(bytes, "owned bytes");
    }

    #[test]
    fn aborted_capture_open_is_partial_but_successful_zero_duration_is_complete() {
        let failed_dir = tempfile_dir("capture-open-failed");
        let failed_session = SessionId::new("s-capture-open-failed").unwrap();
        let (failed_sink, failed_rx) = bounded_sink(SinkKind::Recording, 1);
        let failed = RecordingSinkHandle::spawn(
            failed_dir.clone(),
            failed_session.clone(),
            failed_sink,
            failed_rx,
        );

        let failed_result = failed.abort("capture adapter could not open").unwrap();

        assert_eq!(failed_result.status, CaptureStatus::Partial);
        assert!(failed_result.committed.is_none());
        assert_eq!(scan_recordings(&failed_dir).unwrap().complete.len(), 0);
        assert_eq!(scan_recordings(&failed_dir).unwrap().partial.len(), 1);

        let complete_dir = tempfile_dir("capture-zero-duration");
        let complete_session = SessionId::new("s-capture-zero-duration").unwrap();
        let (complete_sink, complete_rx) = bounded_sink(SinkKind::Recording, 1);
        let complete = RecordingSinkHandle::spawn(
            complete_dir.clone(),
            complete_session,
            complete_sink,
            complete_rx,
        );

        assert_eq!(complete.finalize().unwrap().status, CaptureStatus::Complete);
        assert_eq!(scan_recordings(&complete_dir).unwrap().complete.len(), 1);
    }

    #[test]
    fn scan_rejects_symlinked_committed_artifacts_when_links_are_supported() {
        let dir = tempfile_dir("symlinked-artifact");
        let session = SessionId::new("s-symlinked-artifact").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap();

        let audio = dir.join(format!("live-{session}.wav"));
        let outside = std::env::temp_dir().join(format!(
            "yap-recording-symlink-target-{}-{session}.wav",
            std::process::id()
        ));
        std::fs::remove_file(&outside).ok();
        std::fs::rename(&audio, &outside).unwrap();
        if let Err(error) = create_file_symlink_for_test(&outside, &audio) {
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(1314)
            {
                std::fs::rename(&outside, &audio).unwrap();
                return;
            }
            panic!("failed to create symlink: {error}");
        }

        let scan = scan_recordings(&dir).unwrap();
        assert!(scan.complete.is_empty());
        assert_eq!(scan.partial.len(), 1);
        std::fs::remove_file(&audio).ok();
        std::fs::remove_file(&outside).ok();
    }

    #[cfg(unix)]
    fn create_file_symlink_for_test(original: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(original, link)
    }

    #[cfg(windows)]
    fn create_file_symlink_for_test(original: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(original, link)
    }

    fn prepared_frame(session_id: &SessionId) -> PreparedFrame {
        PreparedFrame {
            metadata: AudioFrame {
                session_id: session_id.clone(),
                track_id: TrackId::new("live-microphone").unwrap(),
                sequence: 0,
                sample_rate_hz: 16_000,
                channels: 1,
                start_ms: 0,
                duration_ms: 1,
                sample_count: 1,
            },
            samples: Arc::from([0.25]),
        }
    }

    fn tempfile_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("yap-recording-{label}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
