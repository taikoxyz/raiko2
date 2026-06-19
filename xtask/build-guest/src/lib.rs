use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use risc0_binfmt::ProgramBinary;
use risc0_zkos_v1compat::V1COMPAT_ELF;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
const SP1_RELEASE_BINARY_NAMES: &[&str] = &["sp1-shasta-proposal", "sp1-shasta-aggregation"];
const SP1_RELEASE_ELF_STEMS: &[&str] = &["sp1_shasta_proposal", "sp1_shasta_aggregation"];

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
    /// Enable the guest bench feature. SP1 lab binaries also require --all-binaries.
    #[arg(long)]
    pub bench: bool,
    /// Build every SP1 binary from guests/sp1/Cargo.toml instead of only release-required Shasta binaries.
    #[arg(long, default_value_t = false)]
    pub all_binaries: bool,
    /// Force rebuilding guests even if fingerprints match.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sp1BuildProfile {
    Release,
    Full,
}

impl Sp1BuildProfile {
    const fn from_all_binaries(all_binaries: bool) -> Self {
        if all_binaries {
            Self::Full
        } else {
            Self::Release
        }
    }

    const fn fingerprint_label(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Full => "full",
        }
    }

    fn log_label(self) -> String {
        match self {
            Self::Release => format!("release binaries ({})", SP1_RELEASE_ELF_STEMS.join(", ")),
            Self::Full => "full manifest".to_string(),
        }
    }
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask-build-guest lives under xtask/")
        .to_path_buf()
}

pub fn run(root: &Path, args: BuildGuestArgs) -> Result<()> {
    ensure_release_guest_elves_with_sp1_profile(
        root,
        args.backend,
        args.bench,
        None,
        args.force,
        Sp1BuildProfile::from_all_binaries(args.all_binaries),
    )?;
    println!("[INFO] Build complete!");
    Ok(())
}

pub fn build(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
) -> Result<()> {
    build_with_sp1_profile(
        root,
        backend,
        bench,
        sp1_docker_tag,
        Sp1BuildProfile::Release,
    )
}

