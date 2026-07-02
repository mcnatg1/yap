use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::stt::error::SttError;
use crate::stt::pin::CrispasrPin;

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

fn verified_marker_path(model: &Path) -> PathBuf {
    model.with_extension("verified")
}

fn trusted_cache(model: &Path, expected_hash: &str) -> bool {
    let marker = verified_marker_path(model);
    let Ok(contents) = std::fs::read_to_string(&marker) else {
        return false;
    };
    let Ok(metadata) = std::fs::metadata(model) else {
        return false;
    };
    let mut lines = contents.lines();
    let Some(hash) = lines.next() else {
        return false;
    };
    let Some(size) = lines.next() else {
        return false;
    };
    hash.eq_ignore_ascii_case(expected_hash)
        && size.parse::<u64>().ok() == Some(metadata.len())
}

fn write_verified_marker(model: &Path, expected_hash: &str) -> Result<(), SttError> {
    let metadata = std::fs::metadata(model).map_err(|_| SttError::ModelMissing)?;
    let marker = verified_marker_path(model);
    std::fs::write(marker, format!("{expected_hash}\n{}\n", metadata.len())).map_err(|_| SttError::ModelMissing)
}

fn verify_or_trust(model: &Path, expected_hash: &str) -> Result<(), SttError> {
    if trusted_cache(model, expected_hash) {
        return Ok(());
    }
    verify_sha256(model, expected_hash)?;
    write_verified_marker(model, expected_hash)
}

pub fn hf_resolve_url(repo: &str, revision: &str, file: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/{revision}/{file}")
}

pub fn is_installed(pin: &CrispasrPin) -> bool {
    let dir = models_dir();
    verify_or_trust(&dir.join(&pin.gguf_file), &pin.gguf_sha256).is_ok()
        && verify_or_trust(&dir.join(&pin.tokenizer_file), &pin.tokenizer_sha256).is_ok()
        && verify_or_trust(&dir.join(&pin.punc_file), &pin.punc_sha256).is_ok()
}

pub fn ensure_model_at<D>(dir: &Path, pin: &CrispasrPin, mut download: D) -> Result<PathBuf, SttError>
where
    D: FnMut(&str, &Path) -> Result<(), SttError>,
{
    let dest = ensure_artifact_at(
        dir,
        &pin.gguf_repo,
        &pin.gguf_revision,
        &pin.gguf_file,
        &pin.gguf_sha256,
        &mut download,
    )?;
    ensure_artifact_at(
        dir,
        &pin.gguf_repo,
        &pin.gguf_revision,
        &pin.tokenizer_file,
        &pin.tokenizer_sha256,
        &mut download,
    )?;
    ensure_artifact_at(
        dir,
        &pin.punc_repo,
        &pin.punc_revision,
        &pin.punc_file,
        &pin.punc_sha256,
        &mut download,
    )?;
    Ok(dest)
}

