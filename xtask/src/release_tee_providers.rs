use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;

use crate::release_tee_manifest::{
    TeeAttestationManifest, TeeProviderAttestation, TeeProviderImage, TeeProviderManifestEntry,
    TeeProviderSource, release_manifest_path, write_manifest,
};
use crate::tee_provider_lock::{TeeProviderEntry, load};
use crate::util;

const DEFAULT_LOCAL_PROVIDER: &str = "raiko2-sgx";
const DEFAULT_LOCAL_LANE: &str = "sgx";
const DEFAULT_LOCAL_REPOSITORY: &str = "us-docker.pkg.dev/evmchain/images/raiko2-sgx";
const DEFAULT_LOCAL_DOCKERFILE: &str = "Dockerfile.sgx";
const DEFAULT_LOCAL_ATTESTATION_PATH: &str = "/opt/raiko2-sgx/etc/attestation.raiko2.json";
const DEFAULT_GRAMINE_ENCLAVE_KEY_PATH: &str = ".config/gramine/enclave-key.pem";
const DEFAULT_BUILDX_BUILDER: &str = "raiko2-local-cache";

#[derive(Args, Debug)]
pub(crate) struct ReleaseTeeProvidersArgs {
    #[arg(long)]
    pub(crate) tag: String,

    #[arg(long, default_value_t = false)]
    pub(crate) no_push: bool,
}

pub(crate) fn run(root: &Path, args: ReleaseTeeProvidersArgs) -> Result<()> {
    ensure_non_empty("tag", &args.tag)?;
    util::ensure_docker()?;
    util::ensure_docker_buildx()?;
    util::ensure_docker_buildx_builder(DEFAULT_BUILDX_BUILDER)?;
    ensure_clean_source_tree(root, "before release-tee-providers starts")?;

    let provider_lock = load(&provider_lock_path(root))?;
    let manifest = build_manifest(root, &args.tag, args.no_push, &provider_lock.providers)?;
    let output_path = release_manifest_path(root, &args.tag);
    write_manifest(&output_path, &manifest)?;
    println!(
        "[INFO] TEE attestation manifest written: {}",
        output_path.display()
    );

    Ok(())
}

fn build_manifest(
    root: &Path,
    tag: &str,
    no_push: bool,
    providers: &std::collections::BTreeMap<String, TeeProviderEntry>,
) -> Result<TeeAttestationManifest> {
    let generated_at = current_timestamp_rfc3339()?;
    let local = build_local_provider_entry(root, tag, no_push)?;
    let mut entries = vec![local];

    for (name, provider) in providers {
        entries.push(build_external_provider_entry(
            root, tag, no_push, name, provider,
        )?);
    }

    Ok(TeeAttestationManifest {
        release: tag.to_string(),
        generated_at,
        providers: entries,
    })
}

fn build_local_provider_entry(
    root: &Path,
    tag: &str,
    no_push: bool,
) -> Result<TeeProviderManifestEntry> {
    let image_ref = local_provider_image_ref(tag, DEFAULT_LOCAL_REPOSITORY);
    let build_context = root;
    let dockerfile = root.join(DEFAULT_LOCAL_DOCKERFILE);
    docker_build_local_sgx(build_context, &dockerfile, &image_ref)?;
    let digest = if no_push {
        format!("{DEFAULT_LOCAL_REPOSITORY}:{tag}")
    } else {
        docker_push(&image_ref)?;
        resolve_repo_digest(&image_ref, DEFAULT_LOCAL_REPOSITORY)?
    };
    let attestation = read_attestation_json(&image_ref, DEFAULT_LOCAL_ATTESTATION_PATH)?;
    let source = TeeProviderSource {
        repo: "local".to_string(),
        commit: source_revision(root)?,
    };

    Ok(TeeProviderManifestEntry {
        lane: DEFAULT_LOCAL_LANE.to_string(),
        provider: DEFAULT_LOCAL_PROVIDER.to_string(),
        source,
        image: TeeProviderImage {
            repository: DEFAULT_LOCAL_REPOSITORY.to_string(),
            tag: tag.to_string(),
            digest,
        },
        attestation,
    })
}

