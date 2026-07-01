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
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(SttError::ModelCorrupt)
    }
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
            std::path::PathBuf::from("C:/Users/me/AppData/Local").join("Yap").join("models")
        );
    }

    #[test]
    fn verify_sha256_matches_and_mismatches() {
        let path = std::env::temp_dir().join(format!("yap-sha-{}.bin", std::process::id()));
        std::fs::write(&path, b"hello").unwrap();
        let expected = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify_sha256(&path, expected).is_ok());
        assert_eq!(verify_sha256(&path, &"0".repeat(64)).unwrap_err(), SttError::ModelCorrupt);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn verify_sha256_missing_file_is_model_missing() {
        let path = std::env::temp_dir().join("yap-absent-3f9c1a.bin");
        assert_eq!(verify_sha256(&path, &"0".repeat(64)).unwrap_err(), SttError::ModelMissing);
    }
}