pub fn build_with_sp1_profile(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
    sp1_profile: Sp1BuildProfile,
) -> Result<()> {
    match backend {
        Backend::Risc0 => {
            build_risc0(root, bench)?;
        }
        Backend::Sp1 => {
            build_sp1(root, bench, sp1_docker_tag, sp1_profile)?;
        }
        Backend::All => {
            build_risc0(root, bench)?;
            build_sp1(root, bench, sp1_docker_tag, sp1_profile)?;
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

#[derive(Debug, PartialEq, Eq)]
enum GuestBuildDecision {
    Skip,
    Rebuild(GuestRebuildReason),
}

#[derive(Debug, PartialEq, Eq)]
enum GuestRebuildReason {
    Forced,
    Changed,
}

pub fn ensure_release_guest_elves(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
    force_rebuild: bool,
) -> Result<()> {
    ensure_release_guest_elves_with_sp1_profile(
        root,
        backend,
        bench,
        sp1_docker_tag,
        force_rebuild,
        Sp1BuildProfile::Release,
    )
}

fn ensure_release_guest_elves_with_sp1_profile(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
    force_rebuild: bool,
    sp1_profile: Sp1BuildProfile,
) -> Result<()> {
    match backend {
        Backend::Risc0 => ensure_release_backend(
            root,
            Backend::Risc0,
            bench,
            sp1_docker_tag,
            force_rebuild,
            sp1_profile,
        ),
        Backend::Sp1 => ensure_release_backend(
            root,
            Backend::Sp1,
            bench,
            sp1_docker_tag,
            force_rebuild,
            sp1_profile,
        ),
        Backend::All => {
            ensure_release_backend(
                root,
                Backend::Risc0,
                bench,
                sp1_docker_tag,
                force_rebuild,
                sp1_profile,
            )?;
            ensure_release_backend(
                root,
                Backend::Sp1,
                bench,
                sp1_docker_tag,
                force_rebuild,
                sp1_profile,
            )
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
    force_rebuild: bool,
    sp1_profile: Sp1BuildProfile,
) -> Result<()> {
    let backend_key = match backend {
        Backend::Risc0 => "risc0",
        Backend::Sp1 => "sp1",
        Backend::All => unreachable!("release backend cache is evaluated per concrete backend"),
    };
    if matches!(backend, Backend::Sp1) {
        println!(
            "[INFO] SP1 guest build profile: {}",
            sp1_profile.log_label()
        );
    }
    let outputs_exist = guest_outputs_exist(root, backend, sp1_profile)?;
    let fingerprint = compute_guest_fingerprint(
        root,
        backend,
        bench,
        sp1_docker_tag,
        sp1_profile,
        outputs_exist,
    )?;
    let fingerprint_path = guest_fingerprint_path(root, backend_key);

    match guest_build_decision(
        &fingerprint_path,
        backend_key,
        bench,
        &fingerprint,
        outputs_exist,
        force_rebuild,
    )? {
        GuestBuildDecision::Skip => {
            println!(
                "[INFO] Guest ELFs for backend `{backend_key}` match the fingerprint cache; skipping rebuild."
            );
            return Ok(());
        }
        GuestBuildDecision::Rebuild(GuestRebuildReason::Forced) => {
            println!(
                "[INFO] Rebuilding guest ELFs for backend `{backend_key}` because rebuild was forced."
            );
        }
        GuestBuildDecision::Rebuild(GuestRebuildReason::Changed) => {
            println!(
                "[INFO] Rebuilding guest ELFs for backend `{backend_key}` because outputs are missing or fingerprints changed..."
            );
        }
    }

    build_with_sp1_profile(root, backend, bench, sp1_docker_tag, sp1_profile)?;
    let fingerprint =
        compute_guest_fingerprint(root, backend, bench, sp1_docker_tag, sp1_profile, true)?;
    write_guest_fingerprint(&fingerprint_path, backend_key, bench, &fingerprint)?;
    Ok(())
}

fn default_fingerprint_dir(root: &Path) -> PathBuf {
    util::target_root(root).join("xtask/guest-fingerprints")
}

fn guest_fingerprint_path(root: &Path, backend_key: &str) -> PathBuf {
    default_fingerprint_dir(root).join(format!("{backend_key}.json"))
}

fn guest_build_decision(
    fingerprint_path: &Path,
    backend_key: &str,
    bench: bool,
    fingerprint: &str,
    outputs_exist: bool,
    force_rebuild: bool,
) -> Result<GuestBuildDecision> {
    if force_rebuild {
        return Ok(GuestBuildDecision::Rebuild(GuestRebuildReason::Forced));
    }
    if outputs_exist
        && matches_existing_fingerprint(fingerprint_path, backend_key, bench, fingerprint)?
    {
        return Ok(GuestBuildDecision::Skip);
    }
    Ok(GuestBuildDecision::Rebuild(GuestRebuildReason::Changed))
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

fn guest_outputs_exist(
    root: &Path,
    backend: Backend,
    sp1_profile: Sp1BuildProfile,
) -> Result<bool> {
    for output in expected_guest_outputs(root, backend, sp1_profile)? {
        if !output.exists() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn expected_guest_outputs(
    root: &Path,
    backend: Backend,
    sp1_profile: Sp1BuildProfile,
) -> Result<Vec<PathBuf>> {
    let mut outputs = Vec::new();
    match backend {
        Backend::Risc0 => outputs.extend(expected_backend_outputs(root, "risc0", sp1_profile)?),
        Backend::Sp1 => outputs.extend(expected_backend_outputs(root, "sp1", sp1_profile)?),
        Backend::All => {
            outputs.extend(expected_backend_outputs(root, "risc0", sp1_profile)?);
            outputs.extend(expected_backend_outputs(root, "sp1", sp1_profile)?);
        }
    }
    Ok(outputs)
}

fn expected_backend_outputs(
    root: &Path,
    backend_key: &str,
    sp1_profile: Sp1BuildProfile,
) -> Result<Vec<PathBuf>> {
    let manifest = read_manifest(&root.join(format!("guests/{backend_key}/Cargo.toml")))?;
    expected_backend_outputs_for_manifest(root, backend_key, &manifest, sp1_profile)
}

fn expected_backend_outputs_for_manifest(
    root: &Path,
    backend_key: &str,
    manifest: &CargoManifest,
    sp1_profile: Sp1BuildProfile,
) -> Result<Vec<PathBuf>> {
    Ok(select_backend_bins(manifest, backend_key, sp1_profile)?
        .into_iter()
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
    sp1_profile: Sp1BuildProfile,
    include_outputs: bool,
) -> Result<String> {
    let mut hasher = Sha256::new();
    hash_tagged_bytes(&mut hasher, "bench", if bench { b"1" } else { b"0" });

    match backend {
        Backend::Risc0 => compute_backend_fingerprint(
            root,
            &mut hasher,
            "risc0",
            None,
            sp1_profile,
            include_outputs,
        )?,
        Backend::Sp1 => compute_backend_fingerprint(
            root,
            &mut hasher,
            "sp1",
            Some(resolve_sp1_docker_tag(root, sp1_docker_tag)),
            sp1_profile,
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
    sp1_profile: Sp1BuildProfile,
    include_outputs: bool,
) -> Result<()> {
    hash_tagged_bytes(hasher, "backend", backend_key.as_bytes());
    if let Some(tag) = sp1_tag {
        hash_tagged_bytes(hasher, "sp1_docker_tag", tag.as_bytes());
    }
    hash_backend_env(hasher, backend_key);

    let manifest_path = root.join(format!("guests/{backend_key}/Cargo.toml"));
    let manifest = read_manifest(&manifest_path)?;
    if backend_key == "sp1" {
        hash_tagged_bytes(
            hasher,
            "sp1_build_profile",
            sp1_profile.fingerprint_label().as_bytes(),
        );
        for bin in select_sp1_bins(&manifest, sp1_profile)? {
            hash_tagged_bytes(hasher, "sp1_binary", bin.name.as_bytes());
        }
    }

    let mut paths = vec![
        root.join("rust-toolchain.toml"),
        root.join("xtask/build-guest/Cargo.toml"),
        root.join("xtask/build-guest/src/lib.rs"),
        root.join("xtask/build-guest/src/main.rs"),
        root.join("xtask/build-guest/src/util.rs"),
        root.join("crates/guest-common/Cargo.toml"),
        manifest_path,
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
        for output in
            expected_backend_outputs_for_manifest(root, backend_key, &manifest, sp1_profile)?
        {
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

    export_risc0_elves(root, &manifest, &target_root)?;
    println!("[INFO] RISC0 guest build complete");
    Ok(())
}

fn build_risc0_with_toolchain_image(root: &Path, bench: bool, image: &str) -> Result<()> {
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
        .arg(format!("CC_riscv32im_risc0_zkvm_elf={DEFAULT_RISC0_CC}"))
        .arg("-e")
        .arg(format!("CXX_riscv32im_risc0_zkvm_elf={DEFAULT_RISC0_CXX}"))
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

    cmd.arg(image)
        .arg("cargo")
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

    println!("[INFO] Building RISC0 guest package (toolchain image)...");
    util::run(cmd)?;
    util::restore_docker_ownership(
        image,
        root,
        extra_mount.as_deref(),
        &[target_root.as_path()],
    )?;

    export_risc0_elves(root, &manifest, &target_root)?;
    println!("[INFO] RISC0 guest build complete");
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

fn build_sp1(
    root: &Path,
    bench: bool,
    sp1_docker_tag: Option<&str>,
    sp1_profile: Sp1BuildProfile,
) -> Result<()> {
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
        return build_sp1_with_toolchain_image(root, bench, toolchain_image, sp1_profile);
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
    let bins = select_sp1_bins(&manifest, sp1_profile)?;
    println!("[INFO] SP1 build profile: {}", sp1_profile.log_label());

    println!("[INFO] Building SP1 guest binaries in a single cargo prove invocation...");

    let mut cmd = Command::new("cargo");
    cmd.current_dir(root.join("guests/sp1"));
    cmd.arg("prove")
        .arg("build")
        .arg("--docker")
        .arg("--tag")
        .arg(&sp1_tag)
        .arg("--ignore-rust-version");
    if bench {
        cmd.arg("--features").arg("bench");
    }
    for bin in &bins {
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
    export_sp1_elves(&bins, &export_dir, &output_dir)?;

    println!("[INFO] SP1 guest build complete");
    Ok(())
}

fn build_sp1_with_toolchain_image(
    root: &Path,
    bench: bool,
    image: &str,
    sp1_profile: Sp1BuildProfile,
) -> Result<()> {
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
    let bins = select_sp1_bins(&manifest, sp1_profile)?;
    println!("[INFO] SP1 build profile: {}", sp1_profile.log_label());

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

    cmd.arg("-e")
        .arg(format!("{DEFAULT_SP1_CC_ENV}={DEFAULT_SP1_CC}"));
    cmd.arg("-e")
        .arg(format!("{DEFAULT_SP1_CXX_ENV}={DEFAULT_SP1_CXX}"));
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

    let mut script = String::from("set -eu\ncargo prove build --ignore-rust-version ");
    if bench {
        script.push_str("--features bench ");
    }
    for bin in &bins {
        script.push_str("--binaries ");
        script.push_str(&bin.name);
        script.push(' ');
    }
    script.push_str("--output-directory ");
    script.push_str(container_export_dir.to_string_lossy().as_ref());
    script.push_str(" --locked --workspace-directory /work\n");

    cmd.arg(image).arg("sh").arg("-lc").arg(script);

    println!("[INFO] Building SP1 guest package (toolchain image)...");
    util::run(cmd)?;
    util::restore_docker_ownership(
        image,
        root,
        extra_mount.as_deref(),
        &[target_root.as_path()],
    )?;
    export_sp1_elves(&bins, &export_dir, &output_dir)?;

    println!("[INFO] SP1 guest build complete");
    Ok(())
}

fn export_sp1_elves(bins: &[&BinSection], export_dir: &Path, output_dir: &Path) -> Result<()> {
    for bin in bins {
        let source = export_dir.join(&bin.name);
        let destination = output_dir.join(format!("{}.elf", bin.name.replace('-', "_")));
        fs::copy(&source, &destination)
            .with_context(|| format!("export {source:?} -> {destination:?}"))?;
        println!(
            "[INFO] Exported {}",
            destination.file_name().unwrap().to_string_lossy()
        );
    }

    Ok(())
}

fn select_backend_bins<'a>(
    manifest: &'a CargoManifest,
    backend_key: &str,
    sp1_profile: Sp1BuildProfile,
) -> Result<Vec<&'a BinSection>> {
    match backend_key {
        "sp1" => select_sp1_bins(manifest, sp1_profile),
        "risc0" => Ok(manifest.bin.iter().collect()),
        _ => unreachable!("unknown guest backend {backend_key}"),
    }
}

fn select_sp1_bins(manifest: &CargoManifest, profile: Sp1BuildProfile) -> Result<Vec<&BinSection>> {
    match profile {
        Sp1BuildProfile::Full => Ok(manifest.bin.iter().collect()),
        Sp1BuildProfile::Release => {
            let mut bins = Vec::with_capacity(SP1_RELEASE_BINARY_NAMES.len());
            let mut missing = Vec::new();
            for name in SP1_RELEASE_BINARY_NAMES {
                if let Some(bin) = manifest.bin.iter().find(|bin| bin.name == *name) {
                    bins.push(bin);
                } else {
                    missing.push(*name);
                }
            }
            if !missing.is_empty() {
                bail!(
                    "SP1 release binary target(s) missing from guests/sp1/Cargo.toml: {}",
                    missing.join(", ")
                );
            }
            Ok(bins)
        }
    }
}

fn ensure_local_sp1_toolchain_image(root: &Path, image: &str, sp1_tag: &str) -> Result<()> {
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
    util::run(build)
}

fn ensure_local_risc0_toolchain_image(root: &Path, image: &str) -> Result<()> {
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
    util::run(build)
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
    use clap::Parser;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        args: BuildGuestArgs,
    }

    #[test]
    fn build_guest_args_parse_force_flag() {
        let cli = TestCli::parse_from(["test", "sp1", "--force"]);

        assert_eq!(cli.args.backend, Backend::Sp1);
        assert!(cli.args.force);
    }

    #[test]
    fn build_guest_args_default_to_non_force() {
        let cli = TestCli::parse_from(["test", "risc0"]);

        assert_eq!(cli.args.backend, Backend::Risc0);
        assert!(!cli.args.force);
    }

    #[test]
    fn build_guest_args_parse_all_binaries_flag() {
        let cli = TestCli::parse_from(["test", "sp1", "--all-binaries"]);

        assert_eq!(cli.args.backend, Backend::Sp1);
        assert!(cli.args.all_binaries);
    }

    #[test]
    fn release_sp1_selection_includes_only_shasta_release_bins() {
        let manifest = test_sp1_manifest();
        let bins = select_sp1_bins(&manifest, Sp1BuildProfile::Release)
            .unwrap()
            .into_iter()
            .map(|bin| bin.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(bins, vec!["sp1-shasta-proposal", "sp1-shasta-aggregation"]);
    }

    #[test]
    fn full_sp1_selection_includes_lab_bins() {
        let manifest = test_sp1_manifest();
        let bins = select_sp1_bins(&manifest, Sp1BuildProfile::Full)
            .unwrap()
            .into_iter()
            .map(|bin| bin.name.as_str())
            .collect::<Vec<_>>();

        assert!(bins.contains(&"sp1-opcode-lab"));
        assert!(bins.contains(&"sp1-precompile-lab"));
        assert!(bins.contains(&"sp1-revm-opcode-lab"));
    }

    #[test]
    fn default_fingerprint_dir_uses_xtask_target_cache() {
        let temp_root = temp_test_dir();

        assert_eq!(
            default_fingerprint_dir(&temp_root),
            util::target_root(&temp_root).join("xtask/guest-fingerprints")
        );

        let _ = fs::remove_dir_all(temp_root);
    }

    #[test]
    fn guest_build_decision_skips_matching_fingerprints_unless_forced() {
        let temp_root = temp_test_dir();
        let fingerprint_path = temp_root.join("fingerprint.json");
        write_guest_fingerprint(&fingerprint_path, "sp1", false, "abc123").unwrap();

        assert_eq!(
            guest_build_decision(&fingerprint_path, "sp1", false, "abc123", true, false).unwrap(),
            GuestBuildDecision::Skip
        );
        assert_eq!(
            guest_build_decision(&fingerprint_path, "sp1", false, "abc123", true, true).unwrap(),
            GuestBuildDecision::Rebuild(GuestRebuildReason::Forced)
        );
        assert_eq!(
            guest_build_decision(&fingerprint_path, "sp1", false, "abc123", false, false).unwrap(),
            GuestBuildDecision::Rebuild(GuestRebuildReason::Changed)
        );

        let _ = fs::remove_dir_all(temp_root);
    }

    #[test]
    fn sp1_guest_fingerprint_changes_with_docker_tag() {
        let root = repo_root();
        let v1 = compute_guest_fingerprint(
            &root,
            Backend::Sp1,
            false,
            Some("v5.2.4"),
            Sp1BuildProfile::Release,
            false,
        )
        .unwrap();
        let v2 = compute_guest_fingerprint(
            &root,
            Backend::Sp1,
            false,
            Some("v5.2.5"),
            Sp1BuildProfile::Release,
            false,
        )
        .unwrap();
        assert_ne!(v1, v2);
    }

    #[test]
    fn sp1_guest_fingerprint_changes_with_build_profile() {
        let root = repo_root();
        let release = compute_guest_fingerprint(
            &root,
            Backend::Sp1,
            false,
            Some("v5.2.4"),
            Sp1BuildProfile::Release,
            false,
        )
        .unwrap();
        let full = compute_guest_fingerprint(
            &root,
            Backend::Sp1,
            false,
            Some("v5.2.4"),
            Sp1BuildProfile::Full,
            false,
        )
        .unwrap();

        assert_ne!(release, full);
    }

    #[test]
    fn release_sp1_outputs_expect_only_shasta_elves() {
        let temp_root = temp_test_dir();
        write_sp1_manifest(&temp_root, &test_sp1_manifest());

        let outputs = expected_guest_outputs(&temp_root, Backend::Sp1, Sp1BuildProfile::Release)
            .unwrap()
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            outputs,
            vec!["sp1_shasta_proposal.elf", "sp1_shasta_aggregation.elf"]
        );

        let _ = fs::remove_dir_all(temp_root);
    }

    #[test]
    fn full_sp1_outputs_expect_lab_elves() {
        let temp_root = temp_test_dir();
        write_sp1_manifest(&temp_root, &test_sp1_manifest());

        let outputs = expected_guest_outputs(&temp_root, Backend::Sp1, Sp1BuildProfile::Full)
            .unwrap()
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(outputs.contains(&"sp1_opcode_lab.elf".to_string()));
        assert!(outputs.contains(&"sp1_precompile_lab.elf".to_string()));
        assert!(outputs.contains(&"sp1_revm_opcode_lab.elf".to_string()));

        let _ = fs::remove_dir_all(temp_root);
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

    fn temp_test_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = TEMP_TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "raiko2-xtask-build-guest-test-{}-{unique}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn test_sp1_manifest() -> CargoManifest {
        CargoManifest {
            package: PackageSection {
                name: "raiko2-guest-sp1".to_string(),
            },
            bin: vec![
                BinSection {
                    name: "sp1-shasta-proposal".to_string(),
                },
                BinSection {
                    name: "sp1-shasta-aggregation".to_string(),
                },
                BinSection {
                    name: "sp1-opcode-lab".to_string(),
                },
                BinSection {
                    name: "sp1-precompile-lab".to_string(),
                },
                BinSection {
                    name: "sp1-revm-opcode-lab".to_string(),
                },
            ],
        }
    }

    fn write_sp1_manifest(root: &Path, manifest: &CargoManifest) {
        let manifest_dir = root.join("guests/sp1");
        fs::create_dir_all(&manifest_dir).unwrap();
        let mut contents = format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            manifest.package.name
        );
        for bin in &manifest.bin {
            contents.push_str(&format!("\n[[bin]]\nname = \"{}\"\n", bin.name));
        }
        fs::write(manifest_dir.join("Cargo.toml"), contents).unwrap();
    }
}
