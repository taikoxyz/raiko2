//! Atomic immutable output publication.
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use rustix::fs::{CWD, OFlags, RenameFlags, renameat_with};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Created,
    Reused,
}

fn parent(path: &Path) -> Result<&Path> {
    ensure!(path.file_name().is_some(), "output must have a filename");
    Ok(path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new(".")))
}

fn read_regular(path: &Path) -> Result<Vec<u8>> {
    read_regular_bounded(path, u64::MAX)
}

fn read_regular_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32)
        .open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "output is not a regular file: {}",
        path.display()
    );
    ensure!(
        file.metadata()?.len() <= limit,
        "existing output byte limit exceeded"
    );
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "existing output byte limit exceeded"
    );
    Ok(bytes)
}

/// Publish complete bytes atomically without ever overwriting an existing destination.
pub fn commit_file(path: &Path, bytes: &[u8]) -> Result<CommitOutcome> {
    commit_file_bounded(path, bytes, u64::MAX)
}

/// Same atomic publication, with a hard read cap even if a conflicting file appears concurrently.
pub fn commit_file_bounded(
    path: &Path,
    bytes: &[u8],
    max_existing_bytes: u64,
) -> Result<CommitOutcome> {
    let parent = parent(path)?;
    let staging = parent.join(format!(".zkgas-file-{}.tmp", uuid::Uuid::new_v4()));
    let mut temporary = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)?;
    let result = (|| {
        temporary.write_all(bytes)?;
        temporary.sync_all()?;
        match renameat_with(CWD, &staging, CWD, path, RenameFlags::NOREPLACE) {
            Ok(()) => {
                File::open(parent)?.sync_all()?;
                Ok(CommitOutcome::Created)
            }
            Err(rustix::io::Errno::EXIST) => {
                ensure!(
                    read_regular_bounded(path, max_existing_bytes)? == bytes,
                    "immutable output conflict: {}",
                    path.display()
                );
                Ok(CommitOutcome::Reused)
            }
            Err(error) => Err(error.into()),
        }
    })();
    drop(temporary);
    // This randomly named file was created by this call; a conflicting destination is untouched.
    let cleanup = match fs::remove_file(&staging) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    };
    match result {
        Ok(outcome) => {
            cleanup?;
            Ok(outcome)
        }
        Err(error) => Err(error),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TreeEntry {
    Directory,
    File(Vec<u8>),
}

fn inspect_tree(root: &Path, sync: bool) -> Result<BTreeMap<PathBuf, TreeEntry>> {
    fn walk(
        root: &Path,
        path: &Path,
        entries: &mut BTreeMap<PathBuf, TreeEntry>,
        sync: bool,
    ) -> Result<()> {
        let meta = fs::symlink_metadata(path)?;
        ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "output tree contains a symlink or non-directory: {}",
            path.display()
        );
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            let meta = fs::symlink_metadata(&path)?;
            let relative = path.strip_prefix(root)?.to_owned();
            if meta.is_dir() && !meta.file_type().is_symlink() {
                entries.insert(relative, TreeEntry::Directory);
                walk(root, &path, entries, sync)?;
            } else if meta.is_file() && !meta.file_type().is_symlink() {
                entries.insert(relative, TreeEntry::File(read_regular(&path)?));
                if sync {
                    File::open(&path)?.sync_all()?;
                }
            } else {
                bail!(
                    "output tree contains a symlink or special file: {}",
                    path.display()
                );
            }
        }
        if sync {
            File::open(path)?.sync_all()?;
        }
        Ok(())
    }
    let mut entries = BTreeMap::new();
    walk(root, root, &mut entries, sync)?;
    Ok(entries)
}

/// Publish a caller-created complete staging directory beside the destination.
///
/// On reuse or conflict, only the supplied staging directory is removed. Existing output is
/// never removed. Reuse requires exact relative paths, node types, and file bytes, including empty
/// directories. The caller must exclusively own staging until this function returns.
pub fn commit_directory(staging: &Path, final_path: &Path) -> Result<CommitOutcome> {
    let destination_parent = parent(final_path)?;
    ensure!(staging != final_path, "staging cannot be the destination");
    ensure!(
        fs::canonicalize(parent(staging)?)? == fs::canonicalize(destination_parent)?,
        "staging must be beside the destination"
    );
    let staging_meta = fs::symlink_metadata(staging)?;
    ensure!(
        staging_meta.is_dir() && !staging_meta.file_type().is_symlink(),
        "staging must be a real directory"
    );
    // Reject aliases such as ./run versus run before accepting ownership of staging cleanup.
    if let Ok(destination) = fs::canonicalize(final_path) {
        ensure!(
            fs::canonicalize(staging)? != destination,
            "staging aliases destination"
        );
    }
    let result = (|| {
        let expected =
            inspect_tree(staging, true).context("validate complete staging directory")?;
        match renameat_with(CWD, staging, CWD, final_path, RenameFlags::NOREPLACE) {
            Ok(()) => {
                File::open(destination_parent)?.sync_all()?;
                Ok(CommitOutcome::Created)
            }
            Err(rustix::io::Errno::EXIST) => {
                let actual = inspect_tree(final_path, false)
                    .context("validate existing immutable directory")?;
                ensure!(
                    actual == expected,
                    "immutable directory conflict: {}",
                    final_path.display()
                );
                Ok(CommitOutcome::Reused)
            }
            Err(error) => Err(error.into()),
        }
    })();
    let cleanup = match fs::remove_dir_all(staging) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    };
    match result {
        Ok(outcome) => {
            cleanup?;
            Ok(outcome)
        }
        Err(error) => Err(error),
    }
}