fn build_external_provider_entry(
    root: &Path,
    tag: &str,
    no_push: bool,
    provider_name: &str,
    provider: &TeeProviderEntry,
) -> Result<TeeProviderManifestEntry> {
    validate_attestation_path(&provider.attestation_path)?;

    let checkout_dir = external_source_checkout_dir(root, tag, provider_name);
    clone_provider_source(&provider.repo, &provider.commit, &checkout_dir)?;

    let image_ref = local_provider_image_ref(tag, &provider.repository);
    let dockerfile = checkout_dir.join(&provider.dockerfile);
    let build_context = checkout_dir.join(&provider.context);
    docker_build(&build_context, &dockerfile, &image_ref)?;
    let digest = if no_push {
        format!("{}:{}", provider.repository, tag)
    } else {
        docker_push(&image_ref)?;
        resolve_repo_digest(&image_ref, &provider.repository)?
    };
    let attestation = read_attestation_json(&image_ref, &provider.attestation_path)?;

    Ok(TeeProviderManifestEntry {
        lane: provider.lane.clone(),
        provider: provider.provider.clone(),
        source: TeeProviderSource {
            repo: provider.repo.clone(),
            commit: provider.commit.clone(),
        },
        image: TeeProviderImage {
            repository: provider.repository.clone(),
            tag: tag.to_string(),
            digest,
        },
        attestation,
    })
}

fn provider_lock_path(root: &Path) -> PathBuf {
    root.join("release").join("providers.toml")
}

fn local_provider_image_ref(tag: &str, repository: &str) -> String {
    format!("{repository}:{tag}")
}

fn external_source_checkout_dir(root: &Path, tag: &str, provider: &str) -> PathBuf {
    util::target_root(root)
        .join("tee-release")
        .join(tag)
        .join("sources")
        .join(provider)
}

fn validate_attestation_path(path: &str) -> Result<()> {
    if path.trim().is_empty() {
        bail!("attestation_path must not be empty");
    }
    if !path.starts_with('/') {
        bail!("attestation_path must be absolute: {path}");
    }
    Ok(())
}

fn ensure_non_empty(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(())
}

fn current_timestamp_rfc3339() -> Result<String> {
    let now = time::OffsetDateTime::now_utc();
    now.format(&time::format_description::well_known::Rfc3339)
        .context("format current time as RFC3339")
}

fn source_revision(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .with_context(|| format!("failed to read git revision at {}", root.display()))?;
    if !output.status.success() {
        bail!("git rev-parse HEAD failed at {}", root.display());
    }
    let revision = String::from_utf8(output.stdout).with_context(|| {
        format!(
            "git rev-parse HEAD produced non-utf8 output at {}",
            root.display()
        )
    })?;
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
        .with_context(|| format!("failed to inspect git worktree at {}", root.display()))?;
    if !output.status.success() {
        bail!("git status failed at {}", root.display());
    }

    let status = String::from_utf8(output.stdout)
        .with_context(|| format!("git status produced non-utf8 output at {}", root.display()))?;
    if !status.trim().is_empty() {
        bail!(
            "raiko2 worktree at {} must be clean {requirement}:\n{}",
            root.display(),
            status.trim_end(),
        );
    }
    Ok(())
}

fn clone_provider_source(repo: &str, commit: &str, path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove existing checkout {}", path.display()))?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let mut clone_cmd = Command::new("git");
    clone_cmd.arg("clone").arg(repo).arg(path);
    util::run(clone_cmd)?;

    let mut checkout_cmd = Command::new("git");
    checkout_cmd.arg("-C").arg(path).arg("checkout").arg(commit);
    util::run(checkout_cmd)
}

fn docker_build(context: &Path, dockerfile: &Path, image_ref: &str) -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.env("DOCKER_BUILDKIT", "1");
    cmd.arg("build")
        .arg("-f")
        .arg(dockerfile)
        .arg("-t")
        .arg(image_ref)
        .arg(context);
    util::run(cmd)
}

fn docker_build_local_sgx(context: &Path, dockerfile: &Path, image_ref: &str) -> Result<()> {
    let secret_src = default_gramine_enclave_key_path()?;
    let mut cmd = Command::new("docker");
    cmd.env("DOCKER_BUILDKIT", "1");
    cmd.arg("build")
        .arg("--secret")
        .arg(format!(
            "id=gramine_enclave_key,src={}",
            secret_src.display()
        ))
        .arg("-f")
        .arg(dockerfile)
        .arg("-t")
        .arg(image_ref)
        .arg(context);
    util::run(cmd)
}

