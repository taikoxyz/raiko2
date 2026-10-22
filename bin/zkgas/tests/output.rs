mod common;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
};

use common::trusted_temp_root;
use zkgas::output::{CommitOutcome, DirectoryCommitLimits, commit_directory_bounded, commit_file};

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = trusted_temp_root().join(format!("zkgas-output-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn stage(base: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = base.join(name);
    fs::create_dir(&path).unwrap();
    fs::create_dir(path.join("rows")).unwrap();
    fs::write(path.join("manifest.json"), b"manifest").unwrap();
    fs::write(path.join("rows/row.json"), contents).unwrap();
    path
}

fn commit_directory(staging: &Path, final_path: &Path) -> anyhow::Result<CommitOutcome> {
    commit_directory_bounded(
        staging,
        final_path,
        DirectoryCommitLimits {
            max_entries: 16,
            max_depth: 4,
            max_bytes: 1024,
        },
    )
}

#[test]
fn file_commit_is_immutable_and_identical_bytes_are_reused() {
    let dir = TestDir::new();
    let path = dir.path().join("quote.jsonl");
    assert_eq!(
        commit_file(&path, b"first").unwrap(),
        CommitOutcome::Created
    );
    assert_eq!(commit_file(&path, b"first").unwrap(), CommitOutcome::Reused);
    assert!(commit_file(&path, b"different").is_err());
    assert_eq!(fs::read(path).unwrap(), b"first");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn directories_reuse_only_the_exact_complete_path_and_byte_set() {
    let dir = TestDir::new();
    let final_path = dir.path().join("run");
    assert_eq!(
        commit_directory(&stage(dir.path(), "stage-1", b"row"), &final_path).unwrap(),
        CommitOutcome::Created
    );
    assert_eq!(
        commit_directory(&stage(dir.path(), "stage-2", b"row"), &final_path).unwrap(),
        CommitOutcome::Reused
    );
    assert!(commit_directory(&stage(dir.path(), "stage-3", b"modified"), &final_path).is_err());
    assert_eq!(fs::read(final_path.join("rows/row.json")).unwrap(), b"row");
    fs::write(final_path.join("extra.json"), b"extra").unwrap();
    assert!(commit_directory(&stage(dir.path(), "stage-4", b"row"), &final_path).is_err());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn bounded_directory_reuse_rejects_existing_bytes_entries_and_depth_before_comparison() {
    let limits = DirectoryCommitLimits {
        max_entries: 3,
        max_depth: 2,
        max_bytes: 11,
    };
    for mutation in ["bytes", "entries", "depth"] {
        let limits = DirectoryCommitLimits {
            max_entries: if mutation == "depth" {
                10
            } else {
                limits.max_entries
            },
            ..limits
        };
        let dir = TestDir::new();
        let final_path = dir.path().join("run");
        assert_eq!(
            commit_directory_bounded(&stage(dir.path(), "stage-1", b"row"), &final_path, limits,)
                .unwrap(),
            CommitOutcome::Created
        );
        match mutation {
            "bytes" => {
                fs::OpenOptions::new()
                    .write(true)
                    .open(final_path.join("rows/row.json"))
                    .unwrap()
                    .set_len(12)
                    .unwrap();
            }
            "entries" => fs::write(final_path.join("extra.json"), b"").unwrap(),
            "depth" => {
                fs::create_dir(final_path.join("rows/nested")).unwrap();
                fs::write(final_path.join("rows/nested/extra.json"), b"").unwrap();
            }
            _ => unreachable!(),
        }
        let error =
            commit_directory_bounded(&stage(dir.path(), "stage-2", b"row"), &final_path, limits)
                .unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains(match mutation {
                "bytes" => "byte limit",
                "entries" => "entry limit",
                "depth" => "depth limit",
                _ => unreachable!(),
            }),
            "{mutation}: {error:#}"
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn existing_empty_or_partial_directories_are_never_replaced() {
    for partial in [false, true] {
        let dir = TestDir::new();
        let final_path = dir.path().join("run");
        fs::create_dir(&final_path).unwrap();
        if partial {
            fs::write(final_path.join("manifest.json"), b"manifest").unwrap();
        }
        assert!(commit_directory(&stage(dir.path(), "stage", b"row"), &final_path).is_err());
        assert!(!final_path.join("rows").exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn existing_symlinks_are_rejected_even_when_the_target_bytes_match() {
    use std::os::unix::fs::symlink;
    let dir = TestDir::new();
    fs::write(dir.path().join("target"), b"row").unwrap();
    let link = dir.path().join("quote");
    symlink("target", &link).unwrap();
    assert!(commit_file(&link, b"row").is_err());
    let final_path = dir.path().join("run");
    fs::create_dir(&final_path).unwrap();
    fs::create_dir(final_path.join("rows")).unwrap();
    fs::write(final_path.join("manifest.json"), b"manifest").unwrap();
    symlink("../../target", final_path.join("rows/row.json")).unwrap();
    assert!(commit_directory(&stage(dir.path(), "stage", b"row"), &final_path).is_err());
    assert!(
        fs::symlink_metadata(final_path.join("rows/row.json"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn existing_ancestor_symlink_is_never_followed_during_directory_reuse() {
    use std::os::unix::fs::symlink;

    let dir = TestDir::new();
    let outside = TestDir::new();
    fs::create_dir(outside.path().join("rows")).unwrap();
    fs::write(outside.path().join("rows/row.json"), b"row").unwrap();

    let final_path = dir.path().join("run");
    fs::create_dir(&final_path).unwrap();
    fs::write(final_path.join("manifest.json"), b"manifest").unwrap();
    symlink(outside.path().join("rows"), final_path.join("rows")).unwrap();

    assert!(commit_directory(&stage(dir.path(), "stage", b"row"), &final_path).is_err());
    assert!(
        fs::symlink_metadata(final_path.join("rows"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn caller_supplied_symlinked_output_parent_remains_idempotent() {
    use std::os::unix::fs::symlink;

    let real = TestDir::new();
    let links = TestDir::new();
    let linked_parent = links.path().join("output");
    symlink(real.path(), &linked_parent).unwrap();

    let file = linked_parent.join("quote.jsonl");
    assert_eq!(
        commit_file(&file, b"quote").unwrap(),
        CommitOutcome::Created
    );
    assert_eq!(commit_file(&file, b"quote").unwrap(), CommitOutcome::Reused);

    let final_path = linked_parent.join("run");
    assert_eq!(
        commit_directory(&stage(&linked_parent, "stage-1", b"row"), &final_path).unwrap(),
        CommitOutcome::Created
    );
    assert_eq!(
        commit_directory(&stage(&linked_parent, "stage-2", b"row"), &final_path).unwrap(),
        CommitOutcome::Reused
    );
}

#[test]
fn concurrent_file_writers_have_one_canonical_result_without_staging_leaks() {
    for identical in [true, false] {
        let dir = TestDir::new();
        let path = dir.path().join("output");
        let barrier = Arc::new(Barrier::new(2));
        let results: Vec<_> = [
            b"first".as_slice(),
            if identical { b"first" } else { b"second" },
        ]
        .into_iter()
        .map(|bytes| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                commit_file(&path, bytes)
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Ok(CommitOutcome::Created)))
                .count(),
            1
        );
        if identical {
            assert!(results.iter().all(Result::is_ok));
        } else {
            assert_eq!(results.iter().filter(|r| r.is_err()).count(), 1);
        }
        let bytes = fs::read(path).unwrap();
        assert!(bytes == b"first" || (!identical && bytes == b"second"));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn concurrent_directory_writers_never_mix_runs_or_leave_staging_paths() {
    for identical in [true, false] {
        let dir = TestDir::new();
        let final_path = dir.path().join("run");
        let first = stage(dir.path(), "first-stage", b"first");
        let second = stage(
            dir.path(),
            "second-stage",
            if identical { b"first" } else { b"second" },
        );
        let barrier = Arc::new(Barrier::new(2));
        let results: Vec<_> = [first, second]
            .into_iter()
            .map(|staging| {
                let final_path = final_path.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    commit_directory(&staging, &final_path)
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Ok(CommitOutcome::Created)))
                .count(),
            1
        );
        if identical {
            assert!(results.iter().all(Result::is_ok));
        } else {
            assert_eq!(results.iter().filter(|r| r.is_err()).count(), 1);
        }
        let bytes = fs::read(final_path.join("rows/row.json")).unwrap();
        assert!(bytes == b"first" || (!identical && bytes == b"second"));
        assert_eq!(
            fs::read(final_path.join("manifest.json")).unwrap(),
            b"manifest"
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn staging_alias_is_rejected_without_removing_existing_output() {
    let dir = TestDir::new();
    let staging = stage(dir.path(), "run", b"row");
    let alias = dir.path().join(".").join("run");
    assert!(commit_directory(&staging, &alias).is_err());
    assert_eq!(fs::read(staging.join("rows/row.json")).unwrap(), b"row");
}
#[test]
fn bounded_commit_rejects_oversized_existing_files_and_preserves_no_replace_races() {
    let root = trusted_temp_root().join(format!("zkgas-bounded-output-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let destination = root.join("report.json");
    std::fs::write(&destination, b"existing").unwrap();
    assert!(zkgas::output::commit_file_bounded(&destination, b"existing", 4).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"existing");
    assert_eq!(
        zkgas::output::commit_file_bounded(&destination, b"existing", 8).unwrap(),
        zkgas::output::CommitOutcome::Reused
    );
    let raced = root.join("raced.json");
    let writers: Vec<_> = (0..8)
        .map(|_| {
            let raced = raced.clone();
            std::thread::spawn(move || {
                zkgas::output::commit_file_bounded(&raced, b"same", 4).unwrap()
            })
        })
        .collect();
    let outcomes: Vec<_> = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .collect();
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == zkgas::output::CommitOutcome::Created)
            .count(),
        1
    );
    assert_eq!(std::fs::read(&raced).unwrap(), b"same");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
    std::fs::remove_dir_all(&root).unwrap();
}
