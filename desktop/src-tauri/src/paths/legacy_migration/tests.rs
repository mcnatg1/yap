mod lifecycle;
mod recovery;
mod security;

use super::*;
use super::{
    platform::{open_migration_lock, open_regular_file_read, rename_no_replace, DirectoryLease},
    recovery::cleanup_after_error_with,
    secure_tree::{copy_tree_verified, trees_equal, validate_regular_tree_with_limits, TreeLimits},
};
use std::sync::{Arc, Barrier};

fn test_root(case: u8) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yap-app-data-migration-{}-{case}",
        std::process::id()
    ))
}

#[cfg(unix)]
fn create_file_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}

#[cfg(windows)]
fn create_file_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(source, destination)
}

fn test_symlink_is_unavailable(error: &io::Error) -> bool {
    cfg!(windows)
        && (error.kind() == io::ErrorKind::PermissionDenied || error.raw_os_error() == Some(1314))
}
