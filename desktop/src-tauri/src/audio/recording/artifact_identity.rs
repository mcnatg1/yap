use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PublicationArtifact {
    Audio,
    CompleteSidecar,
    PartialSidecar,
    Commit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PublicationBarrier {
    BeforeHardLink,
    AfterHardLink,
}

#[derive(Debug, Clone)]
pub(super) struct PublicationReceipt {
    pub(super) file_name: String,
    pub(super) sha256: String,
    pub(super) status: CaptureStatus,
    pub(super) path: PathBuf,
    pub(super) identity: FileIdentity,
}

impl PublicationReceipt {
    pub(super) fn lineage(&self) -> PartialCaptureLineage {
        match self.status {
            CaptureStatus::Complete | CaptureStatus::Partial => {}
        }
        PartialCaptureLineage {
            capture_sidecar_file: self.file_name.clone(),
            capture_sidecar_sha256: self.sha256.clone(),
        }
    }

    pub(super) fn revalidate(&self) -> Result<(), String> {
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
    pub(super) device: u64,
    pub(super) inode: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg(windows)]
pub(crate) struct FileIdentity {
    pub(super) volume_serial: u32,
    pub(super) file_index: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg(not(any(unix, windows)))]
pub(crate) struct FileIdentity;

#[derive(Debug)]
pub(crate) struct RegularArtifactIdentity {
    pub(super) path: PathBuf,
    pub(super) identity: FileIdentity,
    pub(super) require_single_link: bool,
}

impl RegularArtifactIdentity {
    pub(crate) fn matches_artifact_name(&self, name: &str) -> bool {
        self.path.file_name().and_then(|value| value.to_str()) == Some(name)
    }

    pub(super) fn open_current(&self) -> Result<File, String> {
        self.open_current_at(&self.path)
    }

    pub(super) fn open_current_at(&self, path: &Path) -> Result<File, String> {
        let current = open_regular_path(path)?;
        if file_identity(&current)? != self.identity {
            return Err("recording artifact path no longer names the admitted file".into());
        }
        self.ensure_link_ownership(&current)?;
        Ok(current)
    }

    pub(super) fn ensure_open_file(&self, file: &File) -> Result<(), String> {
        if file_identity(file)? != self.identity {
            return Err("recording artifact path no longer names the admitted file".into());
        }
        self.ensure_link_ownership(file)?;
        Ok(())
    }

    pub(super) fn ensure_link_ownership(&self, file: &File) -> Result<(), String> {
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
    pub(super) path: PathBuf,
    pub(super) sha256: String,
    pub(super) identity: FileIdentity,
}
