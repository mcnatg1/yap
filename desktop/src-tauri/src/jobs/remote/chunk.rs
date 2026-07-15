use super::{
    artifact_io::{metadata_is_link_or_reparse, open_no_follow_read, sha256_bytes},
    spool::prepare_spool_root,
};
use crate::server_connector::batch::CaptureChunkReference;
use std::{fs, io::Read, path::Path};

pub(in crate::jobs) fn read_prepared_chunk(
    artifact_path: &Path,
    spool_root: &Path,
    reference: &CaptureChunkReference,
) -> Result<Vec<u8>, String> {
    prepare_spool_root(spool_root)?;
    if !artifact_path.is_absolute() || !spool_root.is_absolute() {
        return Err("prepared chunk paths must be absolute".into());
    }
    let relative = artifact_path
        .strip_prefix(spool_root)
        .map_err(|_| "prepared chunk is outside the owned spool".to_string())?;
    let components = relative.components().collect::<Vec<_>>();
    if components.len() != 2
        || components
            .iter()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("prepared chunk path has an invalid owned shape".into());
    }
    let parent = artifact_path
        .parent()
        .ok_or_else(|| "prepared chunk has no parent directory".to_string())?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|error| format!("failed to inspect prepared chunk directory: {error}"))?;
    if !parent_metadata.is_dir() || metadata_is_link_or_reparse(&parent_metadata) {
        return Err("prepared chunk directory is not a safe owned directory".into());
    }
    let path_metadata = fs::symlink_metadata(artifact_path)
        .map_err(|error| format!("failed to inspect prepared chunk: {error}"))?;
    if !path_metadata.is_file() || metadata_is_link_or_reparse(&path_metadata) {
        return Err("prepared chunk must be a regular non-link file".into());
    }
    let expected_length = usize::try_from(reference.content_identity.byte_length)
        .map_err(|_| "prepared chunk length is out of range".to_string())?;
    if expected_length == 0 || expected_length > 1024 * 1024 {
        return Err("prepared chunk length is outside the server contract".into());
    }
    let mut file = open_no_follow_read(artifact_path)
        .map_err(|error| format!("failed to open prepared chunk: {error}"))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("failed to inspect opened prepared chunk: {error}"))?;
    if !opened_metadata.is_file()
        || metadata_is_link_or_reparse(&opened_metadata)
        || opened_metadata.len() != reference.content_identity.byte_length
    {
        return Err("opened prepared chunk differs from its immutable declaration".into());
    }
    let mut body = Vec::with_capacity(expected_length);
    file.read_to_end(&mut body)
        .map_err(|error| format!("failed to read prepared chunk: {error}"))?;
    if body.len() != expected_length || sha256_bytes(&body) != reference.content_identity.sha256 {
        return Err("prepared chunk differs from its immutable content identity".into());
    }
    Ok(body)
}
