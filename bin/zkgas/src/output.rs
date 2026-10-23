//! Atomic immutable output publication.
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use rustix::fs::{CWD, Dir, Mode, OFlags, RenameFlags, openat, renameat_with};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Created,
    Reused,
}

/// A bounded sibling staging file that is atomically published without replacing an output.
pub struct AtomicFileWriter {
    destination: PathBuf,
    staging: PathBuf,
    temporary: File,
    max_bytes: u64,
    written: u64,
}

impl AtomicFileWriter {
    pub fn new(destination: &Path, max_bytes: u64) -> Result<Self> {
        let destination_parent = parent(destination)?;
        let staging = destination_parent.join(format!(".zkgas-file-{}.tmp", uuid::Uuid::new_v4()));
        let temporary = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        Ok(Self {
            destination: destination.to_owned(),
            staging,
            temporary,
            max_bytes,
            written: 0,
        })
    }

    pub fn append(&mut self, bytes: &[u8]) -> Result<()> {
        let next = self
            .written
            .checked_add(bytes.len() as u64)
            .context("output byte count overflow")?;
        ensure!(next <= self.max_bytes, "output byte limit exceeded");
        self.temporary.write_all(bytes)?;
        self.written = next;
        Ok(())
    }

    pub fn commit(self) -> Result<CommitOutcome> {
        self.temporary.sync_all()?;
        let destination_parent = parent(&self.destination)?;
        match renameat_with(
            CWD,
            &self.staging,
            CWD,
            &self.destination,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {
                File::open(destination_parent)?.sync_all()?;
                Ok(CommitOutcome::Created)
            }
            Err(rustix::io::Errno::EXIST) => {
                ensure!(
                    files_equal_bounded(
                        &self.staging,
                        &self.destination,
                        self.written,
                        self.max_bytes,
                    )?,
                    "immutable output conflict: {}",
                    self.destination.display()
                );
                Ok(CommitOutcome::Reused)
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for AtomicFileWriter {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.staging);
    }
}

fn parent(path: &Path) -> Result<&Path> {
    ensure!(path.file_name().is_some(), "output must have a filename");
    Ok(path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new(".")))
}

fn open_directory(path: &Path) -> Result<File> {
    let directory = File::open(path)?;
    ensure!(
        directory.metadata()?.is_dir(),
        "output parent is not a directory"
    );
    Ok(directory)
}

fn open_beneath_no_symlinks(root: &File, path: &Path, flags: OFlags) -> Result<File> {
    let mut current = root.try_clone()?;
    let mut components = path.components().peekable();
    ensure!(components.peek().is_some(), "output path must not be empty");
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            bail!("output tree path must stay beneath its root");
        };
        let component_flags = if components.peek().is_some() {
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY
        } else {
            flags
        };
        current = File::from(openat(
            &current,
            name,
            component_flags | OFlags::NOFOLLOW,
            Mode::empty(),
        )?);
    }
    Ok(current)
}

fn open_path_no_symlinks(path: &Path, flags: OFlags) -> Result<File> {
    let directory = open_directory(parent(path)?)?;
    open_beneath_no_symlinks(
        &directory,
        Path::new(path.file_name().context("output must have a filename")?),
        flags,
    )
}

fn files_equal_bounded(
    expected_path: &Path,
    actual_path: &Path,
    expected_length: u64,
    limit: u64,
) -> Result<bool> {
    let mut expected = open_path_no_symlinks(
        expected_path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
    )?;
    let mut actual = open_path_no_symlinks(
        actual_path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
    )?;
    open_files_equal_bounded(&mut expected, &mut actual, expected_length, limit)
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
    ensure!(
        bytes.len() as u64 <= max_existing_bytes,
        "output byte limit exceeded"
    );
    let mut output = AtomicFileWriter::new(path, max_existing_bytes)?;
    output.append(bytes)?;
    output.commit()
}

#[derive(Debug, PartialEq, Eq)]
enum TreeEntry {
    Directory,
    File { length: u64 },
}

#[derive(Clone, Copy, Debug)]
pub struct DirectoryCommitLimits {
    pub max_entries: usize,
    pub max_depth: usize,
    pub max_bytes: u64,
}

