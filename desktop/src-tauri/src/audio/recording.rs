use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Condvar, Mutex};
use std::thread::JoinHandle;

use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::audio::coordinator::{BoundedReceiver, BoundedSink};
use crate::audio::frame::PreparedFrame;
use crate::audio::preprocess::f32_to_i16_le_bytes;
use crate::audio::session::SessionId;

const CAPTURE_SCHEMA_VERSION: u16 = 1;
const WAV_HEADER_BYTES: u64 = 44;
const PCM16_BYTES_PER_SAMPLE: u64 = 2;
const DEFAULT_SYNC_INTERVAL_SAMPLES: u64 = 16_000;

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
pub struct RecordingFinalizeResult {
    pub session_id: SessionId,
    pub status: CaptureStatus,
    pub committed: Option<CommittedCapture>,
    pub error: Option<String>,
}

pub struct RecordingSinkHandle {
    sink: BoundedSink<PreparedFrame>,
    state: Mutex<RecordingSinkState>,
    completed: Condvar,
}

struct RecordingSinkState {
    worker: Option<JoinHandle<RecordingFinalizeResult>>,
    result: Option<RecordingFinalizeResult>,
    finalizing: bool,
}

impl RecordingSinkHandle {
    pub fn spawn(
        directory: PathBuf,
        session_id: SessionId,
        sink: BoundedSink<PreparedFrame>,
        receiver: BoundedReceiver<PreparedFrame>,
    ) -> Self {
        let worker_session_id = session_id.clone();
        let worker = std::thread::spawn(move || {
            run_recording_worker(directory, worker_session_id, receiver)
        });
        Self {
            sink,
            state: Mutex::new(RecordingSinkState {
                worker: Some(worker),
                result: None,
                finalizing: false,
            }),
            completed: Condvar::new(),
        }
    }

    pub fn sink(&self) -> BoundedSink<PreparedFrame> {
        self.sink.clone()
    }

