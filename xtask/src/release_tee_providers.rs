use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use sha2::{Digest, Sha256};

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
const DEFAULT_BUILDX_BUILDER: &str = "raiko2-local-cache";
const ENV_GCP_ENCLAVE_KEY_SECRET: &str = "GCP_ENCLAVE_KEY_SECRET";
const ENV_GCP_ENCLAVE_KEY_VERSION: &str = "GCP_ENCLAVE_KEY_VERSION";
const ENV_GCP_ENCLAVE_KEY_PROJECT: &str = "GCP_ENCLAVE_KEY_PROJECT";
const ENV_RAIKO2_SGX_ENCLAVE_KEY_HOST: &str = "RAIKO2_SGX_ENCLAVE_KEY_HOST";

#[derive(Debug)]
struct GramineSigningKey {
    path: PathBuf,
    _temp: Option<tempfile::NamedTempFile>,
}

impl GramineSigningKey {
    fn local(path: PathBuf) -> Self {
        Self { path, _temp: None }
    }

    fn temp(temp: tempfile::NamedTempFile) -> Self {
        Self {
            path: temp.path().to_path_buf(),
            _temp: Some(temp),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone, Copy)]
struct DockerBuildSecret<'a> {
    id: &'a str,
    path: &'a Path,
}

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
    docker_build_external_tee_provider(&build_context, &dockerfile, &image_ref)?;
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

fn docker_build_command(
    context: &Path,
    dockerfile: &Path,
    image_ref: &str,
    build_args: &[String],
    secret: Option<DockerBuildSecret<'_>>,
) -> Command {
    let mut cmd = Command::new("docker");
    cmd.env("DOCKER_BUILDKIT", "1");
    cmd.arg("build");
    for build_arg in build_args {
        cmd.arg("--build-arg").arg(build_arg);
    }
    if let Some(secret) = secret {
        cmd.arg("--secret")
            .arg(format!("id={},src={}", secret.id, secret.path.display()));
    }
    cmd.arg("-f")
        .arg(dockerfile)
        .arg("-t")
        .arg(image_ref)
        .arg(context);
    cmd
}

fn docker_build_local_sgx(context: &Path, dockerfile: &Path, image_ref: &str) -> Result<()> {
    let secret_src = resolve_gramine_enclave_key(context)?;
    let key_sha256 = file_sha256_hex(secret_src.path())?;
    let build_args = vec![format!("GRAMINE_ENCLAVE_KEY_SHA256={key_sha256}")];
    let cmd = docker_build_command(
        context,
        dockerfile,
        image_ref,
        &build_args,
        Some(DockerBuildSecret {
            id: "gramine_enclave_key",
            path: secret_src.path(),
        }),
    );
    util::run(cmd)
}

fn docker_build_external_tee_provider(
    context: &Path,
    dockerfile: &Path,
    image_ref: &str,
) -> Result<()> {
    let secret_src = resolve_gramine_enclave_key(context)?;
    let key_public_sha256 = rsa_public_key_sha256_hex(secret_src.path())?;
    let cmd = external_provider_docker_build_command(
        context,
        dockerfile,
        image_ref,
        secret_src.path(),
        &key_public_sha256,
    );
    util::run(cmd)
}