fn docker_push(image_ref: &str) -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.arg("push").arg(image_ref);
    util::run(cmd)
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

fn read_attestation_json(
    image_ref: &str,
    attestation_path: &str,
) -> Result<TeeProviderAttestation> {
    let output = Command::new("docker")
        .arg("run")
        .arg("--rm")
        .arg("--entrypoint")
        .arg("cat")
        .arg(image_ref)
        .arg(attestation_path)
        .output()
        .with_context(|| format!("failed to read attestation metadata from {image_ref}"))?;
    if !output.status.success() {
        bail!("docker run cat failed for {image_ref}:{attestation_path}");
    }

    parse_attestation_json(std::str::from_utf8(&output.stdout).context("attestation is not utf-8")?)
}

fn default_gramine_enclave_key_path() -> Result<PathBuf> {
    let home =
        std::env::var("HOME").context("HOME is required to locate the Gramine signing key")?;
    let path = Path::new(&home).join(DEFAULT_GRAMINE_ENCLAVE_KEY_PATH);
    if !path.exists() {
        bail!("missing Gramine enclave signing key at {}", path.display());
    }
    Ok(path)
}

fn parse_attestation_json(raw: &str) -> Result<TeeProviderAttestation> {
    let value: serde_json::Value = serde_json::from_str(raw).context("parse attestation json")?;
    let mr_enclave = string_field(&value, &["mr_enclave", "unique_id"])?;
    let mr_signer = string_field(&value, &["mr_signer", "signer_id"])?;
    let isv_prod_id = value
        .get("isv_prod_id")
        .or_else(|| value.get("product_id"))
        .and_then(serde_json::Value::as_u64)
        .map(|v| u32::try_from(v).expect("u64 product id should fit into u32"));
    let isv_svn = value
        .get("isv_svn")
        .or_else(|| value.get("security_version"))
        .and_then(serde_json::Value::as_u64)
        .map(|v| u32::try_from(v).expect("u64 security version should fit into u32"));
    let debug_enclave = value
        .get("debug_enclave")
        .and_then(serde_json::Value::as_bool);

    Ok(TeeProviderAttestation {
        mr_enclave,
        mr_signer,
        isv_prod_id,
        isv_svn,
        debug_enclave,
    })
}

fn string_field(value: &serde_json::Value, names: &[&str]) -> Result<String> {
    for name in names {
        if let Some(raw) = value.get(*name).and_then(serde_json::Value::as_str) {
            let trimmed = raw.trim().to_string();
            if !trimmed.is_empty() {
                return Ok(trimmed);
            }
        }
    }
    bail!("missing required string field: {}", names.join(" or "))
}

#[cfg(test)]
mod tests {
    use super::{
        external_source_checkout_dir, local_provider_image_ref, parse_attestation_json,
        validate_attestation_path,
    };

    #[test]
    fn release_tee_providers_builds_local_image_ref() {
        assert_eq!(
            local_provider_image_ref("v1.2.3", "us-docker.pkg.dev/evmchain/images/raiko2-sgx"),
            "us-docker.pkg.dev/evmchain/images/raiko2-sgx:v1.2.3"
        );
    }

    #[test]
    fn release_tee_providers_checkout_dir_is_tagged_and_namespaced() {
        let root = std::path::Path::new("/tmp/raiko2");
        assert_eq!(
            external_source_checkout_dir(root, "v1.2.3", "gaiko2"),
            std::path::Path::new("/tmp/raiko2/target/tee-release/v1.2.3/sources/gaiko2")
        );
    }

    #[test]
    fn release_tee_providers_rejects_non_absolute_attestation_path() {
        let err = validate_attestation_path("attestation.json").expect_err("relative path fails");
        assert!(err.to_string().contains("must be absolute"));
    }

    #[test]
    fn release_tee_providers_parses_attestation_alias_fields() {
        let attestation = parse_attestation_json(
            r#"{
  "unique_id": "abc",
  "signer_id": "def",
  "product_id": 1,
  "security_version": 2
}"#,
        )
        .expect("parse attestation");

        assert_eq!(attestation.mr_enclave, "abc");
        assert_eq!(attestation.mr_signer, "def");
        assert_eq!(attestation.isv_prod_id, Some(1));
        assert_eq!(attestation.isv_svn, Some(2));
        assert_eq!(attestation.debug_enclave, None);
    }
}
