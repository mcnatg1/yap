use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::error::ConfigError;
use super::platform::{
    atomic_replace_same_directory, file_identity, open_settings_lock, sync_parent_directory,
    FileIdentity, SettingsFileLock,
};

static NEXT_SETTINGS_ARTIFACT: AtomicU64 = AtomicU64::new(0);

pub(super) fn acquire_settings_lock(path: &Path) -> Result<SettingsFileLock, ConfigError> {
    open_settings_lock(path).map_err(ConfigError::SaveIo)
}

pub(super) fn acquire_settings_access_lock(path: &Path) -> Result<SettingsFileLock, ConfigError> {
    open_settings_lock(path).map_err(ConfigError::AccessIo)
}

#[cfg(test)]
pub(super) fn write_atomically_with_before_publish<F>(
    path: &Path,
    contents: &[u8],
    before_publish: F,
) -> Result<(), ConfigError>
where
    F: FnOnce(&Path, &Path) -> std::io::Result<()>,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(ConfigError::SaveIo)?;
    }
    let _lock = acquire_settings_lock(path)?;
    write_atomically_locked_with_hooks(path, contents, before_publish, |_| Ok(()))
}

pub(super) fn write_atomically_locked_with_hooks<BeforePublish, AfterPublish>(
    path: &Path,
    contents: &[u8],
    before_publish: BeforePublish,
    after_publish: AfterPublish,
) -> Result<(), ConfigError>
where
    BeforePublish: FnOnce(&Path, &Path) -> std::io::Result<()>,
    AfterPublish: FnOnce(&Path) -> std::io::Result<()>,
{
    let legacy_partial = path.with_extension("json.part");
    match std::fs::remove_file(&legacy_partial) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(ConfigError::SaveIo(error)),
    }
    scavenge_abandoned_unique_partials(path).map_err(ConfigError::SaveIo)?;

    let (partial, mut file) = reserve_unique_partial(path).map_err(ConfigError::SaveIo)?;

    let staging = (|| -> std::io::Result<()> {
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        Ok(())
    })();

    if let Err(error) = staging {
        std::fs::remove_file(&partial).ok();
        return Err(ConfigError::SaveIo(error));
    }

    let destination_before = match snapshot_destination(path) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            std::fs::remove_file(&partial).ok();
            return Err(ConfigError::SaveIo(error));
        }
    };
    let publication =
        before_publish(&partial, path).and_then(|_| atomic_replace_same_directory(&partial, path));
    if let Err(error) = publication {
        return Err(reconcile_publication_failure(
            path,
            &partial,
            contents,
            &destination_before,
            error,
        ));
    }
    if let Err(error) = after_publish(path).and_then(|_| sync_parent_directory(path)) {
        return Err(ConfigError::PublishedButDurabilityUnconfirmed(error));
    }
    Ok(())
}

pub(super) fn reserve_unique_partial(path: &Path) -> std::io::Result<(PathBuf, std::fs::File)> {
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "settings path has no file name",
        )
    })?;
    for _ in 0..64 {
        let counter = NEXT_SETTINGS_ARTIFACT.fetch_add(1, Ordering::Relaxed);
        let mut partial_name = file_name.to_os_string();
        partial_name.push(format!(".{}.{counter}.part", std::process::id()));
        let partial = path.with_file_name(partial_name);
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&partial)
        {
            Ok(file) => return Ok((partial, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not reserve unique server settings partial",
    ))
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DestinationSnapshot {
    Missing,
    Present {
        identity: Option<FileIdentity>,
        bytes: Vec<u8>,
    },
}

pub(super) fn snapshot_destination(path: &Path) -> std::io::Result<DestinationSnapshot> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DestinationSnapshot::Missing);
        }
        Err(error) => return Err(error),
    };
    let identity = file_identity(&file)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(DestinationSnapshot::Present { identity, bytes })
}

fn destination_is_proven_unchanged(
    before: &DestinationSnapshot,
    after: &DestinationSnapshot,
) -> bool {
    match (before, after) {
        (DestinationSnapshot::Missing, DestinationSnapshot::Missing) => true,
        (
            DestinationSnapshot::Present {
                identity: Some(before_identity),
                bytes: before_bytes,
            },
            DestinationSnapshot::Present {
                identity: Some(after_identity),
                bytes: after_bytes,
            },
        ) => before_identity == after_identity && before_bytes == after_bytes,
        _ => false,
    }
}

