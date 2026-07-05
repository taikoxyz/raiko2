use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use risc0_binfmt::ProgramBinary;
use risc0_zkos_v1compat::V1COMPAT_ELF;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sp1_sdk::{
    ProvingKey as _,
    blocking::{Prover as _, ProverClient},
};

#[cfg(feature = "digests")]
pub mod guest_digests;
mod util;

const DEFAULT_RISC0_RUSTFLAGS: &str = "-C passes=lower-atomic -C link-arg=-Ttext=0x00200800 -C link-arg=--fatal-warnings -C panic=abort --cfg getrandom_backend=\"custom\"";
const DEFAULT_SP1_RUSTFLAGS: &str = "-C passes=lower-atomic -C link-arg=-Ttext=0x00200800 -C panic=abort --cfg getrandom_backend=\"custom\"";
const DEFAULT_RISC0_TOOLCHAIN_IMAGE: &str = "raiko2-risc0-toolchain:local";
const DEFAULT_SP1_TOOLCHAIN_IMAGE: &str = "raiko2-sp1-toolchain:local";
const DEFAULT_RISC0_GUEST_BUILDER_TAG: &str = "r0.1.91.1";
const RISC0_GUEST_BUILDER_TAG_LABEL: &str = "org.raiko2.risc0.guest-builder-tag";
const DEFAULT_RISC0_CC: &str = "/root/.risc0/toolchains/v2024.1.5-cpp-x86_64-unknown-linux-gnu/riscv32im-linux-x86_64/bin/riscv32-unknown-elf-gcc";
const DEFAULT_RISC0_CXX: &str = "/root/.risc0/toolchains/v2024.1.5-cpp-x86_64-unknown-linux-gnu/riscv32im-linux-x86_64/bin/riscv32-unknown-elf-g++";
const DEFAULT_RISC0_AR: &str = "/root/.risc0/toolchains/v2024.1.5-cpp-x86_64-unknown-linux-gnu/riscv32im-linux-x86_64/bin/riscv32-unknown-elf-ar";
const SP1_TARGET_RUSTFLAGS_ENV: &str = "CARGO_TARGET_RISCV64IM_SUCCINCT_ZKVM_ELF_RUSTFLAGS";
const DEFAULT_SP1_CC_ENV: &str = "CC_riscv64im_succinct_zkvm_elf";
const DEFAULT_SP1_CXX_ENV: &str = "CXX_riscv64im_succinct_zkvm_elf";
const DEFAULT_SP1_AR_ENV: &str = "AR_riscv64im_succinct_zkvm_elf";
const DEFAULT_SP1_CC: &str = "riscv64-unknown-elf-gcc -specs=picolibc.specs";
const DEFAULT_SP1_CXX: &str = "riscv64-unknown-elf-g++ -specs=picolibcpp.specs";
const DEFAULT_SP1_AR: &str = "riscv64-unknown-elf-ar";
const HOST_LAUNCHER_PROFILE_OVERRIDES: &[&str] = &[
    "CARGO_PROFILE_RELEASE_LTO",
    "CARGO_PROFILE_RELEASE_OPT_LEVEL",
    "CARGO_PROFILE_RELEASE_DEBUG",
];

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Risc0,
    Sp1,
    All,
}

#[derive(Args)]
pub struct BuildGuestArgs {
    #[arg(value_enum)]
    pub backend: Backend,
    /// Include benchmark binaries (requires bins in Cargo.toml).
    #[arg(long)]
    pub bench: bool,
    /// Force rebuilding even when guest inputs and checked-in outputs match.
    #[arg(long)]
    pub force: bool,
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask-build-guest lives under xtask/")
        .to_path_buf()
}

pub fn run(root: &Path, args: BuildGuestArgs) -> Result<()> {
    refresh_release_guest_elves(root, args.backend, args.bench, None, args.force)?;
    println!("[INFO] Build complete!");
    Ok(())
}

pub fn build(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
) -> Result<()> {
    match backend {
        Backend::Risc0 => {
            build_risc0(root, bench)?;
        }
        Backend::Sp1 => {
            build_sp1(root, bench, sp1_docker_tag)?;
        }
        Backend::All => {
            build_risc0(root, bench)?;
            build_sp1(root, bench, sp1_docker_tag)?;
        }
    }
    Ok(())
}

pub fn resolve_sp1_docker_tag(root: &Path, override_tag: Option<&str>) -> String {
    let override_tag = override_tag.and_then(non_empty_str);
    let env_tag = env::var("SP1_DOCKER_TAG").ok().and_then(non_empty);
    let lock_tag = default_sp1_docker_tag(root);
    override_tag
        .or(env_tag)
        .or(lock_tag)
        .unwrap_or_else(|| "v5.2.4".to_string())
}

#[derive(Deserialize)]
struct CargoManifest {
    package: PackageSection,
    #[serde(default)]
    bin: Vec<BinSection>,
}

#[derive(Deserialize)]
struct PackageSection {
    name: String,
}

#[derive(Deserialize)]
struct BinSection {
    name: String,
}

#[derive(Deserialize)]
struct CargoLockFile {
    #[serde(default)]
    package: Vec<CargoLockPackage>,
}

