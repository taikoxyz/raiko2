use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use serde::Deserialize;

use crate::Backend;
use crate::util;

const DEFAULT_RISC0_RUSTFLAGS: &str = "-C passes=lower-atomic -C link-arg=-Ttext=0x00200800 -C link-arg=--fatal-warnings -C panic=abort --cfg getrandom_backend=\"custom\"";
const DEFAULT_SP1_RUSTFLAGS: &str = "-C passes=lower-atomic -C link-arg=-Ttext=0x00200800 -C panic=abort --cfg getrandom_backend=\"custom\"";
const DEFAULT_RISC0_TOOLCHAIN_IMAGE: &str = "ghcr.io/taikoxyz/raiko2/risc0-toolchain:latest";
const DEFAULT_SP1_TOOLCHAIN_IMAGE: &str = "ghcr.io/taikoxyz/raiko2/sp1-toolchain:latest";

#[derive(Args)]
pub(crate) struct BuildGuestArgs {
    #[arg(value_enum)]
    pub(crate) backend: Backend,
    /// Include benchmark binaries (requires bins in Cargo.toml).
    #[arg(long)]
    pub(crate) bench: bool,
}

pub(crate) fn run(root: &Path, args: BuildGuestArgs) -> Result<()> {
    build(root, args.backend, args.bench, None, true)?;
    println!("[INFO] Build complete!");
    Ok(())
}

pub(crate) fn build(
    root: &Path,
    backend: Backend,
    bench: bool,
    sp1_docker_tag: Option<&str>,
    update_image_ids_flag: bool,
) -> Result<()> {
    match backend {
        Backend::Risc0 => {
            build_risc0(root, bench)?;
            if update_image_ids_flag {
                update_image_ids(root, "risc0")?;
            }
        }
        Backend::Sp1 => {
            build_sp1(root, bench, sp1_docker_tag)?;
            if update_image_ids_flag {
                update_image_ids(root, "sp1")?;
            }
        }
        Backend::All => {
            build_risc0(root, bench)?;
            if update_image_ids_flag {
                update_image_ids(root, "risc0")?;
            }
            build_sp1(root, bench, sp1_docker_tag)?;
            if update_image_ids_flag {
                update_image_ids(root, "sp1")?;
            }
        }
    }
    Ok(())
}

