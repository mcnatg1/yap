use std::path::{Path, PathBuf};

use crate::stt::error::SttError;

mod catalog;
mod lifecycle;
mod load_guard;

pub(crate) use load_guard::ModelLoadGuard;

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

pub(crate) struct LoadedNemotronModel<T> {
    value: T,
    guard: ModelLoadGuard,
}

impl<T> LoadedNemotronModel<T> {
    pub(crate) fn into_parts(self) -> (T, ModelLoadGuard) {
        (self.value, self.guard)
    }
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

pub(crate) fn local_fallback_readiness() -> Result<(), SttError> {
    local_fallback_start_paths_at(&root_dir(), crate::stt::settings::local_fallback_enabled())
        .map(drop)
}

pub(crate) fn load_local_fallback<T, F>(loader: F) -> Result<LoadedNemotronModel<T>, SttError>
where
    F: FnOnce(&NemotronPaths) -> Result<T, SttError>,
{
    load_local_fallback_at_with_artifacts(
        &root_dir(),
        crate::stt::settings::local_fallback_enabled(),
        ARTIFACTS,
        loader,
    )
}
pub fn resolve_model() -> Result<NemotronPaths, SttError> {
    resolve_model_at(&root_dir())
}

pub fn remove_model() -> Result<(), SttError> {
    let root = root_dir();
    load_guard::cleanup_stale_snapshots(&root)?;
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

use catalog::{classify_artifacts, classify_model, status_view, verify_model_at_with_progress};
use lifecycle::{
    ensure_artifact, load_local_fallback_at_with_artifacts, local_fallback_start_paths_at,
    paths_at, remove_download_artifacts, resolve_model_at,
};

#[cfg(test)]
mod tests;
