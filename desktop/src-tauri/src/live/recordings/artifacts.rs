use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::audio::recording;

use super::transcripts::{
    has_valid_transcript_revision, highest_transcript_revision, is_transcript_path,
};

pub(super) fn committed_session_output_path(
    dir: &Path,
    committed: &recording::CommittedCapture,
) -> PathBuf {
    let transcript = dir.join(format!("live-{}.txt", committed.manifest.session_id));
    if has_valid_transcript_revision(
        dir,
        &committed.manifest.session_id,
        &committed.manifest.capture_sidecar_sha256,
    ) && recording::is_regular_artifact(&transcript)
    {
        transcript
    } else {
        dir.join(&committed.manifest.audio_file)
    }
}

pub(crate) fn canonical_committed_live_path_from_dir(
    requested: &Path,
    owned_dir: &Path,
    require_transcript: bool,
) -> Result<PathBuf, String> {
    let owned_dir = owned_dir
        .canonicalize()
        .map_err(|_| "Yap recordings directory is unavailable.".to_string())?;
    let path = requested
        .canonicalize()
        .map_err(|_| "Yap recording is unavailable.".to_string())?;
    if path.parent() != Some(owned_dir.as_path()) || !recording::is_regular_artifact(&path) {
        return Err("Yap recording is not a canonical committed session artifact.".into());
    }
    let scan = recording::scan_recordings(&owned_dir)?;
    for capture in scan.complete {
        let session_id = &capture.manifest.session_id;
        let audio = owned_dir.join(&capture.manifest.audio_file);
        let text = owned_dir.join(format!("live-{session_id}.txt"));
        if path == audio && !require_transcript {
            return Ok(path);
        }
        if path == text
            && has_valid_transcript_revision(
                &owned_dir,
                session_id,
                &capture.manifest.capture_sidecar_sha256,
            )
        {
            return Ok(path);
        }
    }
    Err("Yap recording is not a canonical committed session artifact.".into())
}

pub(crate) fn open_committed_live_transcript_from_dir(
    requested: &Path,
    owned_dir: &Path,
) -> Result<std::fs::File, String> {
    let owned_dir = owned_dir
        .canonicalize()
        .map_err(|_| "Yap recordings directory is unavailable.".to_string())?;
    let parent = requested
        .parent()
        .ok_or_else(|| "Yap recording is unavailable.".to_string())?
        .canonicalize()
        .map_err(|_| "Yap recording is unavailable.".to_string())?;
    let name = requested
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Yap recording is unavailable.".to_string())?;
    if parent != owned_dir || !is_transcript_path(Path::new(name)) {
        return Err("Yap recording is not a canonical committed session artifact.".into());
    }
    let mut file = recording::open_regular_artifact(&owned_dir, name)?;
    let handle_sha256 = recording::sha256_open_regular_file(&mut file)?;
    let scan = recording::scan_recordings(&owned_dir)?;
    for capture in scan.complete {
        let session_id = &capture.manifest.session_id;
        if name != format!("live-{session_id}.txt")
            || !has_valid_transcript_revision(
                &owned_dir,
                session_id,
                &capture.manifest.capture_sidecar_sha256,
            )
        {
            continue;
        }
        let Some(expected_sha256) = validated_transcript_sha256(&owned_dir, session_id) else {
            continue;
        };
        if expected_sha256 == handle_sha256 {
            file.seek(SeekFrom::Start(0))
                .map_err(|error| format!("Failed to rewind validated Yap transcript: {error}"))?;
            return Ok(file);
        }
        return Err("Yap transcript changed after validation.".into());
    }
    Err("Yap recording is not a canonical committed session artifact.".into())
}

fn validated_transcript_sha256(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Option<String> {
    let highest = highest_transcript_revision(dir, session_id)?;
    let revision_name = format!("live-{session_id}.transcript.r{highest}.json");
    let (text, _) = recording::read_and_hash_regular_artifact(dir, &revision_name).ok()?;
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()?
        .get("textSha256")?
        .as_str()
        .map(str::to_string)
}
