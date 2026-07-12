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
        bytes: 657_601_521,
    },
    Artifact {
        file: "decoder.int8.onnx",
        sha256: "19f9c98fc6d0a2c33a65a43b36fdb2e914c26c0aa9764be3aebc502a1e982fb0",
        bytes: 14_978_075,
    },
    Artifact {
        file: "joiner.int8.onnx",
        sha256: "4101c7c679a0bc30483794b27a059e34e79232aa2068d78d51231a22c8b0d7ce",
        bytes: 9_504_438,
    },
    Artifact {
        file: "tokens.txt",
        sha256: "729cc103155bafa785f9cd45746cd41cabe97eab7182fc04d594129587958f8a",
        bytes: 131_440,
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

#[derive(Debug, Clone, Copy)]
struct Artifact {
    file: &'static str,
    sha256: &'static str,
    bytes: u64,
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
    model_status_at_with_artifacts(root, enabled, ARTIFACTS)
}

fn model_status_at_with_artifacts(
    root: &Path,
    enabled: bool,
    artifacts: &[Artifact],
) -> FallbackModelView {
    let status = match classify_artifacts(root, artifacts) {
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
    ensure_model_with_progress(false, |_| {}, &crate::stt::model::DownloadOperation::new(0))
}

pub fn ensure_model_with_progress<P>(
    force: bool,
    mut on_progress: P,
    operation: &crate::stt::model::DownloadOperation,
) -> Result<NemotronPaths, SttError>
where
    P: FnMut(FallbackModelView),
{
    let root = root_dir();
    std::fs::create_dir_all(&root).map_err(|_| SttError::ModelMissing)?;
    for artifact in ARTIFACTS {
        ensure_artifact(&root, artifact, force, &mut on_progress, operation)?;
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
    match std::fs::remove_dir(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(SttError::ModelMissing),
    }
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

fn ensure_artifact<P>(
    root: &Path,
    artifact: &Artifact,
    force: bool,
    on_progress: &mut P,
    operation: &crate::stt::model::DownloadOperation,
) -> Result<(), SttError>
where
    P: FnMut(FallbackModelView),
{
    let dest = root.join(artifact.file);
    crate::stt::model::cleanup_stale_download_temps(&dest, operation)?;

    if !force && verify_or_trust(&dest, artifact).is_ok() {
        return Ok(());
    }

    let url = crate::stt::model::hf_resolve_url(REPO, REVISION, artifact.file);
    crate::stt::model::download_verified_file(
        &crate::stt::model::DownloadRequest {
            url,
            destination: dest.clone(),
            expected_bytes: artifact.bytes,
            expected_sha256: artifact.sha256.to_string(),
        },
        operation,
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
    )?;

    on_progress(status_view(
        root,
        FallbackModelStatus::Verifying,
        Some(artifact.bytes),
        Some(artifact.bytes),
        Some(100.0),
        None,
        Some(format!("Verifying {}", artifact.file)),
    ));

    write_verified_marker(&dest, artifact)?;
    if operation.is_cancelled() {
        return Err(SttError::ModelInstallCancelled);
    }

    Ok(())
}

fn classify_model(root: &Path) -> ArtifactInstallState {
    classify_artifacts(root, ARTIFACTS)
}

fn classify_artifacts(root: &Path, artifacts: &[Artifact]) -> ArtifactInstallState {
    let mut saw_corrupt = false;

    for artifact in artifacts {
        match classify_artifact(&root.join(artifact.file), artifact) {
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

fn classify_artifact(path: &Path, artifact: &Artifact) -> ArtifactInstallState {
    if !path.exists() {
        return ArtifactInstallState::Missing;
    }

    match marker_state(path, artifact) {
        MarkerState::Valid => ArtifactInstallState::Ready,
        MarkerState::Missing | MarkerState::Stale => ArtifactInstallState::Corrupted,
    }
}

fn marker_state(path: &Path, artifact: &Artifact) -> MarkerState {
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

    if hash.eq_ignore_ascii_case(artifact.sha256)
        && size == artifact.bytes
        && metadata.len() == artifact.bytes
    {
        MarkerState::Valid
    } else {
        MarkerState::Stale
    }
}

fn verify_or_trust(path: &Path, artifact: &Artifact) -> Result<(), SttError> {
    if marker_state(path, artifact) == MarkerState::Valid {
        return Ok(());
    }
    verify_sha_and_mark(path, artifact)
}

fn verify_sha_and_mark(path: &Path, artifact: &Artifact) -> Result<(), SttError> {
    let metadata = std::fs::metadata(path).map_err(|_| SttError::ModelMissing)?;
    if metadata.len() != artifact.bytes {
        return Err(SttError::ModelCorrupt);
    }
    crate::stt::model::verify_sha256(path, artifact.sha256)?;
    write_verified_marker(path, artifact)
}

fn write_verified_marker(path: &Path, artifact: &Artifact) -> Result<(), SttError> {
    crate::stt::model::write_text_atomically(
        &path.with_extension("verified"),
        &format!("{}\n{}\n", artifact.sha256, artifact.bytes),
    )
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
    verify_artifacts_at_with_progress(root, ARTIFACTS, on_progress, is_cancelled)
}

fn verify_artifacts_at_with_progress<P, C>(
    root: &Path,
    artifacts: &[Artifact],
    on_progress: &mut P,
    is_cancelled: C,
) -> Result<(), SttError>
where
    P: FnMut(FallbackModelView),
    C: Fn() -> bool + Copy,
{
    let total_bytes = artifacts.iter().try_fold(0u64, |total, artifact| {
        let size = std::fs::metadata(root.join(artifact.file))
            .map_err(|_| SttError::ModelMissing)?
            .len();
        if size != artifact.bytes {
            return Err(SttError::ModelCorrupt);
        }
        total
            .checked_add(artifact.bytes)
            .ok_or(SttError::ModelCorrupt)
    })?;
    let mut verified_bytes = 0u64;

    for artifact in artifacts {
        if is_cancelled() {
            return Err(SttError::ModelInstallCancelled);
        }
        let path = root.join(artifact.file);
        verify_sha_and_mark(&path, artifact)?;
        verified_bytes = verified_bytes
            .checked_add(artifact.bytes)
            .ok_or(SttError::ModelCorrupt)?;
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
    resolve_model_at_with_artifacts(root, ARTIFACTS)
}

fn resolve_model_at_with_artifacts(
    root: &Path,
    artifacts: &[Artifact],
) -> Result<NemotronPaths, SttError> {
    match classify_artifacts(root, artifacts) {
        ArtifactInstallState::Missing => Err(SttError::ModelMissing),
        ArtifactInstallState::Corrupted => Err(SttError::ModelCorrupt),
        ArtifactInstallState::Ready => paths_at(root.to_path_buf()),
    }
}

fn local_fallback_start_paths_at(root: &Path, enabled: bool) -> Result<NemotronPaths, SttError> {
    local_fallback_start_paths_at_with_artifacts(root, enabled, ARTIFACTS)
}

fn local_fallback_start_paths_at_with_artifacts(
    root: &Path,
    enabled: bool,
    artifacts: &[Artifact],
) -> Result<NemotronPaths, SttError> {
    if !enabled {
        return Err(SttError::FallbackDisabled);
    }
    resolve_model_at_with_artifacts(root, artifacts)
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
    remove_unique_partial_artifacts(path)?;
    Ok(())
}

fn remove_unique_partial_artifacts(path: &Path) -> Result<(), SttError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if !parent.exists() {
        return Ok(());
    }
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    let prefix = format!("{file_name}.");

    let entries = std::fs::read_dir(parent).map_err(|_| SttError::ModelMissing)?;
    for entry in entries {
        let entry = entry.map_err(|_| SttError::ModelMissing)?;
        let candidate = entry.path();
        let Some(candidate_name) = candidate.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if candidate_name.starts_with(&prefix) && candidate_name.ends_with(".part") {
            remove_if_exists(candidate)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::thread::sleep;
    use std::time::Duration;
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_ARTIFACT_CONTENTS: &[u8] = b"abc";
    const TEST_ARTIFACT_SHA256: &str =
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const TEST_ARTIFACTS: &[Artifact] = &[
        Artifact {
            file: "encoder.int8.onnx",
            sha256: TEST_ARTIFACT_SHA256,
            bytes: 3,
        },
        Artifact {
            file: "decoder.int8.onnx",
            sha256: TEST_ARTIFACT_SHA256,
            bytes: 3,
        },
        Artifact {
            file: "joiner.int8.onnx",
            sha256: TEST_ARTIFACT_SHA256,
            bytes: 3,
        },
        Artifact {
            file: "tokens.txt",
            sha256: TEST_ARTIFACT_SHA256,
            bytes: 3,
        },
    ];

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

    fn write_verified_artifact(root: &Path, artifact: &Artifact) {
        let path = root.join(artifact.file);
        assert_eq!(artifact.bytes, TEST_ARTIFACT_CONTENTS.len() as u64);
        assert_eq!(artifact.sha256, TEST_ARTIFACT_SHA256);
        std::fs::write(&path, TEST_ARTIFACT_CONTENTS).unwrap();
        std::fs::write(
            path.with_extension("verified"),
            format!("{}\n{}\n", artifact.sha256, artifact.bytes),
        )
        .unwrap();
    }

    fn tamper_artifact_same_size_after_marker(root: &Path, artifact: &Artifact) {
        let path = root.join(artifact.file);
        let marker_modified = std::fs::metadata(path.with_extension("verified"))
            .unwrap()
            .modified()
            .unwrap();
        for _ in 0..20 {
            sleep(Duration::from_millis(25));
            let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.write_all(b"x").unwrap();
            file.sync_all().unwrap();
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
        assert_eq!(
            ARTIFACTS
                .iter()
                .map(|artifact| (artifact.file, artifact.bytes))
                .collect::<Vec<_>>(),
            vec![
                ("encoder.int8.onnx", 657_601_521),
                ("decoder.int8.onnx", 14_978_075),
                ("joiner.int8.onnx", 9_504_438),
                ("tokens.txt", 131_440),
            ]
        );
    }

    #[test]
    fn marker_rejects_a_file_whose_size_does_not_match_the_pinned_artifact() {
        let dir = TestDir::new();
        let artifact = &TEST_ARTIFACTS[0];
        let path = dir.path().join(artifact.file);
        std::fs::write(&path, b"abcd").unwrap();
        std::fs::write(
            path.with_extension("verified"),
            format!("{}\n4\n", artifact.sha256),
        )
        .unwrap();

        assert_eq!(marker_state(&path, artifact), MarkerState::Stale);
    }

    #[test]
    fn verification_rejects_length_before_accepting_a_matching_hash() {
        let dir = TestDir::new();
        let artifact = Artifact {
            file: "model.bin",
            sha256: TEST_ARTIFACT_SHA256,
            bytes: 4,
        };
        let path = dir.path().join(artifact.file);
        std::fs::write(&path, TEST_ARTIFACT_CONTENTS).unwrap();

        assert_eq!(
            verify_sha_and_mark(&path, &artifact),
            Err(SttError::ModelCorrupt)
        );
        assert!(!path.with_extension("verified").exists());
    }

    #[test]
    fn model_verification_preflights_all_lengths_before_hashing() {
        let dir = TestDir::new();
        let artifacts = [
            Artifact {
                file: "encoder.int8.onnx",
                sha256: TEST_ARTIFACT_SHA256,
                bytes: 3,
            },
            Artifact {
                file: "decoder.int8.onnx",
                sha256: TEST_ARTIFACT_SHA256,
                bytes: 2,
            },
        ];
        let first = dir.path().join(artifacts[0].file);
        std::fs::write(&first, TEST_ARTIFACT_CONTENTS).unwrap();
        std::fs::write(dir.path().join(artifacts[1].file), b"x").unwrap();

        assert_eq!(
            verify_artifacts_at_with_progress(dir.path(), &artifacts, &mut |_| {}, || false),
            Err(SttError::ModelCorrupt)
        );
        assert!(!first.with_extension("verified").exists());
    }

    #[test]
    fn model_status_projects_missing_ready_disabled_and_corrupted() {
        let dir = TestDir::new();

        assert_eq!(
            model_status_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS).status,
            FallbackModelStatus::Missing
        );

        for artifact in TEST_ARTIFACTS {
            write_verified_artifact(dir.path(), artifact);
        }

        assert_eq!(
            model_status_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS).status,
            FallbackModelStatus::Ready
        );
        assert_eq!(
            model_status_at_with_artifacts(dir.path(), false, TEST_ARTIFACTS).status,
            FallbackModelStatus::Disabled
        );

        let marker = dir
            .path()
            .join(TEST_ARTIFACTS[0].file)
            .with_extension("verified");
        std::fs::write(&marker, format!("{}\n999\n", TEST_ARTIFACTS[0].sha256)).unwrap();

        assert_eq!(
            model_status_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS).status,
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

        for artifact in TEST_ARTIFACTS {
            write_verified_artifact(dir.path(), artifact);
        }
        std::fs::remove_file(
            dir.path()
                .join(TEST_ARTIFACTS[0].file)
                .with_extension("verified"),
        )
        .unwrap();

        assert_eq!(
            classify_artifacts(dir.path(), TEST_ARTIFACTS),
            ArtifactInstallState::Corrupted
        );
        assert_eq!(
            resolve_model_at_with_artifacts(dir.path(), TEST_ARTIFACTS).unwrap_err(),
            SttError::ModelCorrupt
        );
    }

    #[test]
    fn local_fallback_start_paths_require_enabled_even_when_ready() {
        let dir = TestDir::new();

        for artifact in TEST_ARTIFACTS {
            write_verified_artifact(dir.path(), artifact);
        }

        assert_eq!(
            local_fallback_start_paths_at_with_artifacts(dir.path(), false, TEST_ARTIFACTS)
                .unwrap_err(),
            SttError::FallbackDisabled
        );
        assert!(
            local_fallback_start_paths_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS).is_ok()
        );
    }

    #[test]
    fn local_fallback_start_paths_preserve_missing_and_corrupt_failures() {
        let dir = TestDir::new();

        assert_eq!(
            local_fallback_start_paths_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS)
                .unwrap_err(),
            SttError::ModelMissing
        );

        for artifact in TEST_ARTIFACTS {
            write_verified_artifact(dir.path(), artifact);
        }
        std::fs::remove_file(
            dir.path()
                .join(TEST_ARTIFACTS[0].file)
                .with_extension("verified"),
        )
        .unwrap();

        assert_eq!(
            local_fallback_start_paths_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS)
                .unwrap_err(),
            SttError::ModelCorrupt
        );
    }

    #[test]
    fn same_size_tampering_after_marker_creation_is_corrupted() {
        let dir = TestDir::new();

        for artifact in TEST_ARTIFACTS {
            write_verified_artifact(dir.path(), artifact);
        }
        tamper_artifact_same_size_after_marker(dir.path(), &TEST_ARTIFACTS[0]);

        assert_eq!(
            marker_state(&dir.path().join(TEST_ARTIFACTS[0].file), &TEST_ARTIFACTS[0]),
            MarkerState::Stale
        );
        assert_eq!(
            model_status_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS).status,
            FallbackModelStatus::Corrupted
        );
        assert_eq!(
            resolve_model_at_with_artifacts(dir.path(), TEST_ARTIFACTS).unwrap_err(),
            SttError::ModelCorrupt
        );
    }

    #[test]
    fn remove_download_artifacts_cleans_file_marker_and_partial() {
        let dir = TestDir::new();
        let path = dir.path().join(ARTIFACTS[0].file);
        let unique_partial = path.with_file_name(format!(
            "{}.123.456.0.part",
            path.file_name().and_then(|name| name.to_str()).unwrap()
        ));
        std::fs::write(&path, b"current").unwrap();
        std::fs::write(path.with_extension("verified"), b"marker").unwrap();
        std::fs::write(path.with_extension("part"), b"partial").unwrap();
        std::fs::write(&unique_partial, b"unique partial").unwrap();

        remove_download_artifacts(&path).unwrap();

        assert!(!path.exists());
        assert!(!path.with_extension("verified").exists());
        assert!(!path.with_extension("part").exists());
        assert!(!unique_partial.exists());
    }

    #[test]
    fn remove_download_artifacts_rejects_unique_partial_directories() {
        let dir = TestDir::new();
        let path = dir.path().join(ARTIFACTS[0].file);
        let unique_partial = path.with_file_name(format!(
            "{}.123.456.0.part",
            path.file_name().and_then(|name| name.to_str()).unwrap()
        ));
        std::fs::write(&path, b"current").unwrap();
        std::fs::create_dir_all(&unique_partial).unwrap();

        let error = remove_download_artifacts(&path).unwrap_err();

        assert_eq!(error, SttError::ModelCorrupt);
        assert!(unique_partial.is_dir());
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
