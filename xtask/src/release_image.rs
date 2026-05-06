use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use xtask_build_guest::Backend;

use crate::util;

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
    ensure_clean_source_tree(root, "before release-image starts")?;

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
        xtask_build_guest::build(root, args.backend, false, None)?;
    } else {
        xtask_build_guest::ensure_release_guest_elves(root, args.backend, false, None)?;
    }
    ensure_clean_source_tree(
        root,
        "after refreshing guest ELFs for release-image; review and commit updated release artifacts before retrying",
    )?;

    println!(
        "[INFO] Building runtime image `{image_ref}` with buildx local cache at {:?}...",
        buildx_cache_current
    );
    let source_revision = source_revision(root)?;
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
        .args(build_metadata_flags(&source_revision))
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
    write_release_summary(io::stdout(), &digest_ref)?;

    Ok(())
}

const fn backend_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Risc0 => "risc0",
        Backend::Sp1 => "sp1",
        Backend::Tdx => "tdx",
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

fn build_metadata_flags(source_revision: &str) -> Vec<String> {
    vec![
        "--build-arg".to_string(),
        format!("VCS_REF={source_revision}"),
    ]
}

fn write_release_summary<W: io::Write>(mut writer: W, digest_ref: &str) -> io::Result<()> {
    for line in release_summary_lines(digest_ref) {
        writeln!(writer, "{line}")?;
    }
    Ok(())
}

fn source_revision(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .with_context(|| format!("failed to read git revision at {root:?}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            bail!("git rev-parse HEAD failed at {root:?}");
        }
        bail!("git rev-parse HEAD failed at {root:?}: {stderr}");
    }

    let revision = String::from_utf8(output.stdout)
        .with_context(|| format!("git rev-parse HEAD produced non-utf8 output at {root:?}"))?;
    let revision = revision.trim().to_string();
    ensure_non_empty("source revision", &revision)?;
    Ok(revision)
}

fn ensure_clean_source_tree(root: &Path, requirement: &str) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("status")
        .arg("--short")
        .arg("--untracked-files=all")
        .output()
        .with_context(|| format!("failed to inspect git worktree at {root:?}"))?;
    if !output.status.success() {
        bail!("git status failed at {root:?}");
    }

    let status = String::from_utf8(output.stdout)
        .with_context(|| format!("git status produced non-utf8 output at {root:?}"))?;
    if !status.trim().is_empty() {
        bail!(
            "raiko2 worktree at {root:?} must be clean {requirement}:\n{}",
            status.trim_end(),
        );
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
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{build_metadata_flags, release_summary_lines, write_release_summary};

    #[test]
    fn release_summary_lines_do_not_reference_rollout() {
        let digest_ref = "us-docker.pkg.dev/evmchain/images/raiko2@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let lines = release_summary_lines(digest_ref);
        let mut output = Vec::new();

        write_release_summary(&mut output, digest_ref).expect("summary output should render");
        let output = String::from_utf8(output).expect("summary output should be valid utf-8");

        assert_eq!(lines, vec![format!("[INFO] Image pushed: {digest_ref}")]);
        assert_eq!(output, format!("[INFO] Image pushed: {digest_ref}\n"));
        assert!(!output.contains("kubectl"));
        assert!(!output.contains("rollout"));
        assert!(!output.contains("deployment/"));
    }

    #[test]
    fn build_metadata_flags_include_revision_label() {
        let flags = build_metadata_flags("26eff23");

        assert_eq!(
            flags,
            vec!["--build-arg".to_string(), "VCS_REF=26eff23".to_string()]
        );
    }

    #[test]
    fn clean_source_tree_rejects_dirty_worktree() {
        let repo_root = temp_git_repo("dirty");
        fs::write(repo_root.join("dirty.txt"), "dirty").expect("should write dirty marker");

        let err = super::ensure_clean_source_tree(&repo_root, "before release-image starts")
            .expect_err("dirty repo should be rejected");
        assert!(err.to_string().contains("before release-image starts"));
    }

    #[test]
    fn clean_source_tree_accepts_clean_worktree() {
        let repo_root = temp_git_repo("clean");
        super::ensure_clean_source_tree(&repo_root, "before release-image starts")
            .expect("clean repo should be accepted");
    }

    #[test]
    fn clean_source_tree_reports_guest_refresh_hint() {
        let repo_root = temp_git_repo("guest-refresh-dirty");
        fs::write(repo_root.join("dirty.txt"), "dirty").expect("should write dirty marker");

        let err = super::ensure_clean_source_tree(
            &repo_root,
            "after refreshing guest ELFs for release-image; review and commit updated release artifacts before retrying",
        )
        .expect_err("dirty repo should be rejected");
        assert!(
            err.to_string()
                .contains("review and commit updated release artifacts before retrying")
        );
    }

    fn temp_git_repo(suffix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let repo_root = std::env::temp_dir().join(format!(
            "raiko2-release-image-test-{suffix}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&repo_root).expect("should create temp repo");

        let status = Command::new("git")
            .arg("init")
            .arg(&repo_root)
            .status()
            .expect("git init should run");
        assert!(status.success(), "git init should succeed");
        repo_root
    }
}