    pub fn finalize(&self) -> Result<RecordingFinalizeResult, String> {
        self.sink.close();
        let worker = loop {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "recording handle became unavailable")?;
            if let Some(result) = &state.result {
                return Ok(result.clone());
            }
            if !state.finalizing {
                state.finalizing = true;
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
            .map_err(|_| "recording handle became unavailable")?;
        state.finalizing = false;
        if let Ok(result) = &result {
            state.result = Some(result.clone());
        }
        self.completed.notify_all();
        result
    }
}

fn run_recording_worker(
    directory: PathBuf,
    session_id: SessionId,
    receiver: BoundedReceiver<PreparedFrame>,
) -> RecordingFinalizeResult {
    let mut recording = match StreamingRecording::create(&directory, session_id.clone()) {
        Ok(recording) => recording,
        Err(error) => {
            return RecordingFinalizeResult {
                session_id,
                status: CaptureStatus::Partial,
                committed: None,
                error: Some(error),
            };
        }
    };
    loop {
        match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(frame) => {
                let _ = recording.append_prepared(&frame);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    recording
        .finalize()
        .unwrap_or_else(|error| RecordingFinalizeResult {
            session_id,
            status: CaptureStatus::Partial,
            committed: None,
            error: Some(error),
        })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    sink_degraded: bool,
    directory_sync_supported: bool,
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalTrack {
    track_id: String,
    sample_rate_hz: u32,
    channels: u16,
    first_start_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalClockMapping {
    track_id: String,
    sequence: u64,
    session_time_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SequenceCoverage {
    track_id: String,
    first_sequence: u64,
    last_sequence: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SequenceGap {
    track_id: String,
    first_sequence: u64,
    dropped_frames: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureJournal {
    schema_version: u16,
    session_id: SessionId,
    tracks: BTreeMap<String, JournalTrack>,
    clock_mappings: Vec<JournalClockMapping>,
    sequence_coverage: Vec<SequenceCoverage>,
    sequence_gaps: Vec<SequenceGap>,
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
        self.sequence_gaps.push(SequenceGap {
            track_id: track_id.to_string(),
            first_sequence,
            dropped_frames,
        });
    }

    #[cfg(test)]
    fn serialized_len(&self) -> usize {
        serde_json::to_vec(self).map_or(usize::MAX, |value| value.len())
    }
}

pub struct StreamingRecording {
    paths: RecordingPaths,
    audio: Option<File>,
    journal_file: Option<File>,
    journal: CaptureJournal,
    data_bytes: u64,
    samples_since_sync: u64,
    sync_interval_samples: u64,
    data_limit: u64,
    failure: Option<String>,
    finalized: Option<RecordingFinalizeResult>,
    #[cfg(test)]
    fault: Option<CommitFaultPoint>,
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
}

impl StreamingRecording {
    pub fn create(directory: &Path, session_id: SessionId) -> Result<Self, String> {
        Self::create_inner(directory, session_id, None)
    }

    #[cfg(test)]
    fn create_with_fault(
        directory: &Path,
        session_id: SessionId,
        fault: CommitFaultPoint,
    ) -> Result<Self, String> {
        Self::create_inner(directory, session_id, Some(fault))
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
        let mut audio = create_new(&paths.wav_part, "recording audio")?;
        write_wav_header(&mut audio, 0)?;
        audio
            .sync_data()
            .map_err(|error| format!("Failed to initialize live audio: {error}"))?;
        let mut journal_file = create_new(&paths.journal_part, "recording journal")?;
        let journal = CaptureJournal::new(session_id);
        write_journal_snapshot(&mut journal_file, &journal)?;
        journal_file
            .sync_data()
            .map_err(|error| format!("Failed to initialize recording journal: {error}"))?;
        Ok(Self {
            paths,
            audio: Some(audio),
            journal_file: Some(journal_file),
            journal,
            data_bytes: 0,
            samples_since_sync: 0,
            sync_interval_samples: DEFAULT_SYNC_INTERVAL_SAMPLES,
            data_limit: u64::from(u32::MAX),
            failure: None,
            finalized: None,
            #[cfg(test)]
            fault,
        })
    }

    pub fn append_prepared(&mut self, frame: &PreparedFrame) -> Result<(), String> {
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
            if let Some(journal_file) = self.journal_file.as_mut() {
                write_journal_snapshot(journal_file, &self.journal)?;
                journal_file
                    .sync_data()
                    .map_err(|error| format!("Failed to flush recording journal: {error}"))?;
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

        if let Err(error) = self.finalize_inner() {
            self.failure = Some(error);
            return Ok(self.partial_result());
        }
        let result = RecordingFinalizeResult {
            session_id: self.paths.session_id.clone(),
            status: CaptureStatus::Complete,
            committed: Some(CommittedCapture {
                manifest: read_manifest(&self.paths.commit)?,
                directory: self.paths.directory.clone(),
            }),
            error: None,
        };
        self.finalized = Some(result.clone());
        Ok(result)
    }

    fn finalize_inner(&mut self) -> Result<(), String> {
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
        drop(audio);

        if let Some(mut journal_file) = self.journal_file.take() {
            write_journal_snapshot(&mut journal_file, &self.journal)?;
            journal_file
                .sync_all()
                .map_err(|error| format!("Failed to sync recording journal: {error}"))?;
        }

        self.hit_fault(CommitFaultPoint::FinalArtifactRename)?;
        fs::rename(&self.paths.wav_part, &self.paths.wav)
            .map_err(|error| format!("Failed to finalize live audio: {error}"))?;

        let audio_sha256 = sha256_file(&self.paths.wav)?;
        let audio_bytes = fs::metadata(&self.paths.wav)
            .map_err(|error| format!("Failed to inspect finalized live audio: {error}"))?
            .len();
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
            sink_degraded: self.journal.sink_degraded,
            directory_sync_supported: sync_parent_directory(&self.paths.directory),
        };
        write_json_file(&self.paths.sidecar_part, &sidecar, "capture sidecar")?;
        self.hit_fault(CommitFaultPoint::SidecarSync)?;
        sync_file(&self.paths.sidecar_part, "capture sidecar")?;
        fs::rename(&self.paths.sidecar_part, &self.paths.sidecar)
            .map_err(|error| format!("Failed to publish capture sidecar: {error}"))?;
        let sidecar_sha256 = sha256_file(&self.paths.sidecar)?;

        let manifest = CaptureCommitManifest {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id: self.paths.session_id.clone(),
            status: CaptureStatus::Complete,
            audio_file: self.paths.wav_file_name(),
            audio_sha256: sidecar.audio_sha256,
            audio_bytes,
            capture_sidecar_file: self.paths.sidecar_file_name(),
            capture_sidecar_sha256: sidecar_sha256,
            committed_at_utc: now_utc()?,
        };
        manifest.validate()?;
        write_json_file(&self.paths.commit_part, &manifest, "capture commit")?;
        self.hit_fault(CommitFaultPoint::CommitSync)?;
        sync_file(&self.paths.commit_part, "capture commit")?;
        self.hit_fault(CommitFaultPoint::CommitRename)?;
        fs::rename(&self.paths.commit_part, &self.paths.commit)
            .map_err(|error| format!("Failed to publish capture commit: {error}"))?;
        let _ = sync_parent_directory(&self.paths.directory);
        fs::remove_file(&self.paths.journal_part).ok();
        Ok(())
    }

    fn partial_result(&mut self) -> RecordingFinalizeResult {
        let result = RecordingFinalizeResult {
            session_id: self.paths.session_id.clone(),
            status: CaptureStatus::Partial,
            committed: None,
            error: self.failure.clone(),
        };
        self.finalized = Some(result.clone());
        result
    }

    fn fail<T>(&mut self, error: String) -> Result<T, String> {
        self.failure = Some(error.clone());
        Err(error)
    }

    fn hit_fault(&self, point: CommitFaultPoint) -> Result<(), String> {
        #[cfg(test)]
        if self.fault == Some(point) {
            return Err(format!("injected recording fault at {point:?}"));
        }
        let _ = point;
        Ok(())
    }

    #[cfg(test)]
    fn set_data_limit_for_test(&mut self, data_limit: u64) {
        self.data_limit = data_limit;
    }

    #[cfg(test)]
    fn journal_for_test(&self) -> &CaptureJournal {
        &self.journal
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

fn write_journal_snapshot(file: &mut File, journal: &CaptureJournal) -> Result<(), String> {
    serde_json::to_writer(&mut *file, journal)
        .map_err(|error| format!("Failed to write recording journal: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("Failed to write recording journal: {error}"))
}

fn write_json_file<T: serde::Serialize>(path: &Path, value: &T, label: &str) -> Result<(), String> {
    let mut file = create_new(path, label)?;
    serde_json::to_writer(&mut file, value)
        .map_err(|error| format!("Failed to write {label}: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("Failed to write {label}: {error}"))
}

fn sync_file(path: &Path, label: &str) -> Result<(), String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("Failed to sync {label}: {error}"))
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("Failed to hash recording artifact: {error}"))?;
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

fn read_manifest(path: &Path) -> Result<CaptureCommitManifest, String> {
    let text = fs::read_to_string(path)
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
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if let Some(session) = session_from_private_artifact(name) {
            partial_ids.insert(session.as_str().to_string());
            continue;
        }
        if !name.starts_with("live-") || !name.ends_with(".commit.json") {
            continue;
        }
        match read_manifest(&path)
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
    let audio = resolve_artifact(directory, &manifest.audio_file)?;
    let sidecar = resolve_artifact(directory, &manifest.capture_sidecar_file)?;
    if fs::metadata(&audio)
        .map_err(|error| format!("Failed to inspect committed audio: {error}"))?
        .len()
        != manifest.audio_bytes
    {
        return Err("committed audio size does not match the manifest".into());
    }
    if sha256_file(&audio)? != manifest.audio_sha256
        || sha256_file(&sidecar)? != manifest.capture_sidecar_sha256
    {
        return Err("committed recording artifact hash does not match the manifest".into());
    }
    let sidecar_text = fs::read_to_string(&sidecar)
        .map_err(|error| format!("Failed to read capture sidecar: {error}"))?;
    let sidecar: CaptureSidecar = serde_json::from_str(&sidecar_text)
        .map_err(|error| format!("Failed to parse capture sidecar: {error}"))?;
    sidecar.validate(&manifest)?;
    Ok(CommittedCapture {
        manifest,
        directory: directory.to_path_buf(),
    })
}

fn resolve_artifact(directory: &Path, name: &str) -> Result<PathBuf, String> {
    validate_artifact_name(name)?;
    Ok(directory.join(name))
}

fn session_from_private_artifact(name: &str) -> Option<SessionId> {
    let session = name
        .strip_prefix("live-")?
        .strip_suffix(".capture.journal.part")?;
    SessionId::new(session).ok()
}

fn session_from_commit_name(name: &str) -> Option<SessionId> {
    let session = name.strip_prefix("live-")?.strip_suffix(".commit.json")?;
    SessionId::new(session).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitFaultPoint {
    Append,
    PeriodicFlush,
    WavHeaderPatch,
    AudioSync,
    SidecarSync,
    FinalArtifactRename,
    CommitSync,
    CommitRename,
}

impl CommitFaultPoint {
    #[cfg(test)]
    const ALL: [Self; 8] = [
        Self::Append,
        Self::PeriodicFlush,
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
