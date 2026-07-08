use std::path::{Path, PathBuf};

use crate::stt::error::SttError;

pub const MODEL_LABEL: &str = "Nemotron 3.5 ASR Streaming 0.6B INT8";
pub const CHUNK_MS: u64 = 1120;
pub const NUM_THREADS: i32 = 4;

const MODEL_DIR: &str = "nemotron-3.5-asr-streaming-0.6b-1120ms-int8";
const REPO: &str = "csukuangfj2/sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-1120ms-int8-2026-06-11";
const REVISION: &str = "d2f58fb3c1ae44829133de74c1b5aa6e3e6dda04";

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

struct Artifact {
    file: &'static str,
    sha256: &'static str,
}

pub fn root_dir() -> PathBuf {
    crate::stt::model::models_dir().join(MODEL_DIR)
}

pub fn is_installed() -> bool {
    ARTIFACTS
        .iter()
        .all(|artifact| verify_or_trust(&root_dir().join(artifact.file), artifact.sha256).is_ok())
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
    if !is_installed() {
        return Err(SttError::ModelMissing);
    }
    paths_at(root_dir())
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

fn remove_if_exists(path: PathBuf) -> Result<(), SttError> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|_| SttError::ModelMissing)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
