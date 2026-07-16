use super::*;

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
