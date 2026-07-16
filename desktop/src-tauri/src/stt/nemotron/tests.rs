use super::catalog::{
    classify_artifacts, marker_state, status_view, verify_artifacts_at_with_progress,
    verify_sha_and_mark,
};
use super::lifecycle::{
    load_local_fallback_at_with_artifacts, local_fallback_start_paths_at_with_artifacts,
    remove_download_artifacts, resolve_model_at_with_artifacts,
};
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

mod catalog;
mod loading;
mod removal;