fn external_provider_docker_build_command(
    context: &Path,
    dockerfile: &Path,
    image_ref: &str,
    enclave_key_path: &Path,
    enclave_key_public_sha256: &str,
) -> Command {
    docker_build_command(
        context,
        dockerfile,
        image_ref,
        &[format!(
            "ENCLAVE_KEY_PUBLIC_SHA256={enclave_key_public_sha256}"
        )],
        Some(DockerBuildSecret {
            id: "enclave_key",
            path: enclave_key_path,
        }),
    )
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

fn resolve_gramine_enclave_key(root: &Path) -> Result<GramineSigningKey> {
    resolve_gramine_enclave_key_from_values(
        root,
        env_var(ENV_GCP_ENCLAVE_KEY_SECRET),
        env_var(ENV_RAIKO2_SGX_ENCLAVE_KEY_HOST),
    )
}

fn resolve_gramine_enclave_key_from_values(
    root: &Path,
    gcp_secret: Option<String>,
    local_path: Option<String>,
) -> Result<GramineSigningKey> {
    if let Some(secret) = gcp_secret {
        return fetch_gcp_enclave_key(&secret);
    }

    if let Some(path) = local_path {
        return local_gramine_enclave_key_path(PathBuf::from(path));
    }

    bail!(
        "missing Gramine enclave signing key; set {ENV_GCP_ENCLAVE_KEY_SECRET} for release builds \
         or {ENV_RAIKO2_SGX_ENCLAVE_KEY_HOST} for local builds from {}",
        root.display()
    )
}

fn fetch_gcp_enclave_key(secret: &str) -> Result<GramineSigningKey> {
    let version = env_var(ENV_GCP_ENCLAVE_KEY_VERSION).unwrap_or_else(|| "latest".to_string());

    ensure_gcloud()?;
    let project = env_var(ENV_GCP_ENCLAVE_KEY_PROJECT);
    let output = gcp_secret_access_command(secret, &version, project.as_deref())
        .output()
        .with_context(|| format!("failed to run gcloud for GCP Secret Manager secret {secret}"))?;
    if !output.status.success() {
        bail!("gcloud failed to fetch enclave signing key from secret {secret}");
    }

    let mut temp =
        tempfile::NamedTempFile::new().context("create temporary enclave signing key file")?;
    let path = temp.path().to_path_buf();
    restrict_file_permissions(&path)?;
    temp.write_all(&output.stdout)
        .with_context(|| format!("failed to write {}", path.display()))?;
    temp.flush()
        .with_context(|| format!("failed to flush {}", path.display()))?;
    ensure_non_empty_file(&path)?;

    Ok(GramineSigningKey::temp(temp))
}

fn gcp_secret_access_command(secret: &str, version: &str, project: Option<&str>) -> Command {
    let mut cmd = Command::new("gcloud");
    cmd.arg("secrets")
        .arg("versions")
        .arg("access")
        .arg(version)
        .arg("--secret")
        .arg(secret);
    if let Some(project) = project.filter(|value| !value.trim().is_empty()) {
        cmd.arg("--project").arg(project);
    }
    cmd
}

fn ensure_gcloud() -> Result<()> {
    let mut cmd = Command::new("gcloud");
    cmd.arg("--version");
    util::ensure_command(
        cmd,
        "gcloud",
        "Install the Google Cloud SDK or unset GCP_ENCLAVE_KEY_SECRET.",
    )
}

fn local_gramine_enclave_key_path(path: PathBuf) -> Result<GramineSigningKey> {
    if !path.exists() {
        bail!("missing Gramine enclave signing key at {}", path.display());
    }
    ensure_non_empty_file(&path)?;
    Ok(GramineSigningKey::local(path))
}

fn env_var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn ensure_non_empty_file(path: &Path) -> Result<()> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        bail!("Gramine enclave signing key is empty: {}", path.display());
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to chmod 0600 {}", path.display()))
}

