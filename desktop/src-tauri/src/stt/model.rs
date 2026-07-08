use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::stt::error::SttError;

pub fn models_dir_from<F>(env: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = env("YAP_MODELS_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(local) = env("LOCALAPPDATA") {
        return PathBuf::from(local).join("Yap").join("models");
    }
    PathBuf::from(".").join("models")
}

pub fn models_dir() -> PathBuf {
    models_dir_from(|key| std::env::var(key).ok())
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(hex)
}

pub fn verify_sha256(path: &Path, expected: &str) -> Result<(), SttError> {
    let actual = sha256_file(path).map_err(|_| SttError::ModelMissing)?;
    actual
        .eq_ignore_ascii_case(expected)
        .then_some(())
        .ok_or(SttError::ModelCorrupt)
}

pub fn hf_resolve_url(repo: &str, revision: &str, file: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/{revision}/{file}")
}

pub fn download_file(url: &str, dest: &Path) -> Result<(), SttError> {
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|_| SttError::ModelMissing)?;
    let mut response = client.get(url).send().map_err(|_| SttError::ModelMissing)?;
    if !response.status().is_success() {
        return Err(SttError::ModelMissing);
    }
    let tmp = dest.with_extension("part");
    let mut file = std::fs::File::create(&tmp).map_err(|_| SttError::ModelMissing)?;
    std::io::copy(&mut response, &mut file).map_err(|_| SttError::ModelMissing)?;
    drop(file);
    std::fs::rename(&tmp, dest).map_err(|_| SttError::ModelMissing)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_dir_prefers_override() {
        let dir = models_dir_from(|key| match key {
            "YAP_MODELS_DIR" => Some("D:/custom".into()),
            _ => None,
        });
        assert_eq!(dir, std::path::PathBuf::from("D:/custom"));
    }

    #[test]
    fn models_dir_falls_back_to_localappdata() {
        let dir = models_dir_from(|key| match key {
            "LOCALAPPDATA" => Some("C:/Users/me/AppData/Local".into()),
            _ => None,
        });
        assert_eq!(
            dir,
            std::path::PathBuf::from("C:/Users/me/AppData/Local")
                .join("Yap")
                .join("models")
        );
    }

    #[test]
    fn hf_resolve_url_is_pinned_by_revision() {
        assert_eq!(
            hf_resolve_url("owner/repo", "abc123", "model.onnx"),
            "https://huggingface.co/owner/repo/resolve/abc123/model.onnx"
        );
    }

    #[test]
    fn verify_sha256_matches_and_mismatches() {
        let dir = std::env::temp_dir().join(format!("yap-model-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("model.bin");
        std::fs::write(&file, b"hello").unwrap();

        assert_eq!(
            sha256_file(&file).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert!(verify_sha256(
            &file,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        )
        .is_ok());
        assert_eq!(
            verify_sha256(&file, "bad").unwrap_err(),
            SttError::ModelCorrupt
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verify_sha256_missing_file_is_model_missing() {
        assert_eq!(
            verify_sha256(Path::new("missing.bin"), "abc").unwrap_err(),
            SttError::ModelMissing
        );
    }
}
