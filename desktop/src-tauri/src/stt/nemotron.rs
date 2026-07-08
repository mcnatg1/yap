use std::path::{Path, PathBuf};

use crate::stt::error::SttError;

pub const MODEL_ID: &str = "nemotron-3.5-asr-streaming-0.6b-1120ms-int8";
pub const MODEL_LABEL: &str = "Nemotron 3.5 ASR Streaming 0.6B INT8";
pub const CHUNK_MS: u64 = 1120;
pub const NUM_THREADS: i32 = 4;

const MODEL_DIR: &str = "nemotron-3.5-asr-streaming-0.6b-1120ms-int8";
const REPO: &str = "csukuangfj2/sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-1120ms-int8-2026-06-11";
const REVISION: &str = "d2f58fb3c1ae44829133de74c1b5aa6e3e6dda04";
const FALLBACK_MODEL_LABEL: &str = "Nemotron local fallback";

const ARTIFACTS: &[Artifact] = &[
    Artifact {
        file: "encoder.int8.onnx",
        sha256: "2fff2166acaa535bd969fb223c1f0783d71029f143cb298bc54c2afe85abf772",
    },
    Artifact {
        file: "decoder.int8.onnx",
        sha256: "19f9c98fc6d0a2c33a65a43b36fdb2e914c26c0aa9764be3aebc502a1e982fb0",
    },
    Artifact {
        file: "joiner.int8.onnx",
        sha256: "4101c7c679a0bc30483794b27a059e34e79232aa2068d78d51231a22c8b0d7ce",
    },
    Artifact {
        file: "tokens.txt",
        sha256: "729cc103155bafa785f9cd45746cd41cabe97eab7182fc04d594129587958f8a",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NemotronPaths {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub joiner: PathBuf,
    pub tokens: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FallbackModelStatus {
    Missing,
    Downloading,
    Verifying,
    Ready,
    Corrupted,
    Disabled,
    Error,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FallbackModelView {
    pub id: String,
    pub label: String,
    pub status: FallbackModelStatus,
    pub installed_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub progress_percent: Option<f32>,
    pub speed_mbps: Option<f32>,
    pub message: Option<String>,
    pub models_dir: String,
}

struct Artifact {
    file: &'static str,
    sha256: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactInstallState {
    Missing,
    Ready,
    Corrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerState {
    Valid,
    Missing,
    Stale,
}

pub fn root_dir() -> PathBuf {
    crate::stt::model::models_dir().join(MODEL_DIR)
}

pub fn is_installed() -> bool {
    classify_model(&root_dir()) == ArtifactInstallState::Ready
}

pub fn model_status(enabled: bool) -> FallbackModelView {
    model_status_at(&root_dir(), enabled)
}

pub fn model_status_at(root: &Path, enabled: bool) -> FallbackModelView {
    let status = match classify_model(root) {
        ArtifactInstallState::Missing => FallbackModelStatus::Missing,
        ArtifactInstallState::Corrupted => FallbackModelStatus::Corrupted,
        ArtifactInstallState::Ready if enabled => FallbackModelStatus::Ready,
        ArtifactInstallState::Ready => FallbackModelStatus::Disabled,
    };
    status_view(root, status, None, None, None, None, None)
}

pub fn verify_model(enabled: bool) -> FallbackModelView {
    let root = root_dir();
    match verify_model_at(&root) {
        Ok(()) => model_status_at(&root, enabled),
        Err(SttError::ModelMissing) => status_view(
            &root,
            FallbackModelStatus::Missing,
            None,
            None,
            None,
            None,
            None,
        ),
        Err(SttError::ModelCorrupt) => status_view(
            &root,
            FallbackModelStatus::Corrupted,
            None,
            None,
            None,
            None,
            None,
        ),
        Err(error) => status_view(
            &root,
            FallbackModelStatus::Error,
            None,
            None,
            None,
            None,
            Some(error.user_message().to_string()),
        ),
    }
}

pub fn ensure_model() -> Result<NemotronPaths, SttError> {
    let root = root_dir();
    std::fs::create_dir_all(&root).map_err(|_| SttError::ModelMissing)?;
    for artifact in ARTIFACTS {
        ensure_artifact(&root, artifact)?;
    }
    paths_at(root)
}

pub fn resolve_model() -> Result<NemotronPaths, SttError> {
    resolve_model_at(&root_dir())
}

pub fn remove_model() -> Result<(), SttError> {
    let root = root_dir();
    for artifact in ARTIFACTS {
        remove_if_exists(root.join(artifact.file))?;
        remove_if_exists(root.join(artifact.file).with_extension("verified"))?;
    }
    let _ = std::fs::remove_dir(&root);
    Ok(())
}

fn paths_at(root: PathBuf) -> Result<NemotronPaths, SttError> {
    Ok(NemotronPaths {
        encoder: require(root.join("encoder.int8.onnx"))?,
        decoder: require(root.join("decoder.int8.onnx"))?,
        joiner: require(root.join("joiner.int8.onnx"))?,
        tokens: require(root.join("tokens.txt"))?,
    })
}

fn require(path: PathBuf) -> Result<PathBuf, SttError> {
    path.exists().then_some(path).ok_or(SttError::ModelMissing)
}

fn ensure_artifact(root: &Path, artifact: &Artifact) -> Result<(), SttError> {
    let dest = root.join(artifact.file);
    if verify_or_trust(&dest, artifact.sha256).is_ok() {
        return Ok(());
    }
    let _ = std::fs::remove_file(&dest);
    let _ = std::fs::remove_file(dest.with_extension("verified"));
    let url = crate::stt::model::hf_resolve_url(REPO, REVISION, artifact.file);
    crate::stt::model::download_file(&url, &dest)?;
    verify_sha_and_mark(&dest, artifact.sha256)
}

fn classify_model(root: &Path) -> ArtifactInstallState {
    let mut saw_corrupt = false;

    for artifact in ARTIFACTS {
        match classify_artifact(&root.join(artifact.file), artifact.sha256) {
            ArtifactInstallState::Missing => return ArtifactInstallState::Missing,
            ArtifactInstallState::Corrupted => saw_corrupt = true,
            ArtifactInstallState::Ready => {}
        }
    }

    if saw_corrupt {
        ArtifactInstallState::Corrupted
    } else {
        ArtifactInstallState::Ready
    }
}

fn classify_artifact(path: &Path, expected_hash: &str) -> ArtifactInstallState {
    if !path.exists() {
        return ArtifactInstallState::Missing;
    }

    match marker_state(path, expected_hash) {
        MarkerState::Valid => ArtifactInstallState::Ready,
        MarkerState::Missing | MarkerState::Stale => ArtifactInstallState::Corrupted,
    }
}

fn marker_state(path: &Path, expected_hash: &str) -> MarkerState {
    let marker = path.with_extension("verified");
    let Ok(contents) = std::fs::read_to_string(&marker) else {
        return MarkerState::Missing;
    };
    let Ok(metadata) = std::fs::metadata(path) else {
        return MarkerState::Missing;
    };

    let mut lines = contents.lines();
    let Some(hash) = lines.next() else {
        return MarkerState::Stale;
    };
    let Some(size) = lines.next().and_then(|size| size.parse::<u64>().ok()) else {
        return MarkerState::Stale;
    };

    if hash.eq_ignore_ascii_case(expected_hash) && size == metadata.len() {
        MarkerState::Valid
    } else {
        MarkerState::Stale
    }
}

fn verify_or_trust(path: &Path, expected_hash: &str) -> Result<(), SttError> {
    let marker = path.with_extension("verified");
    if let (Ok(contents), Ok(metadata)) =
        (std::fs::read_to_string(&marker), std::fs::metadata(path))
    {
        let mut lines = contents.lines();
        if lines
            .next()
            .is_some_and(|hash| hash.eq_ignore_ascii_case(expected_hash))
            && lines.next().and_then(|size| size.parse::<u64>().ok()) == Some(metadata.len())
        {
            return Ok(());
        }
    }
    verify_sha_and_mark(path, expected_hash)
}

fn verify_sha_and_mark(path: &Path, expected_hash: &str) -> Result<(), SttError> {
    crate::stt::model::verify_sha256(path, expected_hash)?;
    let metadata = std::fs::metadata(path).map_err(|_| SttError::ModelMissing)?;
    std::fs::write(
        path.with_extension("verified"),
        format!("{expected_hash}\n{}\n", metadata.len()),
    )
    .map_err(|_| SttError::ModelMissing)
}

fn verify_model_at(root: &Path) -> Result<(), SttError> {
    for artifact in ARTIFACTS {
        verify_sha_and_mark(&root.join(artifact.file), artifact.sha256)?;
    }
    Ok(())
}

fn resolve_model_at(root: &Path) -> Result<NemotronPaths, SttError> {
    match classify_model(root) {
        ArtifactInstallState::Missing => Err(SttError::ModelMissing),
        ArtifactInstallState::Corrupted => Err(SttError::ModelCorrupt),
        ArtifactInstallState::Ready => paths_at(root.to_path_buf()),
    }
}

fn status_view(
    root: &Path,
    status: FallbackModelStatus,
    installed_bytes: Option<u64>,
    total_bytes: Option<u64>,
    progress_percent: Option<f32>,
    speed_mbps: Option<f32>,
    message: Option<String>,
) -> FallbackModelView {
    FallbackModelView {
        id: MODEL_ID.to_string(),
        label: FALLBACK_MODEL_LABEL.to_string(),
        status,
        installed_bytes,
        total_bytes,
        progress_percent,
        speed_mbps,
        message,
        models_dir: root.display().to_string(),
    }
}

fn remove_if_exists(path: PathBuf) -> Result<(), SttError> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|_| SttError::ModelMissing)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "nemotron-status-test-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).ok();
        }
    }

    fn write_verified_artifact(root: &Path, artifact: &Artifact, contents: &[u8]) {
        let path = root.join(artifact.file);
        std::fs::write(&path, contents).unwrap();
        std::fs::write(
            path.with_extension("verified"),
            format!("{}\n{}\n", artifact.sha256, contents.len()),
        )
        .unwrap();
    }

    #[test]
    fn model_root_is_named_for_nemotron() {
        assert!(root_dir().ends_with(MODEL_DIR));
    }

    #[test]
    fn pinned_artifacts_cover_sherpa_transducer_files() {
        let files = ARTIFACTS
            .iter()
            .map(|artifact| artifact.file)
            .collect::<Vec<_>>();
        assert_eq!(
            files,
            vec![
                "encoder.int8.onnx",
                "decoder.int8.onnx",
                "joiner.int8.onnx",
                "tokens.txt"
            ]
        );
        assert!(ARTIFACTS.iter().all(|artifact| artifact.sha256.len() == 64));
    }

    #[test]
    fn model_status_projects_missing_ready_disabled_and_corrupted() {
        let dir = TestDir::new();

        assert_eq!(
            model_status_at(dir.path(), true).status,
            FallbackModelStatus::Missing
        );

        for artifact in ARTIFACTS {
            write_verified_artifact(dir.path(), artifact, artifact.sha256.as_bytes());
        }

        assert_eq!(
            model_status_at(dir.path(), true).status,
            FallbackModelStatus::Ready
        );
        assert_eq!(
            model_status_at(dir.path(), false).status,
            FallbackModelStatus::Disabled
        );

        let marker = dir.path().join(ARTIFACTS[0].file).with_extension("verified");
        std::fs::write(&marker, format!("{}\n999\n", ARTIFACTS[0].sha256)).unwrap();

        assert_eq!(
            model_status_at(dir.path(), true).status,
            FallbackModelStatus::Corrupted
        );
    }

    #[test]
    fn synthetic_status_projection_covers_downloading_verifying_and_error() {
        let dir = TestDir::new();

        let downloading = status_view(
            dir.path(),
            FallbackModelStatus::Downloading,
            Some(32),
            Some(64),
            Some(50.0),
            Some(12.5),
            None,
        );
        assert_eq!(downloading.status, FallbackModelStatus::Downloading);
        assert_eq!(downloading.progress_percent, Some(50.0));
        assert_eq!(downloading.speed_mbps, Some(12.5));

        let verifying = status_view(
            dir.path(),
            FallbackModelStatus::Verifying,
            Some(64),
            Some(64),
            Some(100.0),
            None,
            Some("Verifying files".into()),
        );
        assert_eq!(verifying.status, FallbackModelStatus::Verifying);
        assert_eq!(verifying.message.as_deref(), Some("Verifying files"));

        let error = status_view(
            dir.path(),
            FallbackModelStatus::Error,
            None,
            None,
            None,
            None,
            Some("Download failed".into()),
        );
        assert_eq!(error.status, FallbackModelStatus::Error);
        assert_eq!(error.message.as_deref(), Some("Download failed"));
    }

    #[test]
    fn resolve_model_preserves_corrupt_status() {
        let dir = TestDir::new();

        for artifact in ARTIFACTS {
            write_verified_artifact(dir.path(), artifact, artifact.sha256.as_bytes());
        }
        std::fs::remove_file(dir.path().join(ARTIFACTS[0].file).with_extension("verified")).unwrap();

        assert_eq!(classify_model(dir.path()), ArtifactInstallState::Corrupted);
        assert_eq!(
            resolve_model_at(dir.path()).unwrap_err(),
            SttError::ModelCorrupt
        );
    }
}
