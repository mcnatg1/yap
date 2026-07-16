use std::{collections::HashSet, path::Path};

pub(crate) fn stable_existing_path_string(path: &std::path::Path) -> String {
    stable_path_string(path)
}

#[cfg(target_os = "windows")]
pub(in crate::live::recordings) fn stable_path_string(path: &std::path::Path) -> String {
    let display = path.display().to_string();
    if let Some(unc) = display.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    display
        .strip_prefix(r"\\?\")
        .unwrap_or(&display)
        .to_string()
}

#[cfg(not(target_os = "windows"))]
pub(in crate::live::recordings) fn stable_path_string(path: &std::path::Path) -> String {
    path.display().to_string()
}

pub(crate) fn is_primary_live_transcript_path(path: &std::path::Path) -> bool {
    is_transcript_path(path)
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.starts_with("live-s-"))
}

pub(crate) fn is_transcript_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
}

pub(in crate::live::recordings) fn transcript_artifact_names(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<HashSet<String>, String> {
    let text = format!("live-{session_id}.txt");
    let text_partial = format!("{text}.part");
    let revision_prefix = format!("live-{session_id}.transcript.r");
    let names = std::fs::read_dir(dir)
        .map_err(|error| format!("Failed to read transcript artifacts: {error}"))?
        .map(|entry| entry.map_err(|error| format!("Failed to read transcript artifact: {error}")))
        .filter_map(|entry| match entry {
            Ok(entry) => entry.file_name().to_str().map(str::to_owned),
            Err(_) => None,
        })
        .filter(|name| {
            name == &text
                || name == &text_partial
                || (name.starts_with(&revision_prefix)
                    && (name.ends_with(".json") || name.ends_with(".json.part")))
        })
        .collect::<HashSet<_>>();
    Ok(names)
}

pub(crate) fn unix_millis_now() -> Result<u64, String> {
    system_time_to_unix_millis(std::time::SystemTime::now())
        .ok_or_else(|| "System clock error: timestamp out of range.".to_string())
}

pub(in crate::live::recordings) fn system_time_to_unix_millis(
    time: std::time::SystemTime,
) -> Option<u64> {
    let millis = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    u64::try_from(millis).ok()
}
