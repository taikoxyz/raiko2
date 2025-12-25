use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Deserialize;

const DEFAULT_RISC0_RUSTFLAGS: &str = "-C passes=lower-atomic -C link-arg=-Ttext=0x00200800 -C link-arg=--fatal-warnings -C panic=abort --cfg getrandom_backend=\"custom\"";
const DEFAULT_SP1_RUSTFLAGS: &str = "-C passes=lower-atomic -C link-arg=-Ttext=0x00200800 -C panic=abort --cfg getrandom_backend=\"custom\"";

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build guest ELF binaries using official docker tooling.
    BuildGuest {
        #[arg(value_enum)]
        backend: Backend,
        /// Include benchmark binaries (requires bins in Cargo.toml).
        #[arg(long)]
        bench: bool,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Backend {
    Risc0,
    Sp1,
    All,
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

fn main() -> Result<()> {
    let args = Args::parse();
    let root = repo_root();

    match args.cmd {
        Cmd::BuildGuest { backend, bench } => match backend {
            Backend::Risc0 => {
                build_risc0(&root, bench)?;
                update_image_ids(&root, "risc0")?;
            }
            Backend::Sp1 => {
                build_sp1(&root, bench)?;
                update_image_ids(&root, "sp1")?;
            }
            Backend::All => {
                build_risc0(&root, bench)?;
                update_image_ids(&root, "risc0")?;
                build_sp1(&root, bench)?;
                update_image_ids(&root, "sp1")?;
            }
        },
    }

    println!("[INFO] Build complete!");
    Ok(())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in repo root")
        .to_path_buf()
}

fn ensure_command(mut cmd: Command, name: &str, hint: &str) -> Result<()> {
    let status = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status();
    match status {
        Ok(status) if status.success() => Ok(()),
        _ => Err(anyhow!("{name} is required. {hint}")),
    }
}

fn ensure_docker() -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.arg("--version");
    ensure_command(cmd, "docker", "Install Docker and re-run.")
}

fn ensure_cargo_risczero() -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("risczero").arg("--version");
    ensure_command(cmd, "cargo-risczero", "Install via: rzup install")
}

fn ensure_cargo_prove() -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("prove").arg("--version");
    ensure_command(cmd, "cargo-prove", "Install via: sp1up")
}

fn build_risc0(root: &Path, bench: bool) -> Result<()> {
    println!("[INFO] Building RISC0 guest programs...");
    ensure_docker()?;
    ensure_cargo_risczero()?;

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

    let manifest_path = root.join("guests/risc0/Cargo.toml");
    let manifest = read_manifest(&manifest_path)?;

    let mut cmd = Command::new("cargo");
    cmd.arg("risczero")
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest_path);

    let target_root = env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("target"));
    cmd.env("CARGO_TARGET_DIR", &target_root);

    let rustflags =
        env::var("RISC0_GUEST_RUSTFLAGS").unwrap_or_else(|_| DEFAULT_RISC0_RUSTFLAGS.to_string());
    cmd.env("CARGO_TARGET_RISCV32IM_RISC0_ZKVM_ELF_RUSTFLAGS", rustflags);
    cmd.env("RISC0_FEATURE_bigint2", "1");

    if let Ok(cc) = env::var("RISC0_GUEST_CC") && !cc.is_empty() {
        cmd.env("CC", cc);
    }
    if let Ok(cflags) = env::var("RISC0_GUEST_CFLAGS") && !cflags.is_empty() {
        cmd.env("CFLAGS", cflags);
    }
    if let Ok(platform) = env::var("DOCKER_DEFAULT_PLATFORM") && !platform.is_empty() {
        cmd.env("DOCKER_DEFAULT_PLATFORM", platform);
    }
    if env::var("MOCK").ok().as_deref() == Some("1") {
        cmd.env("RISC0_DEV_MODE", "1");
        println!("[INFO] RISC0_DEV_MODE enabled");
    }

    println!("[INFO] Building RISC0 guest package (docker via cargo risczero)...");
    run(cmd)?;

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

fn build_sp1(root: &Path, bench: bool) -> Result<()> {
    println!("[INFO] Building SP1 guest programs...");
    ensure_docker()?;
    ensure_cargo_prove()?;

    let profile = env::var("PROFILE").unwrap_or_else(|_| "release".to_string());
    if profile != "release" {
        println!("[WARN] PROFILE={profile} is ignored by cargo prove; building default profile");
    }
    if bench {
        println!(
            "[WARN] --bench has no effect unless extra bins are defined in guests/sp1/Cargo.toml"
        );
    }

    let manifest_path = root.join("guests/sp1/Cargo.toml");
    let manifest = read_manifest(&manifest_path)?;
    let output_dir = root.join("crates/guests/elf");
    fs::create_dir_all(&output_dir)?;

    let sp1_tag = env::var("SP1_DOCKER_TAG").unwrap_or_else(|_| "v5.2.4".to_string());

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
            .arg(&sp1_tag)
            .arg("--binaries")
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

        if let Ok(cc) = env::var("SP1_GUEST_CC") && !cc.is_empty() {
            cmd.env("CC", cc);
        }
        if let Ok(cflags) = env::var("SP1_GUEST_CFLAGS") && !cflags.is_empty() {
            cmd.env("CFLAGS", cflags);
        }
        if let Ok(platform) = env::var("DOCKER_DEFAULT_PLATFORM") && !platform.is_empty() {
            cmd.env("DOCKER_DEFAULT_PLATFORM", platform);
        }
        if env::var("MOCK").ok().as_deref() == Some("1") {
            cmd.env("SP1_PROVER", "mock");
            println!("[INFO] SP1_PROVER=mock enabled");
        }
        if env::var("VERBOSE").ok().as_deref() == Some("1") {
            println!("[WARN] VERBOSE=1 is ignored by cargo prove build");
        }

        run(cmd)?;
    }

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
        return Err(anyhow!("update_imageid.sh not found at {script:?}"));
    }

    let mut cmd = Command::new(script);
    cmd.arg(backend);
    run(cmd)?;
    Ok(())
}

fn read_manifest(path: &Path) -> Result<CargoManifest> {
    let contents = fs::read_to_string(path).with_context(|| format!("read manifest {path:?}"))?;
    let manifest: CargoManifest =
        toml::from_str(&contents).with_context(|| format!("parse manifest {path:?}"))?;
    Ok(manifest)
}

fn run(mut cmd: Command) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("failed to run {cmd:?}"))?;
    if !status.success() {
        bail!("command failed: {cmd:?}");
    }
    Ok(())
}
