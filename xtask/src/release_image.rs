use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;

use crate::{Backend, build_guest, util};

const DEFAULT_IMAGE_REPOSITORY: &str = "us-docker.pkg.dev/evmchain/images/raiko2";
const DEFAULT_NAMESPACE: &str = "tolba-raiko2-host";
const DEFAULT_DEPLOYMENT: &str = "raiko2";
const DEFAULT_CONTAINER: &str = "raiko2";

#[derive(Args, Debug)]
pub(crate) struct ReleaseImageArgs {
    #[arg(value_enum)]
    pub(crate) backend: Backend,

    #[arg(long)]
    pub(crate) tag: String,

    #[arg(long, default_value = DEFAULT_IMAGE_REPOSITORY)]
    pub(crate) repository: String,

    #[arg(long, default_value = DEFAULT_NAMESPACE)]
    pub(crate) namespace: String,

    #[arg(long, default_value = DEFAULT_DEPLOYMENT)]
    pub(crate) deployment: String,

    #[arg(long, default_value = DEFAULT_CONTAINER)]
    pub(crate) container: String,
}

pub(crate) fn run(root: &std::path::Path, args: ReleaseImageArgs) -> Result<()> {
    util::ensure_docker()?;
    ensure_non_empty("tag", &args.tag)?;
    ensure_non_empty("repository", &args.repository)?;
    ensure_non_empty("namespace", &args.namespace)?;
    ensure_non_empty("deployment", &args.deployment)?;
    ensure_non_empty("container", &args.container)?;

    let image_ref = format!("{}:{}", args.repository, args.tag);

    println!(
        "[INFO] Rebuilding guest ELFs for backend `{}` before image release...",
        backend_name(args.backend)
    );
    build_guest::build(root, args.backend, false, None, true)?;

    println!("[INFO] Building runtime image `{image_ref}`...");
    let mut build_cmd = Command::new("docker");
    build_cmd.arg("build").arg("-t").arg(&image_ref).arg(root);
    util::run(build_cmd)?;

    println!("[INFO] Pushing runtime image `{image_ref}`...");
    let mut push_cmd = Command::new("docker");
    push_cmd.arg("push").arg(&image_ref);
    util::run(push_cmd)?;

    let digest_ref = resolve_repo_digest(&image_ref, &args.repository)?;
    println!("[INFO] Image pushed: {digest_ref}");
    println!("[INFO] Rollout commands:");
    println!(
        "kubectl set image deployment/{} -n {} {}={}",
        args.deployment, args.namespace, args.container, digest_ref
    );
    println!(
        "kubectl rollout status deployment/{} -n {}",
        args.deployment, args.namespace
    );

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
