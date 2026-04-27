use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;

use crate::{Backend, build_guest, util};

const DEFAULT_IMAGE_REPOSITORY: &str = "us-docker.pkg.dev/evmchain/images/raiko2";
const DEFAULT_BUILDX_BUILDER: &str = "raiko2-local-cache";

#[derive(Args, Debug)]
pub(crate) struct ReleaseImageArgs {
    #[arg(value_enum)]
    pub(crate) backend: Backend,

    #[arg(long)]
    pub(crate) tag: String,

    #[arg(long, default_value = DEFAULT_IMAGE_REPOSITORY)]
    pub(crate) repository: String,

    #[arg(long, default_value_t = false)]
    pub(crate) force_rebuild_guests: bool,
}

pub(crate) fn run(root: &std::path::Path, args: ReleaseImageArgs) -> Result<()> {
    util::ensure_docker()?;
    util::ensure_docker_buildx()?;
    util::ensure_docker_buildx_builder(DEFAULT_BUILDX_BUILDER)?;
    ensure_non_empty("tag", &args.tag)?;
    ensure_non_empty("repository", &args.repository)?;

    let image_ref = format!("{}:{}", args.repository, args.tag);
    let buildx_cache_root = util::target_root(root).join("buildx-cache").join("raiko2");
    let buildx_cache_current = buildx_cache_root.join("current");
    let buildx_cache_next = buildx_cache_root.join("next");
    fs::create_dir_all(&buildx_cache_root)
        .with_context(|| format!("failed to create buildx cache dir {buildx_cache_root:?}"))?;
    reset_dir(&buildx_cache_next)?;

    println!(
        "[INFO] Preparing guest ELFs for backend `{}` before image release...",
        backend_name(args.backend)
    );
    if args.force_rebuild_guests {
        println!("[INFO] Guest rebuild forced by --force-rebuild-guests");
        build_guest::build(root, args.backend, false, None)?;
    } else {
        build_guest::ensure_release_guest_elves(root, args.backend, false, None)?;
    }

    println!(
        "[INFO] Building runtime image `{image_ref}` with buildx local cache at {:?}...",
        buildx_cache_current
    );
    let mut build_cmd = Command::new("docker");
    build_cmd
        .arg("buildx")
        .arg("build")
        .arg("--builder")
        .arg(DEFAULT_BUILDX_BUILDER)
        .arg("--load");
    if buildx_cache_current.join("index.json").exists() {
        build_cmd
            .arg("--cache-from")
            .arg(format!("type=local,src={}", buildx_cache_current.display()));
    }
    build_cmd
        .arg("--cache-to")
        .arg(format!(
            "type=local,dest={},mode=max",
            buildx_cache_next.display()
        ))
        .arg("-t")
        .arg(&image_ref)
        .arg(root);
    util::run(build_cmd)?;
    promote_local_cache(&buildx_cache_current, &buildx_cache_next)?;

    println!("[INFO] Pushing runtime image `{image_ref}`...");
    let mut push_cmd = Command::new("docker");
    push_cmd.arg("push").arg(&image_ref);
    util::run(push_cmd)?;

    let digest_ref = resolve_repo_digest(&image_ref, &args.repository)?;
    for line in release_summary_lines(&digest_ref) {
        println!("{line}");
    }

    Ok(())
}

const fn backend_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Risc0 => "risc0",
        Backend::Sp1 => "sp1",
        Backend::All => "all",
    }
}

fn ensure_non_empty(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(())
}

fn release_summary_lines(digest_ref: &str) -> Vec<String> {
    vec![format!("[INFO] Image pushed: {digest_ref}")]
}

fn resolve_repo_digest(image_ref: &str, repository: &str) -> Result<String> {
    let output = Command::new("docker")
        .arg("image")
        .arg("inspect")
        .arg("--format")
        .arg("{{json .RepoDigests}}")
        .arg(image_ref)
        .output()
        .with_context(|| format!("failed to inspect pushed image {image_ref}"))?;
    if !output.status.success() {
        bail!("docker image inspect failed for {image_ref}");
    }

    let repo_digests: Vec<String> = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("failed to parse RepoDigests for {image_ref}"))?;
    repo_digests
        .into_iter()
        .find(|value| value.starts_with(&format!("{repository}@sha256:")))
        .ok_or_else(|| anyhow!("missing pushed digest for repository {repository}"))
}

fn reset_dir(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {path:?}"))?;
    }
    fs::create_dir_all(path).with_context(|| format!("failed to create {path:?}"))
}

fn promote_local_cache(current: &Path, next: &Path) -> Result<()> {
    if !next.join("index.json").exists() {
        bail!("buildx cache export missing index.json at {next:?}");
    }

    let backup = current.with_extension("old");
    if backup.exists() {
        fs::remove_dir_all(&backup).with_context(|| format!("failed to remove {backup:?}"))?;
    }
    if current.exists() {
        fs::rename(current, &backup)
            .with_context(|| format!("failed to move {current:?} to {backup:?}"))?;
    }
    fs::rename(next, current)
        .with_context(|| format!("failed to promote {next:?} to {current:?}"))?;
    if backup.exists() {
        fs::remove_dir_all(&backup).with_context(|| format!("failed to remove {backup:?}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::release_summary_lines;

    #[test]
    fn release_summary_lines_do_not_reference_rollout() {
        let digest_ref =
            "us-docker.pkg.dev/evmchain/images/raiko2@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let lines = release_summary_lines(digest_ref);
        let output = lines.join("\n");

        assert_eq!(lines, vec![format!("[INFO] Image pushed: {digest_ref}")]);
        assert!(output.contains("Image pushed:"));
        assert!(!output.contains("kubectl"));
        assert!(!output.contains("rollout"));
        assert!(!output.contains("deployment/"));
    }
}
