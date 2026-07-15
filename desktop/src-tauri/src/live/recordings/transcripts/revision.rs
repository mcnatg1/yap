use std::io::Write;

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::audio::evidence::ModelRevision;
use crate::audio::recording::{self, PublishedTranscriptReceipt};
use crate::audio::results::{ResultAuthority, ResultStatus, TranscriptResultRevision};

pub(in crate::live::recordings) fn write_transcript_revision(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
    capture_sidecar_sha256: &str,
    transcript_receipt: &PublishedTranscriptReceipt,
    transcript: &str,
    status: ResultStatus,
) -> Result<(), String> {
    write_transcript_revision_with_barrier(
        dir,
        session_id,
        capture_sidecar_sha256,
        transcript_receipt,
        transcript,
        status,
        |_| {},
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::live::recordings) enum TranscriptRevisionPublicationBarrier {
    BeforePublication,
    AfterPublication,
}

pub(in crate::live::recordings) fn write_transcript_revision_with_barrier<F>(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
    capture_sidecar_sha256: &str,
    transcript_receipt: &PublishedTranscriptReceipt,
    transcript: &str,
    status: ResultStatus,
    mut publication_barrier: F,
) -> Result<(), String>
where
    F: FnMut(TranscriptRevisionPublicationBarrier),
{
    let revision = next_transcript_revision(dir, session_id)?;
    let model = ModelRevision::new(crate::stt::nemotron::MODEL_ID, "local", "local")
        .map_err(|error| format!("Failed to describe local transcript model: {error}"))?;
    let result = if revision == 1 {
        TranscriptResultRevision::new(
            session_id.clone(),
            revision,
            ResultAuthority::LocalProvisional,
            capture_sidecar_sha256,
            None,
            status,
            transcript,
            Vec::new(),
            vec![model],
        )
    } else {
        let previous_name = format!("live-{session_id}.transcript.r{}.json", revision - 1);
        let (previous_text, previous_sha256) =
            recording::read_and_hash_regular_artifact(dir, &previous_name)
                .map_err(|error| format!("Failed to read prior transcript revision: {error}"))?;
        let previous: TranscriptResultRevision = serde_json::from_str(&previous_text)
            .map_err(|error| format!("Failed to parse prior transcript revision: {error}"))?;
        previous.next_revision(
            revision,
            ResultAuthority::LocalProvisional,
            capture_sidecar_sha256,
            previous_sha256,
            status,
            transcript,
            Vec::new(),
            vec![model],
        )
    }
    .map_err(|error| format!("Failed to build transcript revision: {error}"))?;
    let path = transcript_revision_path(dir, session_id, revision);
    let serialized = transcript_result_value(&result, capture_sidecar_sha256, transcript_receipt)?;
    let (staging, mut file) = create_unique_transcript_revision_staging(dir, session_id, revision)?;
    let result = (|| {
        serde_json::to_writer(&mut file, &serialized)
            .map_err(|error| format!("Failed to write transcript revision: {error}"))?;
        file.write_all(b"\n")
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("Failed to finalize transcript revision staging: {error}"))?;

        publication_barrier(TranscriptRevisionPublicationBarrier::BeforePublication);
        transcript_receipt.revalidate()?;
        let published =
            recording::publish_no_replace(&staging, &path, &file, "publish transcript revision")?;
        drop(published);

        publication_barrier(TranscriptRevisionPublicationBarrier::AfterPublication);
        // A replacement after this check is external post-completion tamper; consumers still
        // hash-check the transcript against this immutable revision before selecting it.
        transcript_receipt.revalidate()
    })();
    if result.is_err() {
        recording::remove_owned_staging(&staging, &file, "publish transcript revision");
    }
    drop(file);
    result
}

pub(in crate::live::recordings) fn create_unique_transcript_revision_staging(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
    revision: u64,
) -> Result<(std::path::PathBuf, std::fs::File), String> {
    for nonce in 0..128_u64 {
        let path = dir.join(format!(
            "live-{session_id}.transcript.r{revision}.json.part-{}-{nonce}",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Failed to create transcript revision staging file: {error}"
                ));
            }
        }
    }
    Err("Failed to allocate a unique transcript revision staging file".into())
}

pub(in crate::live::recordings) fn has_valid_transcript_revision(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
    capture_sidecar_sha256: &str,
) -> bool {
    let text_name = format!("live-{session_id}.txt");
    let revision_prefix = format!("live-{session_id}.transcript.r");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let highest = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str().map(str::to_owned)?;
            name.strip_prefix(&revision_prefix)
                .and_then(|value| value.strip_suffix(".json"))
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|revision| *revision > 0)
        })
        .max();
    let Some(highest) = highest else {
        return false;
    };
    transcript_revision_chain_matches_receipt(
        dir,
        session_id,
        highest,
        &text_name,
        capture_sidecar_sha256,
    )
}

