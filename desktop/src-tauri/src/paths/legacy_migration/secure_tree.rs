use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

use super::platform::{open_regular_file_read, DirectoryLease};

const MAX_TREE_DEPTH: usize = 64;
const MAX_TREE_ENTRIES: u64 = 100_000;
const MAX_TREE_WORK_BYTES: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) struct TreeLimits {
    max_depth: usize,
    max_entries: u64,
    max_bytes: u64,
}

impl Default for TreeLimits {
    fn default() -> Self {
        Self {
            max_depth: MAX_TREE_DEPTH,
            max_entries: MAX_TREE_ENTRIES,
            max_bytes: MAX_TREE_WORK_BYTES,
        }
    }
}

#[cfg(test)]
impl TreeLimits {
    pub(super) fn bounded(max_depth: usize, max_entries: u64, max_bytes: u64) -> Self {
        Self {
            max_depth,
            max_entries,
            max_bytes,
        }
    }
}

struct TreeBudget {
    limits: TreeLimits,
    entries: u64,
    bytes: u64,
}

impl TreeBudget {
    fn new(limits: TreeLimits) -> Self {
        Self {
            limits,
            entries: 0,
            bytes: 0,
        }
    }

    fn visit(&mut self, depth: usize, bytes: u64) -> io::Result<()> {
        if depth > self.limits.max_depth {
            return Err(limit_error("depth"));
        }
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or_else(|| limit_error("entry count"))?;
        if self.entries > self.limits.max_entries {
            return Err(limit_error("entry count"));
        }
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| limit_error("byte count"))?;
        if self.bytes > self.limits.max_bytes {
            return Err(limit_error("byte count"));
        }
        Ok(())
    }
}

fn limit_error(limit: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("migration tree exceeds its {limit} safety limit"),
    )
}

enum OpenNode {
    File(File, fs::Metadata),
    Directory(DirectoryLease),
}

fn open_node(path: &Path) -> io::Result<OpenNode> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata_is_link_or_reparse(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "migration tree contains a link or reparse point: {}",
                path.display()
            ),
        ));
    }
    if metadata.is_file() {
        let file = open_regular_file_read(path)?;
        let metadata = file.metadata()?;
        return Ok(OpenNode::File(file, metadata));
    }
    if metadata.is_dir() {
        return DirectoryLease::open(path).map(OpenNode::Directory);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("migration tree contains a special file: {}", path.display()),
    ))
}

pub(super) fn ensure_normal_directory(path: &Path) -> io::Result<()> {
    if metadata_if_present(path)?.is_none() {
        fs::create_dir_all(path)?;
    }
    let _lease = DirectoryLease::open(path)?;
    Ok(())
}

pub(super) fn metadata_if_present(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) fn validate_regular_tree(path: &Path) -> io::Result<()> {
    validate_regular_tree_with_limits(path, TreeLimits::default())
}

pub(super) fn validate_regular_tree_with_limits(path: &Path, limits: TreeLimits) -> io::Result<()> {
    let mut budget = TreeBudget::new(limits);
    let mut pending = vec![(path.to_path_buf(), 0_usize)];
    while let Some((path, depth)) = pending.pop() {
        match open_node(&path)? {
            OpenNode::File(_, metadata) => budget.visit(depth, metadata.len())?,
            OpenNode::Directory(directory) => {
                budget.visit(depth, 0)?;
                for name in directory.sorted_entry_names()?.into_iter().rev() {
                    pending.push((path.join(name), depth.saturating_add(1)));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn copy_tree_verified(source: &Path, destination: &Path) -> io::Result<()> {
    if metadata_if_present(destination)?.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "staging destination already exists: {}",
                destination.display()
            ),
        ));
    }
    copy_tree(source, destination, TreeLimits::default())?;
    if !trees_equal(source, destination)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "staged copy does not match its source: {}",
                source.display()
            ),
        ));
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path, limits: TreeLimits) -> io::Result<()> {
    let mut budget = TreeBudget::new(limits);
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf(), 0_usize)];
    while let Some((source, destination, depth)) = pending.pop() {
        match open_node(&source)? {
            OpenNode::File(mut input, metadata) => {
                budget.visit(depth, metadata.len())?;
                let mut output = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&destination)?;
                io::copy(&mut input, &mut output)?;
                output.flush()?;
                output.sync_all()?;
            }
            OpenNode::Directory(directory) => {
                budget.visit(depth, 0)?;
                fs::create_dir(&destination)?;
                for name in directory.sorted_entry_names()?.into_iter().rev() {
                    pending.push((
                        source.join(&name),
                        destination.join(name),
                        depth.saturating_add(1),
                    ));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn trees_equal(left: &Path, right: &Path) -> io::Result<bool> {
    let mut budget = TreeBudget::new(TreeLimits::default());
    let mut pending = vec![(left.to_path_buf(), right.to_path_buf(), 0_usize)];
    while let Some((left, right, depth)) = pending.pop() {
        match (open_node(&left)?, open_node(&right)?) {
            (OpenNode::File(left, left_metadata), OpenNode::File(right, right_metadata)) => {
                budget.visit(depth, left_metadata.len())?;
                budget.visit(depth, right_metadata.len())?;
                if left_metadata.len() != right_metadata.len()
                    || sha256_file(left)? != sha256_file(right)?
                {
                    return Ok(false);
                }
            }
            (OpenNode::Directory(left_dir), OpenNode::Directory(right_dir)) => {
                budget.visit(depth, 0)?;
                budget.visit(depth, 0)?;
                let left_names = left_dir.sorted_entry_names()?;
                let right_names = right_dir.sorted_entry_names()?;
                if left_names != right_names {
                    return Ok(false);
                }
                for name in left_names.into_iter().rev() {
                    pending.push((left.join(&name), right.join(name), depth.saturating_add(1)));
                }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

fn sha256_file(mut file: File) -> io::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

pub(super) fn create_unique_directory(parent: &Path, prefix: &str) -> io::Result<PathBuf> {
    let nonce = unique_nonce();
    for attempt in 0..1000_u16 {
        let candidate = parent.join(format!("{prefix}-{}-{nonce}-{attempt}", std::process::id()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(allocation_error("directory", parent))
}

pub(super) fn create_unique_file(parent: &Path, prefix: &str) -> io::Result<(PathBuf, File)> {
    let nonce = unique_nonce();
    for attempt in 0..1000_u16 {
        let candidate = parent.join(format!(
            "{prefix}-{}-{nonce}-{attempt}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(allocation_error("file", parent))
}

fn allocation_error(kind: &str, parent: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "could not allocate migration {kind} under {}",
            parent.display()
        ),
    )
}

fn unique_nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

pub(super) fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}
