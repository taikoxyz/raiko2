use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};

pub(crate) const DOCKER_CARGO_HOME: &str = "/cargo";

pub(crate) fn target_root(root: &Path) -> PathBuf {
    env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("target"))
}

pub(crate) fn docker_cargo_cache_volume(root: &Path, backend: &str) -> Result<Option<String>> {
    let mode = env::var("DOCKER_CARGO_CACHE").ok().and_then(non_empty);
    let mode = mode
        .as_deref()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "volume".to_string());
    match mode.as_str() {
        "volume" | "1" | "true" => {}
        "none" | "0" | "false" | "off" => return Ok(None),
        _ => {
            bail!("unsupported DOCKER_CARGO_CACHE={mode} (expected: volume|none|true|false|0|1)");
        }
    }

    let explicit = env::var("DOCKER_CARGO_CACHE_VOLUME")
        .ok()
        .and_then(non_empty);
    if let Some(value) = explicit.as_deref()
        && !is_valid_docker_volume_name(value)
    {
        bail!(
            "invalid DOCKER_CARGO_CACHE_VOLUME={value} (expected [A-Za-z0-9_.-], must start with alnum)"
        );
    }
    let volume = explicit.unwrap_or_else(|| {
        let repo = sanitize_docker_name(&repo_name(root));
        let backend = sanitize_docker_name(backend);
        format!("{repo}-cargo-{backend}")
    });
    Ok(Some(volume))
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

pub(crate) fn ensure_cargo_risczero() -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("risczero").arg("--version");
    ensure_command(cmd, "cargo-risczero", "Install via: rzup install")
}

pub(crate) fn ensure_cargo_prove() -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("prove").arg("--version");
    ensure_command(cmd, "cargo-prove", "Install via: sp1up")
}

pub(crate) fn restore_docker_ownership(
    image: &str,
    root: &Path,
    target_mount: Option<&Path>,
    paths: &[&Path],
) -> Result<()> {
    let Some((uid, gid)) = docker_user_ids()? else {
        return Ok(());
    };
    if paths.is_empty() {
        return Ok(());
    }

    let mut cmd = Command::new("docker");
    cmd.arg("run")
        .arg("--rm")
        .arg("--entrypoint")
        .arg("chown")
        .arg("-v")
        .arg(format!("{}:/work", root.display()));

    if let Some(target_mount) = target_mount {
        cmd.arg("-v")
            .arg(format!("{}:/target", target_mount.display()));
    }

    cmd.arg(image).arg("-R").arg(format!("{uid}:{gid}"));
    for path in paths {
        cmd.arg(container_path(root, target_mount, path)?);
    }

    run(cmd)
}

pub(crate) fn docker_user_ids() -> Result<Option<(String, String)>> {
    #[cfg(unix)]
    {
        let uid = current_id_arg("-u")?;
        let gid = current_id_arg("-g")?;
        Ok(Some((uid, gid)))
    }

    #[cfg(not(unix))]
    {
        Ok(None)
    }
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn current_id_arg(flag: &str) -> Result<String> {
    let output = Command::new("id")
        .arg(flag)
        .output()
        .with_context(|| format!("failed to run id {flag}"))?;
    if !output.status.success() {
        bail!("id {flag} failed with status {}", output.status);
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .with_context(|| format!("id {flag} returned non-utf8 output"))
}

fn container_path(root: &Path, target_mount: Option<&Path>, path: &Path) -> Result<String> {
    if let Ok(rel) = path.strip_prefix(root) {
        return Ok(PathBuf::from("/work").join(rel).display().to_string());
    }
    if let Some(target_mount) = target_mount
        && let Ok(rel) = path.strip_prefix(target_mount)
    {
        return Ok(PathBuf::from("/target").join(rel).display().to_string());
    }
    bail!(
        "cannot restore ownership for path outside mounted roots: {}",
        path.display()
    );
}

fn repo_name(root: &Path) -> String {
    root.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .unwrap_or_else(|| "repo".to_string())
}

fn sanitize_docker_name(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_dash = false;
    for c in value.chars() {
        let c = c.to_ascii_lowercase();
        let normalized = match c {
            'a'..='z' | '0'..='9' | '_' | '.' => c,
            '-' => '-',
            _ => '-',
        };

        if normalized == '-' {
            if out.is_empty() || last_was_dash {
                continue;
            }
            out.push('-');
            last_was_dash = true;
        } else {
            out.push(normalized);
            last_was_dash = false;
        }
    }

    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "cache".to_string()
    } else {
        out
    }
}

fn is_valid_docker_volume_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    std::iter::once(first)
        .chain(chars)
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn container_path_maps_worktree_relative_paths() {
        assert_eq!(
            super::container_path(
                Path::new("/repo"),
                None,
                Path::new("/repo/target/elf-compilation/.rustc_info.json")
            )
            .unwrap(),
            "/work/target/elf-compilation/.rustc_info.json"
        );
    }

    #[test]
    fn container_path_maps_explicit_target_mount_paths() {
        assert_eq!(
            super::container_path(
                Path::new("/repo"),
                Some(Path::new("/tmp/target")),
                Path::new("/tmp/target/risc0/.rustc_info.json")
            )
            .unwrap(),
            "/target/risc0/.rustc_info.json"
        );
    }
}
