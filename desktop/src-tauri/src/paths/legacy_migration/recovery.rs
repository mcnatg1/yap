use std::{
    fs, io,
    path::{Path, PathBuf},
};

use super::{
    is_recognized_legacy_runtime_entry,
    platform::DirectoryLease,
    secure_tree::{metadata_if_present, trees_equal, validate_regular_tree},
    MIGRATION_RETIREMENT_PREFIX, MIGRATION_STAGE_PREFIX,
};

pub(super) fn recover_migration_residue(legacy: &Path, canonical: &Path) -> io::Result<()> {
    recover_staging_residue(legacy, canonical)?;
    recover_retirement_residue(legacy, canonical)
}

fn recover_staging_residue(legacy: &Path, canonical: &Path) -> io::Result<()> {
    for residue in migration_residue_directories(canonical, MIGRATION_STAGE_PREFIX)? {
        validate_regular_tree(&residue)?;
        for staged in sorted_entries(&residue)? {
            let name = staged.file_name().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("staging residue has no entry name: {}", staged.display()),
                )
            })?;
            if !is_recognized_legacy_runtime_entry(name) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "staging residue contains an unrecognized entry: {}",
                        staged.display()
                    ),
                ));
            }
            validate_regular_tree(&staged)?;
            let source = legacy.join(name);
            let destination = canonical.join(name);
            let mut verified_copy = false;
            for candidate in [&source, &destination] {
                if metadata_if_present(candidate)?.is_some() {
                    validate_regular_tree(candidate)?;
                    if !trees_equal(&staged, candidate)? {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "staging residue does not match {}: {}",
                                candidate.display(),
                                staged.display()
                            ),
                        ));
                    }
                    verified_copy = true;
                }
            }
            if !verified_copy {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "staging residue is the only remaining copy and was preserved: {}",
                        staged.display()
                    ),
                ));
            }
            remove_regular_tree(&staged)?;
        }
        fs::remove_dir(&residue)?;
    }
    Ok(())
}

fn recover_retirement_residue(legacy: &Path, canonical: &Path) -> io::Result<()> {
    for residue in migration_residue_directories(legacy, MIGRATION_RETIREMENT_PREFIX)? {
        validate_regular_tree(&residue)?;
        for retired in sorted_entries(&residue)? {
            let name = retired.file_name().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "retirement residue has no entry name: {}",
                        retired.display()
                    ),
                )
            })?;
            if !is_recognized_legacy_runtime_entry(name) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "retirement residue contains an unrecognized entry: {}",
                        retired.display()
                    ),
                ));
            }
            validate_regular_tree(&retired)?;
            let source = legacy.join(name);
            let destination = canonical.join(name);
            let source_exists = metadata_if_present(&source)?.is_some();
            let destination_exists = metadata_if_present(&destination)?.is_some();

            if source_exists {
                validate_regular_tree(&source)?;
                if !trees_equal(&retired, &source)? {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "retirement residue conflicts with restored legacy data: {}",
                            retired.display()
                        ),
                    ));
                }
            }
            if destination_exists {
                validate_regular_tree(&destination)?;
                if !trees_equal(&retired, &destination)? {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "retirement residue conflicts with canonical app data: {}",
                            retired.display()
                        ),
                    ));
                }
            }

            match (source_exists, destination_exists) {
                (false, false) => fs::rename(&retired, &source)?,
                (true, _) | (false, true) => remove_regular_tree(&retired)?,
            }
        }
        fs::remove_dir(&residue)?;
    }
    Ok(())
}

pub(super) fn migration_residue_directories(
    parent: &Path,
    prefix: &str,
) -> io::Result<Vec<PathBuf>> {
    if metadata_if_present(parent)?.is_none() {
        return Ok(Vec::new());
    }
    let directory = DirectoryLease::open(parent)?;
    let mut residues = directory
        .sorted_entry_names()?
        .into_iter()
        .filter(|name| name.to_str().is_some_and(|name| name.starts_with(prefix)))
        .map(|name| parent.join(name))
        .collect::<Vec<_>>();
    residues.sort();
    Ok(residues)
}

fn sorted_entries(path: &Path) -> io::Result<Vec<PathBuf>> {
    let directory = DirectoryLease::open(path)?;
    Ok(directory
        .sorted_entry_names()?
        .into_iter()
        .map(|name| path.join(name))
        .collect())
}

fn remove_regular_tree(path: &Path) -> io::Result<()> {
    validate_regular_tree(path)?;
    if fs::symlink_metadata(path)?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

pub(super) fn cleanup_after_error(path: &Path, error: io::Error) -> io::Error {
    cleanup_after_error_with(path, error, |path| fs::remove_dir_all(path))
}

pub(super) fn cleanup_after_error_with<F>(path: &Path, error: io::Error, cleanup: F) -> io::Error
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    match cleanup(path) {
        Ok(()) => error,
        Err(cleanup_error) => io::Error::new(
            error.kind(),
            format!(
                "{error}; migration residue cleanup also failed for {}: {cleanup_error}",
                path.display()
            ),
        ),
    }
}
