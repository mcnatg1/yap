use std::io::Write;

use crate::audio::recording::{self, PublishedTranscriptReceipt};
use crate::live;

pub(crate) fn transcript_text(view: &live::state::LiveSessionView) -> Option<String> {
    view.final_text
        .as_deref()
        .or(view.partial_text.as_deref())
        .map(clean_transcript_text)
        .filter(|text| !text.is_empty())
}

pub(crate) fn completed_transcript_text(view: &live::state::LiveSessionView) -> Option<String> {
    view.final_text
        .as_deref()
        .map(clean_transcript_text)
        .filter(|text| !text.is_empty())
}

pub(in crate::live::recordings) fn clean_transcript_text(text: &str) -> String {
    if text.trim() == "No live transcript captured." {
        return "Transcript unavailable for this live recording.".into();
    }

    let mut cleaned = text
        .split_whitespace()
        .map(fix_word_casing)
        .collect::<Vec<_>>()
        .join(" ");
    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", ".");
    }
    cleaned
}

pub(in crate::live::recordings) fn fix_word_casing(word: &str) -> String {
    let mut chars = word.chars();
    let (Some(first), Some(second), Some(third)) = (chars.next(), chars.next(), chars.next())
    else {
        return word.to_string();
    };

    if first.is_uppercase() && second.is_uppercase() && third.is_lowercase() {
        let mut fixed = String::new();
        fixed.push(first);
        fixed.extend(second.to_lowercase());
        fixed.push(third);
        fixed.extend(chars);
        fixed
    } else {
        word.to_string()
    }
}

pub(in crate::live::recordings) fn write_new_text_file(
    path: &std::path::Path,
    text: &str,
) -> Result<PublishedTranscriptReceipt, String> {
    write_new_text_file_with(
        path,
        text,
        |file| file.sync_all(),
        |from, to, owned| recording::publish_no_replace(from, to, owned, "publish live transcript"),
    )
}

pub(in crate::live::recordings) fn write_new_text_file_with<S, R>(
    path: &std::path::Path,
    text: &str,
    sync: S,
    rename: R,
) -> Result<PublishedTranscriptReceipt, String>
where
    S: FnOnce(&std::fs::File) -> std::io::Result<()>,
    R: FnOnce(&std::path::Path, &std::path::Path, &std::fs::File) -> Result<std::fs::File, String>,
{
    if path.exists() {
        return Err("live transcript already exists".into());
    }
    let partial = partial_text_path(path).map_err(|error| error.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|error| error.to_string())?;
    let result = file
        .write_all(text.as_bytes())
        .and_then(|_| sync(&file))
        .map_err(|error| error.to_string());
    let result = result.and_then(|_| {
        if path.exists() {
            return Err("live transcript already exists".into());
        }
        rename(&partial, path, &file)
    });
    let published = match result {
        Ok(published) => published,
        Err(error) => {
            recording::remove_owned_staging(&partial, &file, "publish live transcript");
            return Err(error);
        }
    };
    drop(file);
    PublishedTranscriptReceipt::from_verified_destination(path, published)
}

pub(in crate::live::recordings) fn partial_text_path(
    path: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "missing transcript file name",
            )
        })?;
    Ok(path.with_file_name(format!("{file_name}.part")))
}
