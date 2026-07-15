mod download;
mod integrity;
mod operation;
mod progress;
mod temp;

use std::{io::ErrorKind, path::PathBuf};

use crate::stt::error::SttError;

pub use download::{download_verified_file, DownloadRequest};
pub use integrity::{sha256_file, verify_sha256};
pub use operation::DownloadOperation;
pub use progress::DownloadProgress;
pub(crate) use temp::{cleanup_stale_download_temps, write_text_atomically};

pub fn models_dir_from<F>(env: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = crate::paths::absolute_env_path(&env, "YAP_MODELS_DIR") {
        return dir;
    }
    crate::paths::app_data_dir_from(env).join("models")
}

pub fn models_dir() -> PathBuf {
    if let Some(dir) =
        crate::paths::absolute_env_path(&|key| std::env::var(key).ok(), "YAP_MODELS_DIR")
    {
        return dir;
    }
    crate::paths::app_data_dir().join("models")
}

pub fn hf_resolve_url(repo: &str, revision: &str, file: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/{revision}/{file}")
}

pub(super) fn reqwest_error_to_stt(error: reqwest::Error) -> SttError {
    if error.is_timeout() {
        SttError::Timeout
    } else {
        SttError::ModelMissing
    }
}

pub(super) fn io_error_to_stt(error: std::io::Error) -> SttError {
    if error.kind() == ErrorKind::TimedOut {
        SttError::Timeout
    } else {
        SttError::ModelMissing
    }
}

#[cfg(test)]
mod tests;