fn destination_contains(after: &DestinationSnapshot, intended: &[u8]) -> bool {
    matches!(
        after,
        DestinationSnapshot::Present { bytes, .. } if bytes == intended
    )
}

fn reconcile_publication_failure(
    path: &Path,
    partial: &Path,
    intended: &[u8],
    before: &DestinationSnapshot,
    publish_error: std::io::Error,
) -> ConfigError {
    reconcile_publication_failure_with_parent_sync(
        path,
        partial,
        intended,
        before,
        publish_error,
        sync_parent_directory,
    )
}

pub(super) fn reconcile_publication_failure_with_parent_sync<ParentSync>(
    path: &Path,
    partial: &Path,
    intended: &[u8],
    before: &DestinationSnapshot,
    publish_error: std::io::Error,
    parent_sync: ParentSync,
) -> ConfigError
where
    ParentSync: Fn(&Path) -> std::io::Result<()>,
{
    let after = snapshot_destination(path);
    if after
        .as_ref()
        .is_ok_and(|after| destination_is_proven_unchanged(before, after))
    {
        std::fs::remove_file(partial).ok();
        return ConfigError::SaveIo(publish_error);
    }

    let visible = after
        .as_ref()
        .is_ok_and(|after| destination_contains(after, intended));
    let recovery_path = match preserve_recovery_artifact(path, partial, intended, parent_sync) {
        Ok(path) => Some(path),
        Err(recovery_error) => {
            let source = std::io::Error::other(format!(
                "{publish_error}; recovery preservation failed: {recovery_error}"
            ));
            return if visible {
                ConfigError::PublicationFailedAfterVisibleChange {
                    source,
                    recovery_path: None,
                }
            } else {
                ConfigError::PublicationStateIndeterminate {
                    source,
                    recovery_path: None,
                }
            };
        }
    };
    if visible {
        ConfigError::PublicationFailedAfterVisibleChange {
            source: publish_error,
            recovery_path,
        }
    } else {
        let source = match after {
            Ok(_) => publish_error,
            Err(observation_error) => std::io::Error::other(format!(
                "{publish_error}; could not verify destination state: {observation_error}"
            )),
        };
        ConfigError::PublicationStateIndeterminate {
            source,
            recovery_path,
        }
    }
}

fn preserve_recovery_artifact<ParentSync>(
    path: &Path,
    partial: &Path,
    contents: &[u8],
    parent_sync: ParentSync,
) -> std::io::Result<PathBuf>
where
    ParentSync: Fn(&Path) -> std::io::Result<()>,
{
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "settings path has no file name",
        )
    })?;
    for _ in 0..64 {
        let counter = NEXT_SETTINGS_ARTIFACT.fetch_add(1, Ordering::Relaxed);
        let mut recovery_name = file_name.to_os_string();
        recovery_name.push(format!(".recovery.{}.{counter}.json", std::process::id()));
        let recovery = path.with_file_name(recovery_name);
        let mut file = match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&recovery)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        if let Err(error) = (|| {
            file.write_all(contents)?;
            file.flush()?;
            file.sync_all()
        })() {
            drop(file);
            std::fs::remove_file(&recovery).ok();
            return Err(error);
        }
        drop(file);
        parent_sync(&recovery)?;
        std::fs::remove_file(partial).ok();
        return Ok(recovery);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not reserve server settings recovery artifact",
    ))
}

fn scavenge_abandoned_unique_partials(path: &Path) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "settings path has no parent",
        )
    })?;
    let base_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "settings path has no UTF-8 file name",
            )
        })?;
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let Some(candidate) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_unique_partial_name(base_name, &candidate) || !entry.file_type()?.is_file() {
            continue;
        }
        std::fs::remove_file(entry.path())?;
    }
    Ok(())
}

fn is_unique_partial_name(base_name: &str, candidate: &str) -> bool {
    let Some(identity) = candidate
        .strip_prefix(base_name)
        .and_then(|rest| rest.strip_prefix('.'))
        .and_then(|rest| rest.strip_suffix(".part"))
    else {
        return false;
    };
    let mut parts = identity.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(pid), Some(counter), None)
            if pid.parse::<u32>().is_ok() && counter.parse::<u64>().is_ok()
    )
}