pub(crate) fn resolve_sp1_docker_tag(root: &Path, override_tag: Option<&str>) -> String {
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

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn non_empty_str(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn default_sp1_docker_tag(root: &Path) -> Option<String> {
    let lock_path = root.join("guests/sp1/Cargo.lock");
    let contents = fs::read_to_string(lock_path).ok()?;
    let lock: CargoLockFile = toml::from_str(&contents).ok()?;
    let version = lock
        .package
        .iter()
        .find(|pkg| pkg.name == "sp1-zkvm")
        .or_else(|| lock.package.iter().find(|pkg| pkg.name == "sp1-sdk"))?
        .version
        .clone();
    Some(format!("v{version}"))
}

fn build_risc0(root: &Path, bench: bool) -> Result<()> {
    println!("[INFO] Building RISC0 guest programs...");
    util::ensure_docker()?;
    let toolchain_image = env::var("RISC0_TOOLCHAIN_IMAGE")
        .unwrap_or_else(|_| DEFAULT_RISC0_TOOLCHAIN_IMAGE.to_string());
    let toolchain_image = toolchain_image.trim();
    if !toolchain_image.is_empty()
        && !toolchain_image.eq_ignore_ascii_case("local")
        && !toolchain_image.eq_ignore_ascii_case("none")
    {
        return build_risc0_with_toolchain_image(root, bench, toolchain_image);
    }
    util::ensure_cargo_risczero()?;

    let risc0_docker_tag =
        env::var("RISC0_DOCKER_CONTAINER_TAG").unwrap_or_else(|_| "r0.1.91.1".to_string());

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

    let target_root = util::target_root(root);
    cmd.env("CARGO_TARGET_DIR", &target_root);

    let rustflags =
        env::var("RISC0_GUEST_RUSTFLAGS").unwrap_or_else(|_| DEFAULT_RISC0_RUSTFLAGS.to_string());
    cmd.env("CARGO_TARGET_RISCV32IM_RISC0_ZKVM_ELF_RUSTFLAGS", rustflags);
    cmd.env("RISC0_FEATURE_bigint2", "1");
    cmd.env("RISC0_DOCKER_CONTAINER_TAG", &risc0_docker_tag);
    println!("[INFO] RISC0 docker tag: {risc0_docker_tag}");

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

    let risc0_docker_tag =
        env::var("RISC0_DOCKER_CONTAINER_TAG").unwrap_or_else(|_| "r0.1.91.1".to_string());
    let rustflags =
        env::var("RISC0_GUEST_RUSTFLAGS").unwrap_or_else(|_| DEFAULT_RISC0_RUSTFLAGS.to_string());

    let manifest_path = root.join("guests/risc0/Cargo.toml");
    let manifest = read_manifest(&manifest_path)?;
    let container_manifest_path = manifest_path
        .strip_prefix(root)
        .map(|rel| PathBuf::from("/work").join(rel))
        .unwrap_or_else(|_| PathBuf::from("/work/guests/risc0/Cargo.toml"));

    let target_root = util::target_root(root);
    let (container_target_dir, extra_mount) = match target_root.strip_prefix(root).ok() {
        Some(rel) => (PathBuf::from("/work").join(rel), None),
        None => (PathBuf::from("/target"), Some(target_root.clone())),
    };

    let mut cmd = Command::new("docker");
    cmd.arg("run").arg("--rm");
    cmd.arg("-v")
        .arg(format!("{}:/work", root.display()))
        .arg("-w")
        .arg("/work")
        .arg("-v")
        .arg("/var/run/docker.sock:/var/run/docker.sock");
    if let Some(docker_path) = util::find_executable("docker") {
        cmd.arg("-v")
            .arg(format!("{}:/usr/bin/docker", docker_path.display()));
    } else {
        println!(
            "[WARN] docker not found in PATH; cargo risczero may fail inside the toolchain image"
        );
    }
    let buildx_path = util::find_docker_buildx_plugin().ok_or_else(|| {
        anyhow!("docker-buildx plugin not found. Install buildx or set RISC0_TOOLCHAIN_IMAGE=none")
    })?;
    cmd.arg("-v").arg(format!(
        "{}:/root/.docker/cli-plugins/docker-buildx",
        buildx_path.display()
    ));

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
        .arg(format!("RISC0_DOCKER_CONTAINER_TAG={risc0_docker_tag}"))
        .arg("-e")
        .arg("RISC0_FEATURE_bigint2=1")
        .arg("-e")
        .arg("DOCKER_BUILDKIT=1")
        .arg("-e")
        .arg("DOCKER_CLI_PLUGIN_EXTRA_DIRS=/root/.docker/cli-plugins");

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
        .arg("risczero")
        .arg("build")
        .arg("--manifest-path")
        .arg(&container_manifest_path);

    println!("[INFO] Building RISC0 guest package (toolchain image)...");
    util::run(cmd)?;

    export_risc0_elves(root, &manifest, &target_root)?;
    println!("[INFO] RISC0 guest build complete");
    Ok(())
}

fn export_risc0_elves(root: &Path, manifest: &CargoManifest, target_root: &Path) -> Result<()> {
    let output_dir = root.join("crates/guests/elf");
    fs::create_dir_all(&output_dir)?;

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
                fs::copy(candidate, output_dir.join(&elf_name))
                    .with_context(|| format!("copy {candidate:?} -> {elf_name}"))?;
                println!("[INFO] Exported {elf_name}");
                copied = true;
                break;
            }
        }

        if !copied {
            println!(
                "[WARN] Missing ELF for {} (checked {:?} and {:?})",
                bin.name, target_dir, legacy_dir
            );
        }
    }

    Ok(())
}

