use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use cargo_metadata::{CargoOpt, MetadataCommand, PackageId};
#[cfg(test)]
use sha2::{Digest, Sha256};

use crate::Backend;

fn backend_key(backend: Backend) -> &'static str {
    match backend {
        Backend::Risc0 => "risc0",
        Backend::Sp1 => "sp1",
        Backend::All => unreachable!("provenance is computed per concrete backend"),
    }
}

pub(crate) fn guest_source_paths(
    root: &Path,
    backend: Backend,
    bench: bool,
) -> Result<BTreeSet<PathBuf>> {
    let backend_key = backend_key(backend);
    let manifest_path = root.join(format!("guests/{backend_key}/Cargo.toml"));
    let mut command = MetadataCommand::new();
    command.manifest_path(&manifest_path);
    command.other_options(vec!["--locked".to_string()]);
    if bench {
        command.features(CargoOpt::SomeFeatures(vec!["bench".to_string()]));
    }
    let metadata = command
        .exec()
        .with_context(|| format!("resolve {backend_key} guest dependencies"))?;
    let root_package = metadata
        .root_package()
        .context("guest metadata has no root package")?;
    let resolve = metadata
        .resolve
        .as_ref()
        .context("guest metadata has no resolve graph")?;
    let nodes = resolve
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect::<HashMap<_, _>>();
    let packages = metadata
        .packages
        .iter()
        .map(|package| (package.id.clone(), package))
        .collect::<HashMap<_, _>>();

    let mut queue = VecDeque::from([root_package.id.clone()]);
    let mut visited = HashSet::<PackageId>::new();
    let mut paths = BTreeSet::new();
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        let package = packages
            .get(&id)
            .with_context(|| format!("resolved package {id} missing metadata"))?;
        if package.source.is_none() {
            let manifest_path = package.manifest_path.clone().into_std_path_buf();
            if !manifest_path.starts_with(root) {
                bail!(
                    "local guest dependency is outside repository: {}",
                    manifest_path.display()
                );
            }
            let package_root = manifest_path
                .parent()
                .context("local guest manifest has no parent directory")?
                .to_path_buf();
            paths.insert(manifest_path);
            collect_files(&package_root.join("src"), &mut paths)?;
            let build_script = package_root.join("build.rs");
            if build_script.is_file() {
                paths.insert(build_script);
            }
        }
        if let Some(node) = nodes.get(&id) {
            queue.extend(node.dependencies.iter().cloned());
        }
    }

    Ok(paths)
}

pub(crate) fn expected_artifacts(root: &Path, backend: Backend) -> Result<Vec<PathBuf>> {
    match backend {
        Backend::All => {
            let mut artifacts = expected_artifacts(root, Backend::Risc0)?;
            artifacts.extend(expected_artifacts(root, Backend::Sp1)?);
            Ok(artifacts)
        }
        Backend::Risc0 | Backend::Sp1 => {
            let backend_key = backend_key(backend);
            let manifest = crate::read_manifest(
                &root.join(format!("guests/{backend_key}/Cargo.toml")),
            )?;
            let mut artifacts = Vec::new();
            for binary in manifest.bin {
                let artifact_name = binary.name.replace('-', "_");
                artifacts.push(
                    root.join("crates/guests/elf")
                        .join(format!("{artifact_name}.elf")),
                );
                if backend == Backend::Sp1 {
                    artifacts.push(
                        root.join("crates/guests/elf")
                            .join(format!("{artifact_name}.vk.bin")),
                    );
                }
            }
            artifacts.sort();
            Ok(artifacts)
        }
    }
}

pub(crate) fn source_fingerprint(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
) -> Result<String> {
    crate::compute_guest_fingerprint(root, backend, bench, sp1_docker_tag, false)
}

#[cfg(test)]
fn hash_source_paths(root: &Path, paths: &BTreeSet<PathBuf>) -> Result<String> {
    let mut hasher = Sha256::new();
    for path in paths {
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("source path {path:?} is outside root {root:?}"))?;
        let bytes = fs::read(path).with_context(|| format!("read source file {path:?}"))?;
        crate::hash_tagged_bytes(
            &mut hasher,
            "source_path",
            relative.to_string_lossy().as_bytes(),
        );
        crate::hash_tagged_bytes(&mut hasher, "source_file", &bytes);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files(path: &Path, paths: &mut BTreeSet<PathBuf>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_file() {
        paths.insert(path.to_path_buf());
        return Ok(());
    }

    for entry in fs::read_dir(path).with_context(|| format!("read source directory {path:?}"))? {
        let entry = entry.with_context(|| format!("read source entry under {path:?}"))?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            collect_files(&entry_path, paths)?;
        } else if entry_path.is_file() {
            paths.insert(entry_path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Backend;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn real_guest_graph_contains_all_transitive_local_crates() {
        let root = crate::repo_root();
        let paths = guest_source_paths(&root, Backend::Sp1, false).unwrap();

        for expected in [
            "crates/guest-common/Cargo.toml",
            "crates/primitives/Cargo.toml",
            "crates/primitives-shasta/Cargo.toml",
            "crates/protocol/Cargo.toml",
            "crates/protocol-shasta/Cargo.toml",
            "crates/stateless/Cargo.toml",
        ] {
            assert!(paths.contains(&root.join(expected)), "missing {expected}");
        }
    }

    #[test]
    fn transitive_local_source_change_changes_fingerprint() {
        let root = crate::repo_root();
        let source_paths = guest_source_paths(&root, Backend::Sp1, false).unwrap();
        let fixture_root = temp_test_dir();
        let mut fixture_paths = BTreeSet::new();

        for source in source_paths {
            let relative = source.strip_prefix(&root).unwrap();
            let destination = fixture_root.join(relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::copy(&source, &destination).unwrap();
            fixture_paths.insert(destination);
        }

        let before = hash_source_paths(&fixture_root, &fixture_paths).unwrap();
        let transitive_source = fixture_root.join("crates/stateless/src/lib.rs");
        fs::write(&transitive_source, b"pub fn changed() {}\n").unwrap();
        let after = hash_source_paths(&fixture_root, &fixture_paths).unwrap();

        assert_ne!(before, after);
        let _ = fs::remove_dir_all(fixture_root);
    }

    #[test]
    fn sp1_artifact_inventory_pairs_every_elf_with_a_vk() {
        let root = crate::repo_root();
        let artifacts = expected_artifacts(&root, Backend::Sp1).unwrap();
        let elf_count = artifacts
            .iter()
            .filter(|path| path.extension().is_some_and(|extension| extension == "elf"))
            .count();
        let vk_count = artifacts
            .iter()
            .filter(|path| path.to_string_lossy().ends_with(".vk.bin"))
            .count();

        assert_eq!(elf_count, vk_count);
        assert!(elf_count >= 2);
    }

    fn temp_test_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("raiko2-guest-provenance-test-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