pub(in crate::live::recordings) fn transcript_revision_chain_matches_receipt(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
    highest_revision: u64,
    text_name: &str,
    capture_sidecar_sha256: &str,
) -> bool {
    let Ok((_, current_text_sha256)) = recording::read_and_hash_regular_artifact(dir, text_name)
    else {
        return false;
    };
    let mut previous_hash = None;
    for revision in 1..=highest_revision {
        let revision_name = format!("live-{session_id}.transcript.r{revision}.json");
        let Ok((revision_text, revision_hash)) =
            recording::read_and_hash_regular_artifact(dir, &revision_name)
        else {
            return false;
        };
        if serde_json::from_str::<TranscriptResultRevision>(&revision_text).is_err() {
            return false;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&revision_text) else {
            return false;
        };
        let Some(object) = value.as_object() else {
            return false;
        };
        let previous_matches = match previous_hash.as_deref() {
            Some(previous_hash) => {
                object
                    .get("previousResultSha256")
                    .and_then(|value| value.as_str())
                    == Some(previous_hash)
            }
            None => {
                object.get("previousResultSha256").is_none()
                    || object
                        .get("previousResultSha256")
                        .is_some_and(serde_json::Value::is_null)
            }
        };
        if object.get("textFile").and_then(|value| value.as_str()) != Some(text_name)
            || object.get("textSha256").and_then(|value| value.as_str())
                != Some(current_text_sha256.as_str())
            || object
                .get("captureSidecarSha256")
                .and_then(|value| value.as_str())
                != Some(capture_sidecar_sha256)
            || object.get("sessionId").and_then(|value| value.as_str()) != Some(session_id.as_str())
            || object.get("revision").and_then(|value| value.as_u64()) != Some(revision)
            || !previous_matches
        {
            return false;
        }
        previous_hash = Some(revision_hash);
    }
    true
}

pub(in crate::live::recordings) fn transcript_result_value(
    result: &TranscriptResultRevision,
    capture_sidecar_sha256: &str,
    transcript_receipt: &PublishedTranscriptReceipt,
) -> Result<serde_json::Value, String> {
    let mut value = serde_json::to_value(result)
        .map_err(|error| format!("Failed to serialize transcript revision: {error}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "Transcript revision did not serialize as an object".to_string())?;
    object.insert("schemaVersion".into(), serde_json::Value::from(1u16));
    object.insert(
        "textFile".into(),
        serde_json::Value::from(transcript_receipt.file_name()),
    );
    object.insert(
        "textSha256".into(),
        serde_json::Value::from(transcript_receipt.sha256()),
    );
    object.insert(
        "modelId".into(),
        serde_json::Value::from(crate::stt::nemotron::MODEL_ID),
    );
    object.insert("modelRevision".into(), serde_json::Value::from("local"));
    object.insert(
        "createdAtUtc".into(),
        serde_json::Value::from(
            OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(|_| "Failed to format transcript revision time")?,
        ),
    );
    object.insert(
        "captureSidecarSha256".into(),
        serde_json::Value::from(capture_sidecar_sha256),
    );
    Ok(value)
}

pub(in crate::live::recordings) fn next_transcript_revision(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<u64, String> {
    let prefix = format!("live-{session_id}.transcript.r");
    let mut highest = 0;
    for entry in std::fs::read_dir(dir)
        .map_err(|error| format!("Failed to read transcript revisions: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("Failed to read transcript revision: {error}"))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if let Some(revision) = name
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".json"))
            .and_then(|value| value.parse::<u64>().ok())
        {
            highest = highest.max(revision);
        }
    }
    highest
        .checked_add(1)
        .ok_or_else(|| "Transcript revision overflowed".into())
}

pub(in crate::live::recordings) fn highest_transcript_revision(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
) -> Option<u64> {
    let prefix = format!("live-{session_id}.transcript.r");
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter_map(|name| {
            name.strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix(".json"))
                .and_then(|value| value.parse::<u64>().ok())
        })
        .filter(|revision| *revision > 0)
        .max()
}

pub(in crate::live::recordings) fn transcript_revision_path(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
    revision: u64,
) -> std::path::PathBuf {
    dir.join(format!("live-{session_id}.transcript.r{revision}.json"))
}