fn build_sp1(root: &Path, bench: bool, sp1_docker_tag: Option<&str>) -> Result<()> {
    println!("[INFO] Building SP1 guest programs...");
    util::ensure_docker()?;

    let toolchain_image =
        env::var("SP1_TOOLCHAIN_IMAGE").unwrap_or_else(|_| DEFAULT_SP1_TOOLCHAIN_IMAGE.to_string());
    let toolchain_image = toolchain_image.trim();
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
    let output_dir = root.join("crates/guests/elf");
    fs::create_dir_all(&output_dir)?;

    let sp1_tag = resolve_sp1_docker_tag(root, sp1_docker_tag);
    println!("[INFO] SP1 docker tag: {sp1_tag}");

    if manifest.bin.is_empty() {
        bail!("No [[bin]] targets found in guests/sp1/Cargo.toml");
    }

    for bin in &manifest.bin {
        println!("[INFO] Building {} (docker via cargo prove)...", bin.name);
        let elf_name = format!("{}.elf", bin.name.replace('-', "_"));

        let mut cmd = Command::new("cargo");
        cmd.current_dir(root.join("guests/sp1"));
        cmd.arg("prove")
            .arg("build")
            .arg("--docker")
            .arg("--tag")
            .arg(&sp1_tag);
        if bench {
            cmd.arg("--features").arg("bench");
        }
        cmd.arg("--binaries")
            .arg(&bin.name)
            .arg("--elf-name")
            .arg(&elf_name)
            .arg("--output-directory")
            .arg(&output_dir)
            .arg("--locked")
            .arg("--workspace-directory")
            .arg(root);

        let rustflags =
            env::var("SP1_GUEST_RUSTFLAGS").unwrap_or_else(|_| DEFAULT_SP1_RUSTFLAGS.to_string());
        cmd.env(
            "CARGO_TARGET_RISCV32IM_SUCCINCT_ZKVM_ELF_RUSTFLAGS",
            rustflags,
        );

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
    }

    println!("[INFO] SP1 guest build complete");
    Ok(())
}

fn build_sp1_with_toolchain_image(root: &Path, bench: bool, image: &str) -> Result<()> {
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

    let output_dir = root.join("crates/guests/elf");
    fs::create_dir_all(&output_dir)?;

    let target_root = util::target_root(root);
    let (container_target_dir, extra_mount) = match target_root.strip_prefix(root).ok() {
        Some(rel) => (PathBuf::from("/work").join(rel), None),
        None => (PathBuf::from("/target"), Some(target_root.clone())),
    };

    let container_output_dir = output_dir
        .strip_prefix(root)
        .map(|rel| PathBuf::from("/work").join(rel))
        .unwrap_or_else(|_| PathBuf::from("/work/crates/guests/elf"));

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
        .arg(format!(
            "CARGO_TARGET_RISCV32IM_SUCCINCT_ZKVM_ELF_RUSTFLAGS={rustflags}"
        ));

    // Ensure crates with C/C++ sources (e.g. `c-kzg`, `blst`) cross-compile for the guest target.
    // Prefer the RISC-V bare-metal GCC toolchain: it provides a proper sysroot with standard headers.
    cmd.arg("-e")
        .arg("CC_riscv32im_succinct_zkvm_elf=riscv64-unknown-elf-gcc -specs=picolibc.specs");
    cmd.arg("-e")
        .arg("CXX_riscv32im_succinct_zkvm_elf=riscv64-unknown-elf-g++ -specs=picolibcpp.specs");
    cmd.arg("-e")
        .arg("AR_riscv32im_succinct_zkvm_elf=riscv64-unknown-elf-ar");

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

    let mut script = String::from("set -eu\n");
    for bin in &manifest.bin {
        let elf_name = format!("{}.elf", bin.name.replace('-', "_"));
        script.push_str("cargo prove build ");
        if bench {
            script.push_str("--features bench ");
        }
        script.push_str("--binaries ");
        script.push_str(&bin.name);
        script.push_str(" --elf-name ");
        script.push_str(&elf_name);
        script.push_str(" --output-directory ");
        script.push_str(container_output_dir.to_string_lossy().as_ref());
        script.push_str(" --locked --workspace-directory /work\n");
    }

    cmd.arg(image).arg("sh").arg("-lc").arg(script);

    println!("[INFO] Building SP1 guest package (toolchain image)...");
    util::run(cmd)?;

    println!("[INFO] SP1 guest build complete");
    Ok(())
}

fn update_image_ids(root: &Path, backend: &str) -> Result<()> {
    println!("[INFO] Updating image IDs for {backend}...");
    if !root.join(".env").exists() {
        println!("[WARN] No .env file found, skipping image ID update");
        return Ok(());
    }

    let script = root.join("script/update_imageid.sh");
    if !script.exists() {
        println!("[WARN] update_imageid.sh not found at {script:?}, skipping image ID update");
        return Ok(());
    }

    let mut cmd = Command::new(script);
    cmd.arg(backend);
    util::run(cmd)?;
    Ok(())
}

fn read_manifest(path: &Path) -> Result<CargoManifest> {
    let contents = fs::read_to_string(path).with_context(|| format!("read manifest {path:?}"))?;
    let manifest: CargoManifest =
        toml::from_str(&contents).with_context(|| format!("parse manifest {path:?}"))?;
    Ok(manifest)
}