#[derive(Deserialize)]
struct CargoLockPackage {
    name: String,
    version: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GuestBuildFingerprint {
    backend: String,
    bench: bool,
    fingerprint: String,
}

pub fn ensure_release_guest_elves(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
) -> Result<()> {
    refresh_release_guest_elves(root, backend, bench, sp1_docker_tag, false)
}

fn refresh_release_guest_elves(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
    force: bool,
) -> Result<()> {
    match backend {
        Backend::Risc0 => {
            ensure_release_backend(root, Backend::Risc0, bench, sp1_docker_tag, force)
        }
        Backend::Sp1 => ensure_release_backend(root, Backend::Sp1, bench, sp1_docker_tag, force),
        Backend::All => {
            ensure_release_backend(root, Backend::Risc0, bench, sp1_docker_tag, force)?;
            ensure_release_backend(root, Backend::Sp1, bench, sp1_docker_tag, force)
        }
    }
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn non_empty_str(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn default_sp1_docker_tag(root: &Path) -> Option<String> {
    let workspace_lock_path = root.join("Cargo.lock");
    if let Some(version) = lock_package_version(
        &workspace_lock_path,
        &[
            "sp1-build",
            "sp1-prover",
            "sp1-core-executor",
            "sp1-zkvm",
            "sp1-sdk",
        ],
    ) {
        return Some(format!("v{version}"));
    }

    let lock_path = root.join("guests/sp1/Cargo.lock");
    lock_package_version(&lock_path, &["sp1-zkvm", "sp1-sdk"]).map(|version| format!("v{version}"))
}

fn lock_package_version(lock_path: &Path, package_names: &[&str]) -> Option<String> {
    let contents = fs::read_to_string(lock_path).ok()?;
    let lock: CargoLockFile = toml::from_str(&contents).ok()?;
    package_names.iter().find_map(|package_name| {
        lock.package
            .iter()
            .find(|pkg| pkg.name == *package_name)
            .map(|pkg| pkg.version.clone())
    })
}

fn ensure_release_backend(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
    force: bool,
) -> Result<()> {
    let started = Instant::now();
    let backend_key = match backend {
        Backend::Risc0 => "risc0",
        Backend::Sp1 => "sp1",
        Backend::All => unreachable!("release backend cache is evaluated per concrete backend"),
    };
    let fingerprint_path = guest_fingerprint_path(root, backend_key);

    if !force {
        let outputs_exist = guest_outputs_exist(root, backend)?;
        let fingerprint =
            compute_guest_fingerprint(root, backend, bench, sp1_docker_tag, outputs_exist)?;
        if outputs_exist
            && matches_existing_fingerprint(&fingerprint_path, backend_key, bench, &fingerprint)?
        {
            println!(
                "[INFO] Guest ELFs for backend `{backend_key}` are up to date; skipping rebuild after {}.",
                util::format_duration(started.elapsed())
            );
            return Ok(());
        }
    }

    if force {
        println!(
            "[INFO] Rebuilding guest ELFs for backend `{backend_key}` because --force was passed..."
        );
    } else {
        println!(
            "[INFO] Rebuilding guest ELFs for backend `{backend_key}` because sources or build inputs changed..."
        );
    }
    build(root, backend, bench, sp1_docker_tag)?;
    let fingerprint = compute_guest_fingerprint(root, backend, bench, sp1_docker_tag, true)?;
    write_guest_fingerprint(&fingerprint_path, backend_key, bench, &fingerprint)?;
    println!(
        "[INFO] Guest ELFs for backend `{backend_key}` refreshed in {}.",
        util::format_duration(started.elapsed())
    );
    Ok(())
}

fn guest_fingerprint_path(root: &Path, backend_key: &str) -> PathBuf {
    util::target_root(root)
        .join("xtask/guest-fingerprints")
        .join(format!("{backend_key}.json"))
}

fn matches_existing_fingerprint(
    fingerprint_path: &Path,
    backend_key: &str,
    bench: bool,
    fingerprint: &str,
) -> Result<bool> {
    if !fingerprint_path.exists() {
        return Ok(false);
    }
    let contents = fs::read_to_string(fingerprint_path)
        .with_context(|| format!("read guest fingerprint {fingerprint_path:?}"))?;
    let existing: GuestBuildFingerprint = serde_json::from_str(&contents)
        .with_context(|| format!("parse guest fingerprint {fingerprint_path:?}"))?;
    Ok(existing.backend == backend_key
        && existing.bench == bench
        && existing.fingerprint == fingerprint)
}

fn write_guest_fingerprint(
    fingerprint_path: &Path,
    backend_key: &str,
    bench: bool,
    fingerprint: &str,
) -> Result<()> {
    if let Some(parent) = fingerprint_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create guest fingerprint directory {parent:?}"))?;
    }
    let payload = GuestBuildFingerprint {
        backend: backend_key.to_string(),
        bench,
        fingerprint: fingerprint.to_string(),
    };
    let bytes = serde_json::to_vec_pretty(&payload)
        .with_context(|| format!("serialize guest fingerprint {fingerprint_path:?}"))?;
    fs::write(fingerprint_path, bytes)
        .with_context(|| format!("write guest fingerprint {fingerprint_path:?}"))
}

fn guest_outputs_exist(root: &Path, backend: Backend) -> Result<bool> {
    for output in expected_guest_outputs(root, backend)? {
        if !output.exists() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn expected_guest_outputs(root: &Path, backend: Backend) -> Result<Vec<PathBuf>> {
    let mut outputs = Vec::new();
    match backend {
        Backend::Risc0 => outputs.extend(expected_backend_outputs(root, "risc0")?),
        Backend::Sp1 => outputs.extend(expected_backend_outputs(root, "sp1")?),
        Backend::All => {
            outputs.extend(expected_backend_outputs(root, "risc0")?);
            outputs.extend(expected_backend_outputs(root, "sp1")?);
        }
    }
    Ok(outputs)
}

fn expected_backend_outputs(root: &Path, backend_key: &str) -> Result<Vec<PathBuf>> {
    let manifest = read_manifest(&root.join(format!("guests/{backend_key}/Cargo.toml")))?;
    Ok(manifest
        .bin
        .iter()
        .map(|bin| {
            root.join("crates/guests/elf")
                .join(format!("{}.elf", bin.name.replace('-', "_")))
        })
        .collect())
}

fn compute_guest_fingerprint(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
    include_outputs: bool,
) -> Result<String> {
    let mut hasher = Sha256::new();
    hash_tagged_bytes(&mut hasher, "bench", if bench { b"1" } else { b"0" });

    match backend {
        Backend::Risc0 => {
            compute_backend_fingerprint(root, &mut hasher, "risc0", None, include_outputs)?
        }
        Backend::Sp1 => compute_backend_fingerprint(
            root,
            &mut hasher,
            "sp1",
            Some(resolve_sp1_docker_tag(root, sp1_docker_tag)),
            include_outputs,
        )?,
        Backend::All => unreachable!("fingerprints are computed per concrete backend"),
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn compute_backend_fingerprint(
    root: &Path,
    hasher: &mut Sha256,
    backend_key: &str,
    sp1_tag: Option<String>,
    include_outputs: bool,
) -> Result<()> {
    hash_tagged_bytes(hasher, "backend", backend_key.as_bytes());
    if let Some(tag) = sp1_tag {
        hash_tagged_bytes(hasher, "sp1_docker_tag", tag.as_bytes());
    }
    hash_backend_env(hasher, backend_key);

    let mut paths = vec![
        root.join("rust-toolchain.toml"),
        root.join("xtask/build-guest/Cargo.toml"),
        root.join("xtask/build-guest/src/lib.rs"),
        root.join("xtask/build-guest/src/main.rs"),
        root.join("xtask/build-guest/src/util.rs"),
        root.join("crates/guest-common/Cargo.toml"),
        root.join(format!("guests/{backend_key}/Cargo.toml")),
        root.join(format!("guests/{backend_key}/Cargo.lock")),
        root.join(format!("docker/{backend_key}-toolchain/Dockerfile")),
    ];
    collect_files_recursively(&root.join("crates/guest-common/src"), &mut paths)?;
    collect_files_recursively(&root.join(format!("guests/{backend_key}/src")), &mut paths)?;
    paths.sort();

    for path in paths {
        hash_file(root, hasher, &path)?;
    }

    if include_outputs {
        for output in expected_backend_outputs(root, backend_key)? {
            hash_file_with_tags(root, hasher, &output, "output_path", "output_file")?;
        }
    }

    Ok(())
}

fn hash_backend_env(hasher: &mut Sha256, backend_key: &str) {
    match backend_key {
        "risc0" => {
            hash_effective_env(
                hasher,
                "RISC0_TOOLCHAIN_IMAGE",
                env::var("RISC0_TOOLCHAIN_IMAGE")
                    .unwrap_or_else(|_| DEFAULT_RISC0_TOOLCHAIN_IMAGE.to_string())
                    .trim(),
            );
            hash_effective_env(
                hasher,
                "RISC0_DOCKER_CONTAINER_TAG",
                env::var("RISC0_DOCKER_CONTAINER_TAG").unwrap_or_default(),
            );
            hash_effective_env(
                hasher,
                "RISC0_GUEST_RUSTFLAGS",
                env::var("RISC0_GUEST_RUSTFLAGS")
                    .unwrap_or_else(|_| DEFAULT_RISC0_RUSTFLAGS.to_string()),
            );
            hash_effective_env(
                hasher,
                "RISC0_GUEST_CC",
                env::var("RISC0_GUEST_CC").unwrap_or_default(),
            );
            hash_effective_env(
                hasher,
                "RISC0_GUEST_CFLAGS",
                env::var("RISC0_GUEST_CFLAGS").unwrap_or_default(),
            );
            hash_effective_env(
                hasher,
                "DOCKER_DEFAULT_PLATFORM",
                env::var("DOCKER_DEFAULT_PLATFORM").unwrap_or_default(),
            );
            hash_effective_env(
                hasher,
                "RISC0_DEV_MODE",
                effective_mock_env("RISC0_DEV_MODE", "1"),
            );
        }
        "sp1" => {
            hash_effective_env(
                hasher,
                "SP1_TOOLCHAIN_IMAGE",
                env::var("SP1_TOOLCHAIN_IMAGE")
                    .unwrap_or_else(|_| DEFAULT_SP1_TOOLCHAIN_IMAGE.to_string())
                    .trim(),
            );
            hash_effective_env(
                hasher,
                "SP1_GUEST_RUSTFLAGS",
                env::var("SP1_GUEST_RUSTFLAGS")
                    .unwrap_or_else(|_| DEFAULT_SP1_RUSTFLAGS.to_string()),
            );
            hash_effective_env(
                hasher,
                "SP1_GUEST_CC",
                env::var("SP1_GUEST_CC").unwrap_or_default(),
            );
            hash_effective_env(
                hasher,
                "SP1_GUEST_CFLAGS",
                env::var("SP1_GUEST_CFLAGS").unwrap_or_default(),
            );
            hash_effective_env(
                hasher,
                "DOCKER_DEFAULT_PLATFORM",
                env::var("DOCKER_DEFAULT_PLATFORM").unwrap_or_default(),
            );
            hash_effective_env(
                hasher,
                "SP1_PROVER",
                effective_mock_env("SP1_PROVER", "mock"),
            );
        }
        _ => unreachable!("unknown guest backend {backend_key}"),
    }
}

fn effective_mock_env(name: &str, mock_value: &str) -> String {
    if env::var("MOCK").ok().as_deref() == Some("1") {
        mock_value.to_string()
    } else {
        env::var(name).unwrap_or_default()
    }
}

fn hash_effective_env(hasher: &mut Sha256, key: &str, value: impl AsRef<str>) {
    hash_tagged_bytes(hasher, "env_key", key.as_bytes());
    hash_tagged_bytes(hasher, "env_value", value.as_ref().as_bytes());
}

fn collect_files_recursively(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).with_context(|| format!("read directory {dir:?}"))? {
        let entry = entry.with_context(|| format!("read entry under {dir:?}"))?;
        let path = entry.path();
        if entry
            .file_type()
            .with_context(|| format!("read file type for {path:?}"))?
            .is_dir()
        {
            collect_files_recursively(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn hash_file(root: &Path, hasher: &mut Sha256, path: &Path) -> Result<()> {
    hash_file_with_tags(root, hasher, path, "path", "file")
}

fn hash_file_with_tags(
    root: &Path,
    hasher: &mut Sha256,
    path: &Path,
    path_tag: &str,
    bytes_tag: &str,
) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned();
    hash_tagged_bytes(hasher, path_tag, relative.as_bytes());

    let bytes = fs::read(path).with_context(|| format!("read fingerprint input {path:?}"))?;
    hash_tagged_bytes(hasher, bytes_tag, &bytes);
    Ok(())
}

fn hash_tagged_bytes(hasher: &mut Sha256, tag: &str, bytes: &[u8]) {
    hasher.update(tag.as_bytes());
    hasher.update([0]);
    hasher.update(bytes.len().to_le_bytes());
    hasher.update(bytes);
    hasher.update([0xff]);
}

fn build_risc0(root: &Path, bench: bool) -> Result<()> {
    let started = Instant::now();
    println!("[INFO] Building RISC0 guest programs...");
    util::ensure_docker()?;
    let toolchain_image = env::var("RISC0_TOOLCHAIN_IMAGE")
        .unwrap_or_else(|_| DEFAULT_RISC0_TOOLCHAIN_IMAGE.to_string());
    let toolchain_image = toolchain_image.trim();
    if toolchain_image == DEFAULT_RISC0_TOOLCHAIN_IMAGE {
        ensure_local_risc0_toolchain_image(root, toolchain_image)?;
    }
    if !toolchain_image.is_empty()
        && !toolchain_image.eq_ignore_ascii_case("local")
        && !toolchain_image.eq_ignore_ascii_case("none")
    {
        return build_risc0_with_toolchain_image(root, bench, toolchain_image);
    }
    util::ensure_cargo_risczero()?;

    let profile = env::var("PROFILE").unwrap_or_else(|_| "release".to_string());
    if profile != "release" {
        println!("[WARN] PROFILE={profile} is ignored by cargo risczero; building default profile");
    }
    let manifest_path = root.join("guests/risc0/Cargo.toml");
    let manifest = read_manifest(&manifest_path)?;
    if env::var("VERBOSE").ok().as_deref() == Some("1") {
        println!("[WARN] VERBOSE=1 is ignored by cargo risczero build");
    }

    let mut cmd = Command::new("cargo");
    cmd.arg("risczero")
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest_path);
    if bench {
        cmd.arg("--features").arg("bench");
    }

    let target_root = util::target_root(root).join("risc0");
    cmd.env("CARGO_TARGET_DIR", &target_root);
    clear_host_launcher_profile_overrides(&mut cmd);

    let risc0_docker_tag = env::var("RISC0_DOCKER_CONTAINER_TAG")
        .ok()
        .and_then(non_empty);
    let rustflags =
        env::var("RISC0_GUEST_RUSTFLAGS").unwrap_or_else(|_| DEFAULT_RISC0_RUSTFLAGS.to_string());
    cmd.env("CARGO_TARGET_RISCV32IM_RISC0_ZKVM_ELF_RUSTFLAGS", rustflags);
    cmd.env("RISC0_FEATURE_bigint2", "1");
    cmd.env("CC_riscv32im_risc0_zkvm_elf", DEFAULT_RISC0_CC);
    cmd.env("CXX_riscv32im_risc0_zkvm_elf", DEFAULT_RISC0_CXX);
    cmd.env("AR_riscv32im_risc0_zkvm_elf", DEFAULT_RISC0_AR);
    if let Some(tag) = risc0_docker_tag.as_deref() {
        cmd.env("RISC0_DOCKER_CONTAINER_TAG", tag);
        println!("[INFO] RISC0 docker tag override: {tag}");
    }

    if let Ok(cc) = env::var("RISC0_GUEST_CC")
        && !cc.is_empty()
    {
        cmd.env("CC", cc);
    }
    if let Ok(cflags) = env::var("RISC0_GUEST_CFLAGS")
        && !cflags.is_empty()
    {
        cmd.env("CFLAGS", cflags);
    }
    if let Ok(platform) = env::var("DOCKER_DEFAULT_PLATFORM")
        && !platform.is_empty()
    {
        cmd.env("DOCKER_DEFAULT_PLATFORM", platform);
    }
    if env::var("MOCK").ok().as_deref() == Some("1") {
        cmd.env("RISC0_DEV_MODE", "1");
        println!("[INFO] RISC0_DEV_MODE enabled");
    }

    println!("[INFO] Building RISC0 guest package (docker via cargo risczero)...");
    util::run(cmd)?;

    let export_started = Instant::now();
    export_risc0_elves(root, &manifest, &target_root)?;
    println!(
        "[INFO] RISC0 guest build complete in {} (export {}).",
        util::format_duration(started.elapsed()),
        util::format_duration(export_started.elapsed())
    );
    Ok(())
}

fn build_risc0_with_toolchain_image(root: &Path, bench: bool, image: &str) -> Result<()> {
    let started = Instant::now();
    println!("[INFO] Using RISC0 toolchain image: {image}");

    let profile = env::var("PROFILE").unwrap_or_else(|_| "release".to_string());
    if profile != "release" {
        println!("[WARN] PROFILE={profile} is ignored by cargo risczero; building default profile");
    }
    if bench {
        println!(
            "[WARN] --bench has no effect unless extra bins are defined in guests/risc0/Cargo.toml"
        );
    }
    if env::var("VERBOSE").ok().as_deref() == Some("1") {
        println!("[WARN] VERBOSE=1 is ignored by cargo risczero build");
    }

    let risc0_docker_tag = env::var("RISC0_DOCKER_CONTAINER_TAG")
        .ok()
        .and_then(non_empty);
    let rustflags =
        env::var("RISC0_GUEST_RUSTFLAGS").unwrap_or_else(|_| DEFAULT_RISC0_RUSTFLAGS.to_string());

    let manifest_path = root.join("guests/risc0/Cargo.toml");
    let manifest = read_manifest(&manifest_path)?;
    let container_manifest_path = manifest_path
        .strip_prefix(root)
        .map(|rel| PathBuf::from("/work").join(rel))
        .unwrap_or_else(|_| PathBuf::from("/work/guests/risc0/Cargo.toml"));

    let target_root = util::target_root(root).join("risc0");
    let (container_target_dir, extra_mount) = match target_root.strip_prefix(root).ok() {
        Some(rel) => (PathBuf::from("/work").join(rel), None),
        None => (PathBuf::from("/target"), Some(target_root.clone())),
    };

    let mut cmd = Command::new("docker");
    cmd.arg("run").arg("--rm").arg("--entrypoint").arg("");
    cmd.arg("-v")
        .arg(format!("{}:/work", root.display()))
        .arg("-w")
        .arg("/work");

    if let Some(volume) = util::docker_cargo_cache_volume(root, "risc0")? {
        println!("[INFO] Using docker cargo cache volume: {volume}");
        cmd.arg("-v")
            .arg(format!("{volume}:{}", util::DOCKER_CARGO_HOME));
        cmd.arg("-e")
            .arg(format!("CARGO_HOME={}", util::DOCKER_CARGO_HOME));
    }

    let sccache_enabled = configure_docker_sccache(&mut cmd, root, "risc0")?;

    if let Some(extra_mount) = &extra_mount {
        cmd.arg("-v")
            .arg(format!("{}:/target", extra_mount.display()));
    }

    cmd.arg("-e")
        .arg(format!(
            "CARGO_TARGET_DIR={}",
            container_target_dir.display()
        ))
        .arg("-e")
        .arg(format!(
            "CARGO_TARGET_RISCV32IM_RISC0_ZKVM_ELF_RUSTFLAGS={rustflags}"
        ))
        .arg("-e")
        .arg("RISC0_FEATURE_bigint2=1")
        .arg("-e")
        .arg(format!(
            "CC_riscv32im_risc0_zkvm_elf={}",
            sccache_compiler(sccache_enabled, DEFAULT_RISC0_CC)
        ))
        .arg("-e")
        .arg(format!(
            "CXX_riscv32im_risc0_zkvm_elf={}",
            sccache_compiler(sccache_enabled, DEFAULT_RISC0_CXX)
        ))
        .arg("-e")
        .arg(format!("AR_riscv32im_risc0_zkvm_elf={DEFAULT_RISC0_AR}"));
    if let Some(tag) = risc0_docker_tag.as_deref() {
        cmd.arg("-e")
            .arg(format!("RISC0_DOCKER_CONTAINER_TAG={tag}"));
        println!("[INFO] RISC0 docker tag override: {tag}");
    }

    if let Ok(cc) = env::var("RISC0_GUEST_CC")
        && !cc.is_empty()
    {
        cmd.arg("-e").arg(format!("CC={cc}"));
    }
    if let Ok(cflags) = env::var("RISC0_GUEST_CFLAGS")
        && !cflags.is_empty()
    {
        cmd.arg("-e").arg(format!("CFLAGS={cflags}"));
    }
    if let Ok(platform) = env::var("DOCKER_DEFAULT_PLATFORM")
        && !platform.is_empty()
    {
        cmd.arg("-e")
            .arg(format!("DOCKER_DEFAULT_PLATFORM={platform}"));
    }
    if env::var("MOCK").ok().as_deref() == Some("1") {
        cmd.arg("-e").arg("RISC0_DEV_MODE=1");
        println!("[INFO] RISC0_DEV_MODE enabled");
    }

    cmd.arg(image);
    if sccache_enabled {
        cmd.arg("sh").arg("-lc").arg(format!(
            "sccache --zero-stats || true\n\
             cargo +risc0 build --release --ignore-rust-version --locked \
             --target riscv32im-risc0-zkvm-elf --manifest-path {}{}\n\
             status=$?\n\
             sccache --show-stats || true\n\
             exit \"$status\"",
            container_manifest_path.display(),
            if bench { " --features bench" } else { "" }
        ));
    } else {
        cmd.arg("cargo")
            .arg("+risc0")
            .arg("build")
            .arg("--release")
            .arg("--ignore-rust-version")
            .arg("--locked")
            .arg("--target")
            .arg("riscv32im-risc0-zkvm-elf")
            .arg("--manifest-path")
            .arg(&container_manifest_path);
        if bench {
            cmd.arg("--features").arg("bench");
        }
    }

    println!("[INFO] Building RISC0 guest package (toolchain image)...");
    util::run(cmd)?;
    util::restore_docker_ownership(
        image,
        root,
        extra_mount.as_deref(),
        &[target_root.as_path()],
    )?;

    let export_started = Instant::now();
    export_risc0_elves(root, &manifest, &target_root)?;
    println!(
        "[INFO] RISC0 guest build complete in {} (export {}).",
        util::format_duration(started.elapsed()),
        util::format_duration(export_started.elapsed())
    );
    Ok(())
}

fn export_risc0_elves(root: &Path, manifest: &CargoManifest, target_root: &Path) -> Result<()> {
    let output_dir = root.join("crates/guests/elf");
    fs::create_dir_all(&output_dir)?;

    let release_dir = target_root.join("riscv32im-risc0-zkvm-elf/release");
    let target_dir = target_root.join("riscv32im-risc0-zkvm-elf/docker");
    let legacy_dir = target_root
        .join("riscv-guest/riscv32im-risc0-zkvm-elf/docker")
        .join(&manifest.package.name);

    if manifest.bin.is_empty() {
        bail!("No [[bin]] targets found in guests/risc0/Cargo.toml");
    }

    for bin in &manifest.bin {
        let elf_name = format!("{}.elf", bin.name.replace('-', "_"));
        let candidates = [
            release_dir.join(&bin.name),
            release_dir.join(format!("{}.elf", bin.name)),
            release_dir.join(format!("{}.bin", bin.name)),
            target_dir.join(format!("{}.bin", bin.name)),
            target_dir.join(&bin.name),
            target_dir.join(format!("{}.elf", bin.name)),
            legacy_dir.join(format!("{}.bin", bin.name)),
            legacy_dir.join(&bin.name),
            legacy_dir.join(format!("{}.elf", bin.name)),
        ];

        let mut copied = false;
        for candidate in candidates.iter() {
            if candidate.exists() {
                export_risc0_binary(candidate, &output_dir.join(&elf_name))
                    .with_context(|| format!("export {candidate:?} -> {elf_name}"))?;
                println!("[INFO] Exported {elf_name}");
                copied = true;
                break;
            }
        }

        if !copied {
            bail!(
                "Missing ELF for {} (checked {:?}, {:?}, and {:?})",
                bin.name,
                release_dir,
                target_dir,
                legacy_dir
            );
        }
    }

    Ok(())
}

fn export_risc0_binary(source: &Path, destination: &Path) -> Result<()> {
    let bytes = fs::read(source).with_context(|| format!("read {source:?}"))?;
    let output = if ProgramBinary::decode(&bytes).is_ok() {
        bytes
    } else {
        ProgramBinary::new(&bytes, V1COMPAT_ELF).encode()
    };
    fs::write(destination, output).with_context(|| format!("write {destination:?}"))
}

fn build_sp1(root: &Path, bench: bool, sp1_docker_tag: Option<&str>) -> Result<()> {
    let started = Instant::now();
    println!("[INFO] Building SP1 guest programs...");
    util::ensure_docker()?;

    let sp1_tag = resolve_sp1_docker_tag(root, sp1_docker_tag);
    let toolchain_image =
        env::var("SP1_TOOLCHAIN_IMAGE").unwrap_or_else(|_| DEFAULT_SP1_TOOLCHAIN_IMAGE.to_string());
    let toolchain_image = toolchain_image.trim();
    if toolchain_image == DEFAULT_SP1_TOOLCHAIN_IMAGE {
        ensure_local_sp1_toolchain_image(root, toolchain_image, &sp1_tag)?;
    }
    if !toolchain_image.is_empty()
        && !toolchain_image.eq_ignore_ascii_case("local")
        && !toolchain_image.eq_ignore_ascii_case("none")
    {
        return build_sp1_with_toolchain_image(root, bench, toolchain_image);
    }

    util::ensure_cargo_prove()?;

    let profile = env::var("PROFILE").unwrap_or_else(|_| "release".to_string());
    if profile != "release" {
        println!("[WARN] PROFILE={profile} is ignored by cargo prove; building default profile");
    }
    let manifest_path = root.join("guests/sp1/Cargo.toml");
    let manifest = read_manifest(&manifest_path)?;
    let export_dir = util::target_root(root).join("sp1-export");
    let output_dir = root.join("crates/guests/elf");
    fs::create_dir_all(&export_dir)?;
    fs::create_dir_all(&output_dir)?;

    println!("[INFO] SP1 docker tag: {sp1_tag}");

    if manifest.bin.is_empty() {
        bail!("No [[bin]] targets found in guests/sp1/Cargo.toml");
    }

    println!("[INFO] Building SP1 guest binaries in a single cargo prove invocation...");

    let mut cmd = Command::new("cargo");
    cmd.current_dir(root.join("guests/sp1"));
    clear_host_launcher_profile_overrides(&mut cmd);
    cmd.arg("prove")
        .arg("build")
        .arg("--docker")
        .arg("--tag")
        .arg(&sp1_tag)
        .arg("--ignore-rust-version");
    if bench {
        cmd.arg("--features").arg("bench");
    }
    for bin in &manifest.bin {
        cmd.arg("--binaries").arg(&bin.name);
    }
    cmd.arg("--output-directory")
        .arg(&export_dir)
        .arg("--locked")
        .arg("--workspace-directory")
        .arg(root);

    let rustflags =
        env::var("SP1_GUEST_RUSTFLAGS").unwrap_or_else(|_| DEFAULT_SP1_RUSTFLAGS.to_string());
    cmd.env(SP1_TARGET_RUSTFLAGS_ENV, rustflags);

    if let Ok(cc) = env::var("SP1_GUEST_CC")
        && !cc.is_empty()
    {
        cmd.env("CC", cc);
    }
    if let Ok(cflags) = env::var("SP1_GUEST_CFLAGS")
        && !cflags.is_empty()
    {
        cmd.env("CFLAGS", cflags);
    }
    if let Ok(platform) = env::var("DOCKER_DEFAULT_PLATFORM")
        && !platform.is_empty()
    {
        cmd.env("DOCKER_DEFAULT_PLATFORM", platform);
    }
    if env::var("MOCK").ok().as_deref() == Some("1") {
        cmd.env("SP1_PROVER", "mock");
        println!("[INFO] SP1_PROVER=mock enabled");
    }
    if env::var("VERBOSE").ok().as_deref() == Some("1") {
        println!("[WARN] VERBOSE=1 is ignored by cargo prove build");
    }

    util::run(cmd)?;
    let export_started = Instant::now();
    export_sp1_elves(&manifest, &export_dir, &output_dir)?;

    println!(
        "[INFO] SP1 guest build complete in {} (export {}).",
        util::format_duration(started.elapsed()),
        util::format_duration(export_started.elapsed())
    );
    Ok(())
}

fn build_sp1_with_toolchain_image(root: &Path, bench: bool, image: &str) -> Result<()> {
    let started = Instant::now();
    println!("[INFO] Using SP1 toolchain image: {image}");

    let profile = env::var("PROFILE").unwrap_or_else(|_| "release".to_string());
    if profile != "release" {
        println!("[WARN] PROFILE={profile} is ignored by cargo prove; building default profile");
    }
    if env::var("VERBOSE").ok().as_deref() == Some("1") {
        println!("[WARN] VERBOSE=1 is ignored by cargo prove build");
    }

    let manifest_path = root.join("guests/sp1/Cargo.toml");
    let manifest = read_manifest(&manifest_path)?;
    if manifest.bin.is_empty() {
        bail!("No [[bin]] targets found in guests/sp1/Cargo.toml");
    }

    let target_root = util::target_root(root).join("sp1");
    let export_dir = target_root.join("sp1-export");
    let output_dir = root.join("crates/guests/elf");
    fs::create_dir_all(&export_dir)?;
    fs::create_dir_all(&output_dir)?;

    let (container_target_dir, extra_mount) = match target_root.strip_prefix(root).ok() {
        Some(rel) => (PathBuf::from("/work").join(rel), None),
        None => (PathBuf::from("/target"), Some(target_root.clone())),
    };

    let container_export_dir = container_target_dir.join("sp1-export");

    let rustflags =
        env::var("SP1_GUEST_RUSTFLAGS").unwrap_or_else(|_| DEFAULT_SP1_RUSTFLAGS.to_string());

    let mut cmd = Command::new("docker");
    cmd.arg("run").arg("--rm");
    if let Ok(platform) = env::var("DOCKER_DEFAULT_PLATFORM")
        && !platform.is_empty()
    {
        cmd.arg("--platform").arg(platform);
    }
    cmd.arg("-v")
        .arg(format!("{}:/work", root.display()))
        .arg("-w")
        .arg("/work/guests/sp1");

    if let Some(volume) = util::docker_cargo_cache_volume(root, "sp1")? {
        println!("[INFO] Using docker cargo cache volume: {volume}");
        cmd.arg("-v")
            .arg(format!("{volume}:{}", util::DOCKER_CARGO_HOME));
        cmd.arg("-e")
            .arg(format!("CARGO_HOME={}", util::DOCKER_CARGO_HOME));
    }

    let sccache_enabled = configure_docker_sccache(&mut cmd, root, "sp1")?;

    if let Some(extra_mount) = &extra_mount {
        cmd.arg("-v")
            .arg(format!("{}:/target", extra_mount.display()));
    }

    cmd.arg("-e")
        .arg(format!(
            "CARGO_TARGET_DIR={}",
            container_target_dir.display()
        ))
        .arg("-e")
        .arg(format!("{SP1_TARGET_RUSTFLAGS_ENV}={rustflags}"));

    cmd.arg("-e").arg(format!(
        "{DEFAULT_SP1_CC_ENV}={}",
        sccache_compiler(sccache_enabled, DEFAULT_SP1_CC)
    ));
    cmd.arg("-e").arg(format!(
        "{DEFAULT_SP1_CXX_ENV}={}",
        sccache_compiler(sccache_enabled, DEFAULT_SP1_CXX)
    ));
    cmd.arg("-e")
        .arg(format!("{DEFAULT_SP1_AR_ENV}={DEFAULT_SP1_AR}"));

    if let Ok(cc) = env::var("SP1_GUEST_CC")
        && !cc.is_empty()
    {
        cmd.arg("-e").arg(format!("CC={cc}"));
    }
    if let Ok(cflags) = env::var("SP1_GUEST_CFLAGS")
        && !cflags.is_empty()
    {
        cmd.arg("-e").arg(format!("CFLAGS={cflags}"));
    }

    let mut script = String::from("set -u\n");
    if sccache_enabled {
        script.push_str("sccache --zero-stats || true\n");
    }
    script.push_str("cargo prove build --ignore-rust-version ");
    if bench {
        script.push_str("--features bench ");
    }
    for bin in &manifest.bin {
        script.push_str("--binaries ");
        script.push_str(&bin.name);
        script.push(' ');
    }
    script.push_str("--output-directory ");
    script.push_str(container_export_dir.to_string_lossy().as_ref());
    script.push_str(" --locked --workspace-directory /work\n");
    if sccache_enabled {
        script.push_str("status=$?\n");
        script.push_str("sccache --show-stats || true\n");
        script.push_str("exit \"$status\"\n");
    }

    cmd.arg(image).arg("sh").arg("-lc").arg(script);

    println!("[INFO] Building SP1 guest package (toolchain image)...");
    util::run(cmd)?;
    util::restore_docker_ownership(
        image,
        root,
        extra_mount.as_deref(),
        &[target_root.as_path()],
    )?;
    let export_started = Instant::now();
    export_sp1_elves(&manifest, &export_dir, &output_dir)?;

    println!(
        "[INFO] SP1 guest build complete in {} (export {}).",
        util::format_duration(started.elapsed()),
        util::format_duration(export_started.elapsed())
    );
    Ok(())
}

fn configure_docker_sccache(cmd: &mut Command, root: &Path, backend_key: &str) -> Result<bool> {
    let Some(volume) = util::docker_sccache_cache_volume(root, backend_key)? else {
        println!("[INFO] Docker sccache cache disabled for backend `{backend_key}`");
        return Ok(false);
    };

    println!("[INFO] Using docker sccache cache volume: {volume}");
    cmd.arg("-v")
        .arg(format!("{volume}:{}", util::DOCKER_SCCACHE_DIR));
    cmd.arg("-e")
        .arg(format!("SCCACHE_DIR={}", util::DOCKER_SCCACHE_DIR));
    cmd.arg("-e").arg("SCCACHE_BASEDIRS=/work");
    cmd.arg("-e").arg("RUSTC_WRAPPER=sccache");
    Ok(true)
}

fn sccache_compiler(enabled: bool, compiler: &str) -> String {
    if enabled {
        format!("sccache {compiler}")
    } else {
        compiler.to_string()
    }
}

fn clear_host_launcher_profile_overrides(cmd: &mut Command) {
    for key in HOST_LAUNCHER_PROFILE_OVERRIDES {
        cmd.env_remove(key);
    }
}

fn export_sp1_elves(manifest: &CargoManifest, export_dir: &Path, output_dir: &Path) -> Result<()> {
    for bin in &manifest.bin {
        let source = export_dir.join(&bin.name);
        let artifact_name = bin.name.replace('-', "_");
        let destination = output_dir.join(format!("{artifact_name}.elf"));
        fs::copy(&source, &destination)
            .with_context(|| format!("export {source:?} -> {destination:?}"))?;
        println!(
            "[INFO] Exported {}",
            destination.file_name().unwrap().to_string_lossy()
        );

        let elf = fs::read(&destination).with_context(|| format!("read {destination:?}"))?;
        let vk_destination = output_dir.join(format!("{artifact_name}.vk.bin"));
        let vk = sp1_vk_bin(elf, artifact_name.clone())?;
        fs::write(&vk_destination, vk).with_context(|| format!("write {vk_destination:?}"))?;
        println!(
            "[INFO] Exported {}",
            vk_destination.file_name().unwrap().to_string_lossy()
        );
    }

    Ok(())
}

fn sp1_vk_bin(elf: Vec<u8>, artifact_name: String) -> Result<Vec<u8>> {
    let panic_artifact = artifact_name.clone();
    let handle = std::thread::Builder::new()
        .name(format!("sp1-vk-{artifact_name}"))
        .spawn(move || {
            let client = ProverClient::builder().cpu().build();
            let pk = client
                .setup(elf.as_slice().into())
                .with_context(|| format!("setup SP1 verifying key for {artifact_name}"))?;
            bincode::serialize(pk.verifying_key())
                .with_context(|| format!("serialize SP1 verifying key for {artifact_name}"))
        })
        .with_context(|| format!("spawn SP1 verifying key setup for {panic_artifact}"))?;

    handle
        .join()
        .map_err(|_| anyhow::anyhow!("SP1 verifying key setup panicked for {panic_artifact}"))?
}

fn ensure_local_sp1_toolchain_image(root: &Path, image: &str, sp1_tag: &str) -> Result<()> {
    let started = Instant::now();
    let mut inspect = Command::new("docker");
    inspect
        .arg("image")
        .arg("inspect")
        .arg(image)
        .arg("--format")
        .arg("{{index .Config.Labels \"org.opencontainers.image.version\"}}");
    inspect.stderr(Stdio::null());
    if let Ok(output) = inspect.output()
        && output.status.success()
    {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if version == sp1_tag {
            println!(
                "[INFO] Local SP1 toolchain image is up to date; checked in {}.",
                util::format_duration(started.elapsed())
            );
            return Ok(());
        }
        println!(
            "[INFO] Rebuilding local SP1 toolchain image: found version {version}, expected {sp1_tag}"
        );
    }

    println!("[INFO] Building local SP1 toolchain image: {image}");
    let mut build = Command::new("docker");
    build
        .arg("build")
        .arg("-f")
        .arg(root.join("docker/sp1-toolchain/Dockerfile"))
        .arg("-t")
        .arg(image)
        .arg("--build-arg")
        .arg(format!("SP1_DOCKER_TAG={sp1_tag}"))
        .arg(root.join("docker/sp1-toolchain"));
    util::run(build)?;
    println!(
        "[INFO] Built local SP1 toolchain image in {}.",
        util::format_duration(started.elapsed())
    );
    Ok(())
}

fn ensure_local_risc0_toolchain_image(root: &Path, image: &str) -> Result<()> {
    let started = Instant::now();
    let mut inspect = Command::new("docker");
    inspect
        .arg("image")
        .arg("inspect")
        .arg(image)
        .arg("--format")
        .arg(format!(
            r#"{{{{index .Config.Labels "{RISC0_GUEST_BUILDER_TAG_LABEL}"}}}}"#
        ))
        .stderr(Stdio::null());
    if let Ok(output) = inspect.output()
        && output.status.success()
    {
        let tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if tag == DEFAULT_RISC0_GUEST_BUILDER_TAG {
            println!(
                "[INFO] Local RISC0 toolchain image is up to date; checked in {}.",
                util::format_duration(started.elapsed())
            );
            return Ok(());
        }
        println!(
            "[INFO] Rebuilding local RISC0 toolchain image: found guest builder tag {tag}, expected {DEFAULT_RISC0_GUEST_BUILDER_TAG}"
        );
    }

    println!("[INFO] Building local RISC0 toolchain image: {image}");
    let mut build = Command::new("docker");
    build
        .arg("build")
        .arg("-f")
        .arg(root.join("docker/risc0-toolchain/Dockerfile"))
        .arg("-t")
        .arg(image)
        .arg("--build-arg")
        .arg(format!(
            "RISC0_GUEST_BUILDER_TAG={DEFAULT_RISC0_GUEST_BUILDER_TAG}"
        ))
        .arg(root.join("docker/risc0-toolchain"));
    util::run(build)?;
    println!(
        "[INFO] Built local RISC0 toolchain image in {}.",
        util::format_duration(started.elapsed())
    );
    Ok(())
}

fn read_manifest(path: &Path) -> Result<CargoManifest> {
    let contents = fs::read_to_string(path).with_context(|| format!("read manifest {path:?}"))?;
    let manifest: CargoManifest =
        toml::from_str(&contents).with_context(|| format!("parse manifest {path:?}"))?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn sp1_guest_fingerprint_changes_with_docker_tag() {
        let root = repo_root();
        let v1 =
            compute_guest_fingerprint(&root, Backend::Sp1, false, Some("v5.2.4"), false).unwrap();
        let v2 =
            compute_guest_fingerprint(&root, Backend::Sp1, false, Some("v5.2.5"), false).unwrap();
        assert_ne!(v1, v2);
    }

    #[test]
    fn export_risc0_elves_fails_when_expected_binary_is_missing() {
        let temp_root = temp_test_dir();
        let manifest = CargoManifest {
            package: PackageSection {
                name: "missing-risc0-guest".to_string(),
            },
            bin: vec![BinSection {
                name: "missing-risc0-bin".to_string(),
            }],
        };

        let err = export_risc0_elves(&temp_root, &manifest, &temp_root.join("target")).unwrap_err();
        assert!(
            err.to_string()
                .contains("Missing ELF for missing-risc0-bin")
        );

        let _ = fs::remove_dir_all(temp_root);
    }

    #[test]
    fn guest_fingerprint_round_trip_matches() {
        let temp_root = temp_test_dir();
        let fingerprint_path = temp_root.join("fingerprint.json");
        write_guest_fingerprint(&fingerprint_path, "sp1", false, "abc123").unwrap();

        assert!(matches_existing_fingerprint(&fingerprint_path, "sp1", false, "abc123").unwrap());
        assert!(!matches_existing_fingerprint(&fingerprint_path, "sp1", false, "def456").unwrap());
        assert!(
            !matches_existing_fingerprint(&fingerprint_path, "risc0", false, "abc123").unwrap()
        );

        let _ = fs::remove_dir_all(temp_root);
    }

    #[test]
    fn sccache_compiler_wraps_only_when_enabled() {
        assert_eq!(
            sccache_compiler(true, "riscv64-unknown-elf-gcc"),
            "sccache riscv64-unknown-elf-gcc"
        );
        assert_eq!(
            sccache_compiler(false, "riscv64-unknown-elf-gcc"),
            "riscv64-unknown-elf-gcc"
        );
    }

    fn temp_test_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("raiko2-xtask-build-guest-test-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
