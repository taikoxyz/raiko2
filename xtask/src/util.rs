use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};

pub(crate) fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in repo root")
        .to_path_buf()
}

pub(crate) fn target_root(root: &Path) -> PathBuf {
    env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("target"))
}

pub(crate) fn ensure_command(mut cmd: Command, name: &str, hint: &str) -> Result<()> {
    let status = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status();
    match status {
        Ok(status) if status.success() => Ok(()),
        _ => Err(anyhow!("{name} is required. {hint}")),
    }
}

pub(crate) fn ensure_docker() -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.arg("--version");
    ensure_command(cmd, "docker", "Install Docker and re-run.")
}

pub(crate) fn ensure_docker_buildx() -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.arg("buildx").arg("version");
    ensure_command(
        cmd,
        "docker buildx",
        "Install a Docker distribution with buildx support and re-run.",
    )
}

pub(crate) fn ensure_docker_buildx_builder(name: &str) -> Result<()> {
    let inspect_status = Command::new("docker")
        .arg("buildx")
        .arg("inspect")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to inspect docker buildx builder {name}"))?;

    if !inspect_status.success() {
        let mut create_cmd = Command::new("docker");
        create_cmd
            .arg("buildx")
            .arg("create")
            .arg("--name")
            .arg(name)
            .arg("--driver")
            .arg("docker-container");
        run(create_cmd)?;
    }

    let mut bootstrap_cmd = Command::new("docker");
    bootstrap_cmd
        .arg("buildx")
        .arg("inspect")
        .arg("--bootstrap")
        .arg(name);
    run(bootstrap_cmd)
}

pub(crate) fn run(mut cmd: Command) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("failed to run {cmd:?}"))?;
    if !status.success() {
        bail!("command failed: {cmd:?}");
    }
    Ok(())
}