#[cfg(not(unix))]
fn restrict_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn file_sha256_hex(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read signing key {}", path.display()))?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn rsa_public_key_sha256_hex(path: &Path) -> Result<String> {
    let output = Command::new("openssl")
        .arg("rsa")
        .arg("-in")
        .arg(path)
        .arg("-pubout")
        .output()
        .with_context(|| {
            format!(
                "failed to derive enclave signing public key from {}",
                path.display()
            )
        })?;
    if !output.status.success() {
        bail!(
            "openssl failed to derive enclave signing public key from {}",
            path.display()
        );
    }

    let digest = Sha256::digest(output.stdout);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
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
        ENV_GCP_ENCLAVE_KEY_SECRET, ENV_RAIKO2_SGX_ENCLAVE_KEY_HOST,
        external_provider_docker_build_command, external_source_checkout_dir, file_sha256_hex,
        gcp_secret_access_command, local_gramine_enclave_key_path, local_provider_image_ref,
        parse_attestation_json, resolve_gramine_enclave_key_from_values, validate_attestation_path,
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
        let temp = tempfile::tempdir().expect("temp repo root");
        let root = temp.path();
        assert_eq!(
            external_source_checkout_dir(root, "v1.2.3", "gaiko2"),
            root.join("target")
                .join("tee-release")
                .join("v1.2.3")
                .join("sources")
                .join("gaiko2")
        );
    }

    #[test]
    fn release_tee_providers_rejects_non_absolute_attestation_path() {
        let err = validate_attestation_path("attestation.json").expect_err("relative path fails");
        assert!(err.to_string().contains("must be absolute"));
    }

    #[test]
    fn release_tee_providers_rejects_missing_signing_key_env() {
        let temp = tempfile::tempdir().expect("temp repo root");
        let root = temp.path();
        let err = resolve_gramine_enclave_key_from_values(root, None, None)
            .expect_err("missing signing key env fails");

        assert!(err.to_string().contains(ENV_GCP_ENCLAVE_KEY_SECRET));
        assert!(err.to_string().contains(ENV_RAIKO2_SGX_ENCLAVE_KEY_HOST));
    }

    #[test]
    fn release_tee_providers_builds_gcp_secret_access_command() {
        let command = gcp_secret_access_command("raiko2-sgx-key", "42", Some("taiko-project"));
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(command.get_program().to_string_lossy(), "gcloud");
        assert_eq!(
            args,
            vec![
                "secrets",
                "versions",
                "access",
                "42",
                "--secret",
                "raiko2-sgx-key",
                "--project",
                "taiko-project",
            ]
        );
    }

    #[test]
    fn release_tee_providers_external_provider_build_uses_enclave_key_secret() {
        let temp = tempfile::tempdir().expect("temp build paths");
        let context = temp.path().join("provider");
        let dockerfile = context.join("docker").join("Dockerfile.tee");
        let enclave_key = temp.path().join("enclave-key.pem");
        std::fs::write(&enclave_key, b"test-key").expect("write test key");
        let command = external_provider_docker_build_command(
            &context,
            &dockerfile,
            "us-docker.pkg.dev/evmchain/images/gaiko2-sgxgeth:v1.2.3",
            &enclave_key,
            "public-key-sha256",
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let expected_secret = format!("id=enclave_key,src={}", enclave_key.display());
        let expected_build_arg = "ENCLAVE_KEY_PUBLIC_SHA256=public-key-sha256".to_string();

        assert_eq!(command.get_program().to_string_lossy(), "docker");
        assert!(
            args.windows(2)
                .any(|pair| { pair == ["--secret".to_string(), expected_secret.clone()] })
        );
        assert!(
            args.windows(2)
                .any(|pair| { pair == ["--build-arg".to_string(), expected_build_arg.clone()] })
        );
    }

    #[test]
    fn release_tee_providers_rejects_empty_local_signing_key() {
        let temp = tempfile::NamedTempFile::new().expect("temp signing key");
        let err = local_gramine_enclave_key_path(temp.path().to_path_buf())
            .expect_err("empty signing key fails");

        assert!(
            err.to_string()
                .contains("Gramine enclave signing key is empty")
        );
    }

    #[test]
    fn release_tee_providers_hashes_signing_key_contents() {
        let temp = tempfile::NamedTempFile::new().expect("temp signing key");
        std::fs::write(temp.path(), b"abc").expect("write temp signing key");
        assert_eq!(
            file_sha256_hex(temp.path()).expect("hash signing key"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
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
