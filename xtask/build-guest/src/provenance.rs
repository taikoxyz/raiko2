use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use cargo_metadata::{CargoOpt, MetadataCommand, PackageId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Backend;

const PROVENANCE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct GuestProvenance {
    schema_version: u32,
    backend: String,
    bench: bool,
    source_fingerprint: String,
    artifacts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GuestProvenanceStatus {
    Current,
    Stale(String),
}

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

pub(crate) fn build_input_paths(
    root: &Path,
    backend: Backend,
    bench: bool,
) -> Result<BTreeSet<PathBuf>> {
    let backend_key = backend_key(backend);
    let mut paths = guest_source_paths(root, backend, bench)?;
    paths.extend([
        root.join("Cargo.toml"),
        root.join("rust-toolchain.toml"),
        root.join("xtask/build-guest/Cargo.toml"),
        root.join(format!("guests/{backend_key}/Cargo.lock")),
        root.join(format!("docker/{backend_key}-toolchain/Dockerfile")),
    ]);
    collect_files(&root.join("xtask/build-guest/src"), &mut paths)?;
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
            let manifest =
                crate::read_manifest(&root.join(format!("guests/{backend_key}/Cargo.toml")))?;
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
    crate::compute_guest_fingerprint(root, backend, bench, sp1_docker_tag)
}

pub(crate) fn check_backend(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
) -> Result<GuestProvenanceStatus> {
    let source_fingerprint = source_fingerprint(root, backend, bench, sp1_docker_tag)?;
    check_backend_with_source(root, backend, bench, &source_fingerprint)
}

pub(crate) fn write_backend(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
) -> Result<()> {
    let source_fingerprint = source_fingerprint(root, backend, bench, sp1_docker_tag)?;
    write_backend_with_source(root, backend, bench, &source_fingerprint)
}

fn check_backend_with_source(
    root: &Path,
    backend: Backend,
    bench: bool,
    source_fingerprint: &str,
) -> Result<GuestProvenanceStatus> {
    let path = provenance_path(root, backend);
    if !path.is_file() {
        return Ok(GuestProvenanceStatus::Stale(format!(
            "missing provenance manifest {}",
            path.display()
        )));
    }

    let bytes = fs::read(&path).with_context(|| format!("read provenance manifest {path:?}"))?;
    let recorded: GuestProvenance = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse provenance manifest {path:?}"))?;
    let backend_key = backend_key(backend);
    if recorded.schema_version != PROVENANCE_SCHEMA_VERSION {
        return Ok(GuestProvenanceStatus::Stale(format!(
            "schema version mismatch: recorded {}, expected {PROVENANCE_SCHEMA_VERSION}",
            recorded.schema_version
        )));
    }
    if recorded.backend != backend_key {
        return Ok(GuestProvenanceStatus::Stale(format!(
            "backend mismatch: recorded {}, expected {backend_key}",
            recorded.backend
        )));
    }
    if recorded.bench != bench {
        return Ok(GuestProvenanceStatus::Stale(format!(
            "bench feature mismatch: recorded {}, expected {bench}",
            recorded.bench
        )));
    }
    if recorded.source_fingerprint != source_fingerprint {
        return Ok(GuestProvenanceStatus::Stale(
            "source fingerprint mismatch".to_string(),
        ));
    }

    let expected_paths = expected_artifact_keys(root, backend)?;
    for path in expected_artifacts(root, backend)? {
        if !path.is_file() {
            return Ok(GuestProvenanceStatus::Stale(format!(
                "missing artifact {}",
                path.display()
            )));
        }
    }
    let actual_paths = actual_backend_artifact_keys(root, backend)?;
    if let Some(unexpected) = actual_paths.difference(&expected_paths).next() {
        return Ok(GuestProvenanceStatus::Stale(format!(
            "unexpected artifact {unexpected}"
        )));
    }

    let current = artifact_digests(root, backend)?;
    if recorded.artifacts.keys().collect::<BTreeSet<_>>() != current.keys().collect::<BTreeSet<_>>()
    {
        return Ok(GuestProvenanceStatus::Stale(
            "artifact set mismatch".to_string(),
        ));
    }
    for (path, digest) in current {
        if recorded.artifacts.get(&path) != Some(&digest) {
            return Ok(GuestProvenanceStatus::Stale(format!(
                "artifact digest mismatch for {path}"
            )));
        }
    }

    Ok(GuestProvenanceStatus::Current)
}

fn write_backend_with_source(
    root: &Path,
    backend: Backend,
    bench: bool,
    source_fingerprint: &str,
) -> Result<()> {
    let provenance = GuestProvenance {
        schema_version: PROVENANCE_SCHEMA_VERSION,
        backend: backend_key(backend).to_string(),
        bench,
        source_fingerprint: source_fingerprint.to_string(),
        artifacts: artifact_digests(root, backend)?,
    };
    let path = provenance_path(root, backend);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create provenance directory {parent:?}"))?;
    }
    let mut bytes = serde_json::to_vec_pretty(&provenance)
        .with_context(|| format!("serialize provenance manifest {path:?}"))?;
    bytes.push(b'\n');
    fs::write(&path, bytes).with_context(|| format!("write provenance manifest {path:?}"))
}

fn provenance_path(root: &Path, backend: Backend) -> PathBuf {
    root.join("crates/guests/elf")
        .join(format!("{}.provenance.json", backend_key(backend)))
}

fn artifact_digests(root: &Path, backend: Backend) -> Result<BTreeMap<String, String>> {
    let mut digests = BTreeMap::new();
    for path in expected_artifacts(root, backend)? {
        let key = relative_path(root, &path)?;
        digests.insert(key, sha256_file(&path)?);
    }
    Ok(digests)
}

fn expected_artifact_keys(root: &Path, backend: Backend) -> Result<BTreeSet<String>> {
    expected_artifacts(root, backend)?
        .iter()
        .map(|path| relative_path(root, path))
        .collect()
}

fn actual_backend_artifact_keys(root: &Path, backend: Backend) -> Result<BTreeSet<String>> {
    let artifact_dir = root.join("crates/guests/elf");
    let prefix = format!("{}_", backend_key(backend));
    let mut artifacts = BTreeSet::new();
    if !artifact_dir.is_dir() {
        return Ok(artifacts);
    }
    for entry in fs::read_dir(&artifact_dir)
        .with_context(|| format!("read guest artifact directory {artifact_dir:?}"))?
    {
        let path = entry
            .with_context(|| format!("read guest artifact entry under {artifact_dir:?}"))?
            .path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if path.is_file()
            && name.starts_with(&prefix)
            && (name.ends_with(".elf") || name.ends_with(".vk.bin"))
        {
            artifacts.insert(relative_path(root, &path)?);
        }
    }
    Ok(artifacts)
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    Ok(path
        .strip_prefix(root)
        .with_context(|| format!("artifact path {path:?} is outside root {root:?}"))?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read guest artifact {path:?}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEST_DIR_ID: AtomicU64 = AtomicU64::new(0);

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

    #[test]
    fn build_input_closure_includes_workspace_and_provenance_sources() {
        let root = crate::repo_root();
        let paths = build_input_paths(&root, Backend::Sp1, false).unwrap();

        assert!(paths.contains(&root.join("Cargo.toml")));
        assert!(paths.contains(&root.join("xtask/build-guest/src/provenance.rs")));
    }

    #[test]
    fn tracked_provenance_round_trip_is_current() {
        let root = provenance_fixture();
        write_backend_with_source(&root, Backend::Risc0, false, "source-a").unwrap();

        assert_eq!(
            check_backend_with_source(&root, Backend::Risc0, false, "source-a").unwrap(),
            GuestProvenanceStatus::Current
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tracked_provenance_rejects_source_drift() {
        let root = provenance_fixture();
        write_backend_with_source(&root, Backend::Risc0, false, "source-a").unwrap();

        let status = check_backend_with_source(&root, Backend::Risc0, false, "source-b").unwrap();
        assert!(matches!(
            status,
            GuestProvenanceStatus::Stale(reason)
                if reason.contains("source fingerprint mismatch")
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tracked_provenance_rejects_modified_artifact_bytes() {
        let root = provenance_fixture();
        write_backend_with_source(&root, Backend::Risc0, false, "source-a").unwrap();
        fs::write(
            root.join("crates/guests/elf/risc0_shasta_proposal.elf"),
            b"changed",
        )
        .unwrap();

        let status = check_backend_with_source(&root, Backend::Risc0, false, "source-a").unwrap();
        assert!(matches!(
            status,
            GuestProvenanceStatus::Stale(reason)
                if reason.contains("artifact digest mismatch")
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tracked_provenance_rejects_missing_artifact() {
        let root = provenance_fixture();
        write_backend_with_source(&root, Backend::Risc0, false, "source-a").unwrap();
        fs::remove_file(root.join("crates/guests/elf/risc0_shasta_aggregation.elf")).unwrap();

        let status = check_backend_with_source(&root, Backend::Risc0, false, "source-a").unwrap();
        assert!(matches!(
            status,
            GuestProvenanceStatus::Stale(reason) if reason.contains("missing artifact")
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tracked_provenance_rejects_unexpected_backend_artifact() {
        let root = provenance_fixture();
        write_backend_with_source(&root, Backend::Risc0, false, "source-a").unwrap();
        fs::write(
            root.join("crates/guests/elf/risc0_unexpected.elf"),
            b"unexpected",
        )
        .unwrap();

        let status = check_backend_with_source(&root, Backend::Risc0, false, "source-a").unwrap();
        assert!(matches!(
            status,
            GuestProvenanceStatus::Stale(reason)
                if reason.contains("unexpected artifact")
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tracked_provenance_is_deterministic_and_repository_relative() {
        let root = provenance_fixture();
        write_backend_with_source(&root, Backend::Risc0, false, "source-a").unwrap();
        let manifest =
            fs::read_to_string(root.join("crates/guests/elf/risc0.provenance.json")).unwrap();

        assert!(manifest.ends_with('\n'));
        assert!(!manifest.contains(&root.display().to_string()));
        let aggregation = manifest.find("risc0_shasta_aggregation.elf").unwrap();
        let proposal = manifest.find("risc0_shasta_proposal.elf").unwrap();
        assert!(aggregation < proposal);
        let _ = fs::remove_dir_all(root);
    }

    fn provenance_fixture() -> PathBuf {
        let root = temp_test_dir();
        let guest_dir = root.join("guests/risc0");
        let artifact_dir = root.join("crates/guests/elf");
        fs::create_dir_all(&guest_dir).unwrap();
        fs::create_dir_all(&artifact_dir).unwrap();
        fs::write(
            guest_dir.join("Cargo.toml"),
            r#"[package]
name = "fixture-risc0-guest"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "risc0-shasta-proposal"
path = "src/proposal.rs"

[[bin]]
name = "risc0-shasta-aggregation"
path = "src/aggregation.rs"
"#,
        )
        .unwrap();
        fs::write(artifact_dir.join("risc0_shasta_proposal.elf"), b"proposal").unwrap();
        fs::write(
            artifact_dir.join("risc0_shasta_aggregation.elf"),
            b"aggregation",
        )
        .unwrap();
        root
    }

    fn temp_test_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_TEST_DIR_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("raiko2-guest-provenance-test-{unique}-{sequence}"));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