fn ensure_artifact_at<D>(
    dir: &Path,
    repo: &str,
    revision: &str,
    file: &str,
    expected_hash: &str,
    download: &mut D,
) -> Result<PathBuf, SttError>
where
    D: FnMut(&str, &Path) -> Result<(), SttError>,
{
    let dest = dir.join(file);
    if dest.exists() {
        match verify_or_trust(&dest, expected_hash) {
            Ok(()) => return Ok(dest),
            Err(SttError::ModelCorrupt) => {
                let _ = std::fs::remove_file(&dest);
                let _ = std::fs::remove_file(verified_marker_path(&dest));
            }
            Err(err) => return Err(err),
        }
    }
    std::fs::create_dir_all(dir).map_err(|_| SttError::ModelMissing)?;
    let url = hf_resolve_url(repo, revision, file);
    download(&url, &dest)?;
    match verify_sha256(&dest, expected_hash) {
        Ok(()) => {
            write_verified_marker(&dest, expected_hash)?;
            Ok(dest)
        }
        Err(err) => {
            let _ = std::fs::remove_file(&dest);
            let _ = std::fs::remove_file(verified_marker_path(&dest));
            Err(err)
        }
    }
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

pub fn ensure_model() -> Result<PathBuf, SttError> {
    let pin = crate::stt::pin::load_pin().map_err(|_| SttError::ModelCorrupt)?;
    ensure_model_at(&models_dir(), &pin, download_file)
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

    #[test]
    fn hf_resolve_url_is_pinned_by_revision() {
        assert_eq!(
            hf_resolve_url("owner/repo", "abc123", "model.gguf"),
            "https://huggingface.co/owner/repo/resolve/abc123/model.gguf"
        );
    }

    fn sample_pin(gguf_sha256: &str) -> crate::stt::pin::CrispasrPin {
        crate::stt::pin::CrispasrPin {
            crispasr_version: "0.6.12".into(),
            binary_sha256: "a".repeat(64),
            gguf_repo: "owner/repo".into(),
            gguf_revision: "rev".into(),
            gguf_file: "m.gguf".into(),
            gguf_sha256: gguf_sha256.into(),
            tokenizer_file: "tokenizer.bin".into(),
            tokenizer_sha256: gguf_sha256.into(),
            punc_repo: "owner/punc".into(),
            punc_revision: "punc-rev".into(),
            punc_file: "punc.gguf".into(),
            punc_sha256: gguf_sha256.into(),
        }
    }

    #[test]
    fn is_installed_accepts_verified_cache() {
        let dir = std::env::temp_dir().join(format!("yap-installed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("m.gguf");
        let tokenizer = dir.join("tokenizer.bin");
        let punc = dir.join("punc.gguf");
        std::fs::write(&model, b"hello").unwrap();
        std::fs::write(&tokenizer, b"hello").unwrap();
        std::fs::write(&punc, b"hello").unwrap();
        let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        write_verified_marker(&model, hello).unwrap();
        write_verified_marker(&tokenizer, hello).unwrap();
        write_verified_marker(&punc, hello).unwrap();
        let pin = sample_pin(hello);
        std::env::set_var("YAP_MODELS_DIR", &dir);
        assert!(is_installed(&pin));
        std::env::remove_var("YAP_MODELS_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_installed_rejects_missing_model() {
        let dir = std::env::temp_dir().join(format!("yap-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pin = sample_pin(&"0".repeat(64));
        std::env::set_var("YAP_MODELS_DIR", &dir);
        assert!(!is_installed(&pin));
        std::env::remove_var("YAP_MODELS_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_model_downloads_then_verifies() {
        let dir = std::env::temp_dir().join(format!("yap-dl-ok-{}", std::process::id()));
        let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let pin = sample_pin(hello);
        let dest = ensure_model_at(&dir, &pin, |_url, path| {
            std::fs::write(path, b"hello").map_err(|_| SttError::ModelMissing)
        })
        .unwrap();
        assert!(dest.exists());
        assert!(dir.join("tokenizer.bin").exists());
        assert!(dir.join("punc.gguf").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_model_rejects_corrupt_download() {
        let dir = std::env::temp_dir().join(format!("yap-dl-bad-{}", std::process::id()));
        let pin = sample_pin(&"0".repeat(64));
        let err = ensure_model_at(&dir, &pin, |_url, path| {
            std::fs::write(path, b"tampered").map_err(|_| SttError::ModelMissing)
        })
        .unwrap_err();
        assert_eq!(err, SttError::ModelCorrupt);
        assert!(!dir.join("m.gguf").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_model_uses_valid_cache_without_downloading() {
        let dir = std::env::temp_dir().join(format!("yap-dl-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("m.gguf"), b"hello").unwrap();
        std::fs::write(dir.join("tokenizer.bin"), b"hello").unwrap();
        std::fs::write(dir.join("punc.gguf"), b"hello").unwrap();
        let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let pin = sample_pin(hello);
        let dest = ensure_model_at(&dir, &pin, |_url, _path| {
            panic!("download must not run when a valid cache exists")
        })
        .unwrap();
        assert!(dest.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_model_deletes_corrupt_cache_and_redownloads() {
        let dir = std::env::temp_dir().join(format!("yap-dl-corrupt-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("m.gguf"), b"tampered-on-disk").unwrap();
        let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let pin = sample_pin(hello);
        let mut download_calls = 0;
        let dest = ensure_model_at(&dir, &pin, |_url, path| {
            download_calls += 1;
            std::fs::write(path, b"hello").map_err(|_| SttError::ModelMissing)
        })
        .unwrap();
        assert_eq!(download_calls, 3, "must re-download corrupt model and missing companions");
        assert!(dest.exists());
        assert!(dir.join("tokenizer.bin").exists());
        assert!(dir.join("punc.gguf").exists());
        assert!(verify_sha256(&dest, hello).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn trusted_cache_skips_full_hash_on_repeat() {
        let dir = std::env::temp_dir().join(format!("yap-trusted-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("m.gguf");
        std::fs::write(&model, b"hello").unwrap();
        let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        write_verified_marker(&model, hello).unwrap();
        assert!(trusted_cache(&model, hello));
        std::fs::remove_dir_all(&dir).ok();
    }
}