fn inspect_tree(
    root: &File,
    sync: bool,
    limits: DirectoryCommitLimits,
) -> Result<BTreeMap<PathBuf, TreeEntry>> {
    fn walk(
        directory: &File,
        relative_directory: &Path,
        entries: &mut BTreeMap<PathBuf, TreeEntry>,
        total_bytes: &mut u64,
        sync: bool,
        limits: DirectoryCommitLimits,
    ) -> Result<()> {
        let mut raw_directory = Dir::read_from(directory)?;
        while let Some(entry) = raw_directory.read() {
            let entry = entry?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            let name = std::ffi::OsStr::from_bytes(name).to_owned();
            let relative = relative_directory.join(&name);
            ensure!(
                relative.components().count() <= limits.max_depth,
                "output tree depth limit exceeded"
            );
            ensure!(
                entries.len() < limits.max_entries,
                "output tree entry limit exceeded"
            );
            let child = File::from(openat(
                directory,
                &name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
                Mode::empty(),
            )?);
            let meta = child.metadata()?;
            if meta.is_dir() {
                ensure!(
                    entries
                        .insert(relative.clone(), TreeEntry::Directory)
                        .is_none(),
                    "duplicate output tree entry"
                );
                walk(&child, &relative, entries, total_bytes, sync, limits)?;
            } else if meta.is_file() {
                *total_bytes = total_bytes
                    .checked_add(meta.len())
                    .context("output tree byte count overflow")?;
                ensure!(
                    *total_bytes <= limits.max_bytes,
                    "output tree byte limit exceeded"
                );
                ensure!(
                    entries
                        .insert(relative, TreeEntry::File { length: meta.len() })
                        .is_none(),
                    "duplicate output tree entry"
                );
                if sync {
                    child.sync_all()?;
                }
            } else {
                bail!("output tree contains a symlink or special file");
            }
        }
        if sync {
            directory.sync_all()?;
        }
        Ok(())
    }
    let mut entries = BTreeMap::new();
    let mut total_bytes = 0;
    walk(
        root,
        Path::new(""),
        &mut entries,
        &mut total_bytes,
        sync,
        limits,
    )?;
    Ok(entries)
}

/// Publish a caller-created complete staging directory beside the destination.
///
/// On reuse or conflict, only the supplied staging directory is removed. Existing output is
/// never removed. Reuse requires exact relative paths, node types, and file bytes, including empty
/// directories. The caller must exclusively own staging until this function returns.
pub fn commit_directory_bounded(
    staging: &Path,
    final_path: &Path,
    limits: DirectoryCommitLimits,
) -> Result<CommitOutcome> {
    let destination_parent = parent(final_path)?;
    let staging_name = staging
        .file_name()
        .context("staging must have a filename")?;
    let final_name = final_path
        .file_name()
        .context("destination must have a filename")?;
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
        let parent_directory = open_directory(destination_parent)?;
        let staging_directory = File::from(openat(
            &parent_directory,
            staging_name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NOFOLLOW,
            Mode::empty(),
        )?);
        let expected = inspect_tree(&staging_directory, true, limits)
            .context("validate complete staging directory")?;
        match renameat_with(
            &parent_directory,
            staging_name,
            &parent_directory,
            final_name,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {
                parent_directory.sync_all()?;
                Ok(CommitOutcome::Created)
            }
            Err(rustix::io::Errno::EXIST) => {
                let final_directory = File::from(openat(
                    &parent_directory,
                    final_name,
                    OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NOFOLLOW,
                    Mode::empty(),
                )?);
                let actual = inspect_tree(&final_directory, false, limits)
                    .context("validate existing immutable directory")?;
                ensure!(
                    actual == expected,
                    "immutable directory conflict: {}",
                    final_path.display()
                );
                for (relative, entry) in &expected {
                    if let TreeEntry::File { length } = entry {
                        let mut expected_file = open_beneath_no_symlinks(
                            &staging_directory,
                            relative,
                            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
                        )?;
                        let mut actual_file = open_beneath_no_symlinks(
                            &final_directory,
                            relative,
                            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
                        )?;
                        ensure!(
                            open_files_equal_bounded(
                                &mut expected_file,
                                &mut actual_file,
                                *length,
                                *length,
                            )?,
                            "immutable directory conflict: {}",
                            final_path.display()
                        );
                    }
                }
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

fn open_files_equal_bounded(
    expected: &mut File,
    actual: &mut File,
    expected_length: u64,
    limit: u64,
) -> Result<bool> {
    ensure!(
        expected_length <= limit,
        "staged output byte limit exceeded"
    );
    ensure!(
        expected.metadata()?.is_file(),
        "staged output is not a regular file"
    );
    ensure!(
        actual.metadata()?.is_file(),
        "existing output is not a regular file"
    );
    ensure!(
        actual.metadata()?.len() <= limit,
        "existing output byte limit exceeded"
    );
    if expected.metadata()?.len() != expected_length || actual.metadata()?.len() != expected_length
    {
        return Ok(false);
    }
    let mut expected_buffer = [0u8; 64 * 1024];
    let mut actual_buffer = [0u8; 64 * 1024];
    let mut remaining = expected_length;
    while remaining != 0 {
        let chunk = usize::try_from(remaining.min(expected_buffer.len() as u64))?;
        let expected_count = expected.read(&mut expected_buffer[..chunk])?;
        let actual_count = actual.read(&mut actual_buffer[..chunk])?;
        if expected_count != actual_count
            || expected_buffer[..expected_count] != actual_buffer[..actual_count]
        {
            return Ok(false);
        }
        if expected_count == 0 {
            return Ok(false);
        }
        remaining -= expected_count as u64;
    }
    let expected_tail = expected.read(&mut expected_buffer[..1])?;
    let actual_tail = actual.read(&mut actual_buffer[..1])?;
    Ok(expected_tail == 0 && actual_tail == 0)
}
