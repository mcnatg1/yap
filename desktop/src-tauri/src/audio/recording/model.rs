use super::*;

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
    pub(super) fn validate(&self) -> Result<(), String> {
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
    pub(super) sidecar_receipt: Option<PublicationReceipt>,
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
