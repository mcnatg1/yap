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
    verify_model_with_progress(enabled, |_| {}, || false)
}

pub fn verify_model_with_progress<P, C>(
    enabled: bool,
    mut on_progress: P,
    is_cancelled: C,
) -> FallbackModelView
where
    P: FnMut(FallbackModelView),
    C: Fn() -> bool + Copy,
{
    let root = root_dir();
    match verify_model_at_with_progress(&root, &mut on_progress, is_cancelled) {
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
    ensure_model_with_progress(false, |_| {}, || false)
}

pub fn ensure_model_with_progress<P, C>(
    force: bool,
    mut on_progress: P,
    is_cancelled: C,
) -> Result<NemotronPaths, SttError>
where
    P: FnMut(FallbackModelView),
    C: Fn() -> bool + Copy,
{
    let root = root_dir();
    std::fs::create_dir_all(&root).map_err(|_| SttError::ModelMissing)?;
    for artifact in ARTIFACTS {
        ensure_artifact(&root, artifact, force, &mut on_progress, is_cancelled)?;
    }
    paths_at(root)
}

pub fn local_fallback_start_paths() -> Result<NemotronPaths, SttError> {
    local_fallback_start_paths_at(&root_dir(), crate::stt::settings::local_fallback_enabled())
}

pub fn resolve_model() -> Result<NemotronPaths, SttError> {
    resolve_model_at(&root_dir())
}

pub fn remove_model() -> Result<(), SttError> {
    let root = root_dir();
    for artifact in ARTIFACTS {
        remove_download_artifacts(&root.join(artifact.file))?;
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

fn ensure_artifact<P, C>(
    root: &Path,
    artifact: &Artifact,
    force: bool,
    on_progress: &mut P,
    is_cancelled: C,
) -> Result<(), SttError>
where
    P: FnMut(FallbackModelView),
    C: Fn() -> bool + Copy,
{
    let dest = root.join(artifact.file);

    if force {
        remove_download_artifacts(&dest)?;
    } else if verify_or_trust(&dest, artifact.sha256).is_ok() {
        return Ok(());
    }

    remove_download_artifacts(&dest)?;
    let url = crate::stt::model::hf_resolve_url(REPO, REVISION, artifact.file);
    let download = crate::stt::model::download_file_with_progress(
        &url,
        &dest,
        |progress| {
            on_progress(status_view(
                root,
                FallbackModelStatus::Downloading,
                Some(progress.downloaded_bytes),
                progress.total_bytes,
                progress.percent(),
                progress.speed_mbps(),
                Some(format!("Downloading {}", artifact.file)),
            ));
        },
        is_cancelled,
    );

    if let Err(error) = download {
        remove_download_artifacts(&dest)?;
        return Err(error);
    }

    on_progress(status_view(
        root,
        FallbackModelStatus::Verifying,
        Some(
            std::fs::metadata(&dest)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
        ),
        Some(
            std::fs::metadata(&dest)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
        ),
        Some(100.0),
        None,
        Some(format!("Verifying {}", artifact.file)),
    ));

    if let Err(error) = verify_sha_and_mark(&dest, artifact.sha256) {
        remove_download_artifacts(&dest)?;
        return Err(error);
    }

    Ok(())
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
    let Ok(marker_metadata) = std::fs::metadata(&marker) else {
        return MarkerState::Missing;
    };
    let Ok(artifact_modified) = metadata.modified() else {
        return MarkerState::Stale;
    };
    let Ok(marker_modified) = marker_metadata.modified() else {
        return MarkerState::Stale;
    };

    let mut lines = contents.lines();
    let Some(hash) = lines.next() else {
        return MarkerState::Stale;
    };
    let Some(size) = lines.next().and_then(|size| size.parse::<u64>().ok()) else {
        return MarkerState::Stale;
    };

    if marker_modified < artifact_modified {
        return MarkerState::Stale;
    }

    if hash.eq_ignore_ascii_case(expected_hash) && size == metadata.len() {
        MarkerState::Valid
    } else {
        MarkerState::Stale
    }
}

fn verify_or_trust(path: &Path, expected_hash: &str) -> Result<(), SttError> {
    if marker_state(path, expected_hash) == MarkerState::Valid {
        return Ok(());
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

fn verify_model_at_with_progress<P, C>(
    root: &Path,
    on_progress: &mut P,
    is_cancelled: C,
) -> Result<(), SttError>
where
    P: FnMut(FallbackModelView),
    C: Fn() -> bool + Copy,
{
    let total_bytes = ARTIFACTS.iter().try_fold(0u64, |acc, artifact| {
        let size = std::fs::metadata(root.join(artifact.file))
            .map_err(|_| SttError::ModelMissing)?
            .len();
        Ok::<u64, SttError>(acc + size)
    })?;
    let mut verified_bytes = 0u64;

    for artifact in ARTIFACTS {
        if is_cancelled() {
            return Err(SttError::ModelInstallCancelled);
        }
        let path = root.join(artifact.file);
        let size = std::fs::metadata(&path)
            .map_err(|_| SttError::ModelMissing)?
            .len();
        verify_sha_and_mark(&path, artifact.sha256)?;
        verified_bytes += size;
        on_progress(status_view(
            root,
            FallbackModelStatus::Verifying,
            Some(verified_bytes),
            Some(total_bytes),
            Some(progress_percent(verified_bytes, total_bytes)),
            None,
            Some(format!("Verifying {}", artifact.file)),
        ));
        if is_cancelled() {
            return Err(SttError::ModelInstallCancelled);
        }
    }
    Ok(())
}

fn progress_percent(complete: u64, total: u64) -> f32 {
    if total == 0 {
        return 100.0;
    }
    ((complete as f32 / total as f32) * 100.0).clamp(0.0, 100.0)
}

fn resolve_model_at(root: &Path) -> Result<NemotronPaths, SttError> {
    match classify_model(root) {
        ArtifactInstallState::Missing => Err(SttError::ModelMissing),
        ArtifactInstallState::Corrupted => Err(SttError::ModelCorrupt),
        ArtifactInstallState::Ready => paths_at(root.to_path_buf()),
    }
}

fn local_fallback_start_paths_at(root: &Path, enabled: bool) -> Result<NemotronPaths, SttError> {
    if !enabled {
        return Err(SttError::FallbackDisabled);
    }
    resolve_model_at(root)
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
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() => return Err(SttError::ModelCorrupt),
        Ok(_) => {
            std::fs::remove_file(path).map_err(|_| SttError::ModelMissing)?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(SttError::ModelMissing),
    }
    Ok(())
}

fn remove_download_artifacts(path: &Path) -> Result<(), SttError> {
    remove_if_exists(path.to_path_buf())?;
    remove_if_exists(path.with_extension("verified"))?;
    remove_if_exists(path.with_extension("part"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;
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

    fn tamper_artifact_same_size_after_marker(root: &Path, artifact: &Artifact) {
        let path = root.join(artifact.file);
        let marker_modified = std::fs::metadata(path.with_extension("verified"))
            .unwrap()
            .modified()
            .unwrap();
        let tampered = vec![b'x'; artifact.sha256.len()];

        for _ in 0..20 {
            sleep(Duration::from_millis(25));
            std::fs::write(&path, &tampered).unwrap();
            let artifact_modified = std::fs::metadata(&path).unwrap().modified().unwrap();
            if artifact_modified > marker_modified {
                return;
            }
        }

        panic!("artifact modified time did not advance past marker modified time");
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

        let marker = dir
            .path()
            .join(ARTIFACTS[0].file)
            .with_extension("verified");
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
        std::fs::remove_file(
            dir.path()
                .join(ARTIFACTS[0].file)
                .with_extension("verified"),
        )
        .unwrap();

        assert_eq!(classify_model(dir.path()), ArtifactInstallState::Corrupted);
        assert_eq!(
            resolve_model_at(dir.path()).unwrap_err(),
            SttError::ModelCorrupt
        );
    }

    #[test]
    fn local_fallback_start_paths_require_enabled_even_when_ready() {
        let dir = TestDir::new();

        for artifact in ARTIFACTS {
            write_verified_artifact(dir.path(), artifact, artifact.sha256.as_bytes());
        }

        assert_eq!(
            local_fallback_start_paths_at(dir.path(), false).unwrap_err(),
            SttError::FallbackDisabled
        );
        assert!(local_fallback_start_paths_at(dir.path(), true).is_ok());
    }

    #[test]
    fn local_fallback_start_paths_preserve_missing_and_corrupt_failures() {
        let dir = TestDir::new();

        assert_eq!(
            local_fallback_start_paths_at(dir.path(), true).unwrap_err(),
            SttError::ModelMissing
        );

        for artifact in ARTIFACTS {
            write_verified_artifact(dir.path(), artifact, artifact.sha256.as_bytes());
        }
        std::fs::remove_file(
            dir.path()
                .join(ARTIFACTS[0].file)
                .with_extension("verified"),
        )
        .unwrap();

        assert_eq!(
            local_fallback_start_paths_at(dir.path(), true).unwrap_err(),
            SttError::ModelCorrupt
        );
    }

    #[test]
    fn same_size_tampering_after_marker_creation_is_corrupted() {
        let dir = TestDir::new();

        for artifact in ARTIFACTS {
            write_verified_artifact(dir.path(), artifact, artifact.sha256.as_bytes());
        }
        tamper_artifact_same_size_after_marker(dir.path(), &ARTIFACTS[0]);

        assert_eq!(
            marker_state(&dir.path().join(ARTIFACTS[0].file), ARTIFACTS[0].sha256),
            MarkerState::Stale
        );
        assert_eq!(
            model_status_at(dir.path(), true).status,
            FallbackModelStatus::Corrupted
        );
        assert_eq!(
            resolve_model_at(dir.path()).unwrap_err(),
            SttError::ModelCorrupt
        );
    }

    #[test]
    fn remove_download_artifacts_cleans_file_marker_and_partial() {
        let dir = TestDir::new();
        let path = dir.path().join(ARTIFACTS[0].file);
        std::fs::write(&path, b"current").unwrap();
        std::fs::write(path.with_extension("verified"), b"marker").unwrap();
        std::fs::write(path.with_extension("part"), b"partial").unwrap();

        remove_download_artifacts(&path).unwrap();

        assert!(!path.exists());
        assert!(!path.with_extension("verified").exists());
        assert!(!path.with_extension("part").exists());
    }

    #[test]
    fn remove_download_artifacts_rejects_artifact_directories() {
        let dir = TestDir::new();
        let path = dir.path().join(ARTIFACTS[0].file);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::create_dir_all(path.with_extension("verified")).unwrap();
        std::fs::create_dir_all(path.with_extension("part")).unwrap();

        let error = remove_download_artifacts(&path).unwrap_err();

        assert_eq!(error, SttError::ModelCorrupt);
        assert!(path.is_dir());
        assert!(path.with_extension("verified").is_dir());
        assert!(path.with_extension("part").is_dir());
    }
}
