use std::path::{Path, PathBuf};

use crate::stt::error::SttError;

pub trait SttBackend {
    fn transcribe(&self, audio: &Path, language: &str) -> Result<String, SttError>;

    fn transcribe_batch(&self, files: &[PathBuf], language: &str) -> Vec<Result<String, SttError>> {
        files.iter().map(|file| self.transcribe(file, language)).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendChoice {
    Crispasr,
    Python,
    PreferCrispasr,
}

pub fn select_backend(value: Option<&str>) -> BackendChoice {
    match value {
        Some("crispasr") => BackendChoice::Crispasr,
        Some("python") => BackendChoice::Python,
        Some(_) | None => BackendChoice::PreferCrispasr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    struct FakeBackend;
    impl SttBackend for FakeBackend {
        fn transcribe(&self, _audio: &Path, _language: &str) -> Result<String, crate::stt::error::SttError> {
            Ok("hi".to_string())
        }
    }

    #[test]
    fn selects_backend_from_env_value() {
        assert_eq!(select_backend(Some("crispasr")), BackendChoice::Crispasr);
        assert_eq!(select_backend(Some("python")), BackendChoice::Python);
        assert_eq!(select_backend(None), BackendChoice::PreferCrispasr);
        assert_eq!(select_backend(Some("bogus")), BackendChoice::PreferCrispasr);
    }

    #[test]
    fn transcribe_batch_defaults_to_per_file_loop() {
        let backend = FakeBackend;
        let files = vec![PathBuf::from("a.wav"), PathBuf::from("b.wav")];
        let out = backend.transcribe_batch(&files, "en");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].as_ref().unwrap(), "hi");
    }
}
