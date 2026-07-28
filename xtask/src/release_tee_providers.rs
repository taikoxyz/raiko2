use std::collections::{BTreeMap, BTreeSet};
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
const OCI_TAG_MAX_LEN: usize = 128;
const LOCAL_SGX_EDMM_TAG_SUFFIX: &str = "-edmm";

#[derive(Debug, Clone, Copy)]
struct LocalSgxVariant {
    provider: &'static str,
    edmm: bool,
}

const LOCAL_SGX_VARIANTS: [LocalSgxVariant; 2] = [
    LocalSgxVariant {
        provider: DEFAULT_LOCAL_PROVIDER,
        edmm: false,
    },
    LocalSgxVariant {
        provider: "raiko2-sgx-edmm",
        edmm: true,
    },
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ArtifactRegistryRepository {
    location: String,
    project: String,
    repository: String,
}

impl ArtifactRegistryRepository {
    fn display_name(&self) -> String {
        format!("{}/{}/{}", self.location, self.project, self.repository)
    }
}

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
    validate_release_tag(&args.tag)?;
    util::ensure_docker()?;
    util::ensure_docker_buildx()?;
    util::ensure_docker_buildx_builder(DEFAULT_BUILDX_BUILDER)?;
    ensure_clean_source_tree(root, "before release-tee-providers starts")?;

    let provider_lock = load(&provider_lock_path(root))?;
    if !args.no_push {
        ensure_release_destination_repositories_immutable(&provider_lock.providers)?;
        ensure_release_destination_tags_unpublished(&args.tag, &provider_lock.providers)?;
    }
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
    providers: &BTreeMap<String, TeeProviderEntry>,
) -> Result<TeeAttestationManifest> {
    let generated_at = current_timestamp_rfc3339()?;
    let mut entries = build_local_provider_entries(root, tag, no_push)?;

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

fn ensure_release_destination_tags_unpublished(
    tag: &str,
    providers: &BTreeMap<String, TeeProviderEntry>,
) -> Result<()> {
    for image_ref in release_destination_image_refs(tag, providers) {
        ensure_remote_image_tag_unpublished(&image_ref)?;
    }
    Ok(())
}

fn ensure_release_destination_repositories_immutable(
    providers: &BTreeMap<String, TeeProviderEntry>,
) -> Result<()> {
    for repository in release_destination_artifact_registry_repositories(providers)? {
        ensure_artifact_registry_repository_immutable_tags(&repository)?;
    }
    Ok(())
}

fn release_destination_artifact_registry_repositories(
    providers: &BTreeMap<String, TeeProviderEntry>,
) -> Result<BTreeSet<ArtifactRegistryRepository>> {
    let mut repositories = BTreeSet::new();
    repositories.insert(artifact_registry_repository_from_image_repository(
        DEFAULT_LOCAL_REPOSITORY,
    )?);
    for provider in providers.values() {
        repositories.insert(artifact_registry_repository_from_image_repository(
            &provider.repository,
        )?);
    }
    Ok(repositories)
}

fn release_destination_image_refs(
    tag: &str,
    providers: &BTreeMap<String, TeeProviderEntry>,
) -> BTreeSet<String> {
    let mut image_refs = BTreeSet::new();
    for variant in LOCAL_SGX_VARIANTS {
        let variant_tag = local_sgx_variant_tag(tag, variant.edmm);
        image_refs.insert(local_provider_image_ref(
            &variant_tag,
            DEFAULT_LOCAL_REPOSITORY,
        ));
    }
    for provider in providers.values() {
        image_refs.insert(local_provider_image_ref(tag, &provider.repository));
    }
    image_refs
}

fn artifact_registry_repository_from_image_repository(
    image_repository: &str,
) -> Result<ArtifactRegistryRepository> {
    let parts = image_repository.split('/').collect::<Vec<_>>();
    if parts.len() < 4 {
        bail!(
            "release image repository must be an Artifact Registry Docker image path \
             LOCATION-docker.pkg.dev/PROJECT/REPOSITORY/IMAGE: {image_repository}"
        );
    }

    let host = parts[0];
    let location = host
        .strip_suffix("-docker.pkg.dev")
        .filter(|location| !location.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "release image repository must use Artifact Registry Docker host \
                 LOCATION-docker.pkg.dev: {image_repository}"
            )
        })?;
    let project = parts[1];
    let repository = parts[2];
    ensure_non_empty("Artifact Registry project", project)?;
    ensure_non_empty("Artifact Registry repository", repository)?;

    Ok(ArtifactRegistryRepository {
        location: location.to_string(),
        project: project.to_string(),
        repository: repository.to_string(),
    })
}

fn build_local_provider_entries(
    root: &Path,
    tag: &str,
    no_push: bool,
) -> Result<Vec<TeeProviderManifestEntry>> {
    let build_context = root;
    let dockerfile = root.join(DEFAULT_LOCAL_DOCKERFILE);
    let signing_key = resolve_gramine_enclave_key(root)?;
    let key_sha256 = file_sha256_hex(signing_key.path())?;
    let source_commit = source_revision(root)?;
    let mut entries = Vec::with_capacity(LOCAL_SGX_VARIANTS.len());

    for variant in LOCAL_SGX_VARIANTS {
        let variant_tag = local_sgx_variant_tag(tag, variant.edmm);
        let image_ref = local_provider_image_ref(&variant_tag, DEFAULT_LOCAL_REPOSITORY);
        docker_build_local_sgx(
            build_context,
            &dockerfile,
            &image_ref,
            signing_key.path(),
            &key_sha256,
            variant,
        )?;
        let attestation = read_attestation_json(&image_ref, DEFAULT_LOCAL_ATTESTATION_PATH)?;
        entries.push(local_sgx_manifest_entry(
            variant,
            &variant_tag,
            image_ref,
            &source_commit,
            attestation,
        ));
    }

    // Validate both variants while they are still local. Final release tags are only
    // published after the cross-variant attestation checks pass.
    validate_local_sgx_entries(&entries, true)?;

    if !no_push {
        for entry in &mut entries {
            let image_ref = local_provider_image_ref(&entry.image.tag, DEFAULT_LOCAL_REPOSITORY);
            docker_push(&image_ref)?;
            entry.image.digest = resolve_repo_digest(&image_ref, DEFAULT_LOCAL_REPOSITORY)?;
        }
        validate_local_sgx_entries(&entries, false)?;
    }

    Ok(entries)
}

fn validate_local_sgx_entries(entries: &[TeeProviderManifestEntry], no_push: bool) -> Result<()> {
    if entries.len() != 2 {
        bail!("local SGX variant invariant violated: expected exactly two entries");
    }

    let non_edmm = &entries[0];
    let edmm = &entries[1];
    if non_edmm.image.tag == edmm.image.tag {
        bail!("local SGX variant invariant violated: image tags must differ");
    }
    if non_edmm.attestation.mr_enclave == edmm.attestation.mr_enclave {
        bail!("local SGX variant invariant violated: MRENCLAVE values must differ");
    }
    if non_edmm.attestation.mr_signer != edmm.attestation.mr_signer {
        bail!("local SGX variant invariant violated: MRSIGNER values must match");
    }
    if !no_push && non_edmm.image.digest == edmm.image.digest {
        bail!("local SGX variant invariant violated: pushed image digests must differ");
    }

    Ok(())
}

fn local_sgx_variant_tag(release_tag: &str, edmm: bool) -> String {
    if edmm {
        format!("{release_tag}{LOCAL_SGX_EDMM_TAG_SUFFIX}")
    } else {
        release_tag.to_string()
    }
}

fn validate_release_tag(tag: &str) -> Result<()> {
    validate_oci_tag(tag, "release tag")?;
    if tag.ends_with(LOCAL_SGX_EDMM_TAG_SUFFIX) {
        bail!(
            "release tag {tag:?} uses reserved local SGX EDMM suffix {LOCAL_SGX_EDMM_TAG_SUFFIX:?}"
        );
    }
    for variant in LOCAL_SGX_VARIANTS {
        let variant_tag = local_sgx_variant_tag(tag, variant.edmm);
        validate_oci_tag(
            &variant_tag,
            &format!("local SGX provider tag for {}", variant.provider),
        )?;
    }
    Ok(())
}

fn validate_oci_tag(tag: &str, label: &str) -> Result<()> {
    if tag.is_empty() {
        bail!("{label} must not be empty");
    }
    if tag.len() > OCI_TAG_MAX_LEN {
        bail!(
            "{label} {tag:?} is too long for an OCI image tag: {} bytes > {OCI_TAG_MAX_LEN}",
            tag.len()
        );
    }

    let mut chars = tag.chars();
    let first = chars.next().expect("empty tag handled above");
    if !first.is_ascii_alphanumeric() && first != '_' {
        bail!("{label} {tag:?} must start with [A-Za-z0-9_]");
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-')) {
        bail!("{label} {tag:?} contains characters outside [A-Za-z0-9_.-]");
    }

    Ok(())
}

fn local_sgx_manifest_entry(
    variant: LocalSgxVariant,
    tag: &str,
    digest: String,
    source_commit: &str,
    attestation: TeeProviderAttestation,
) -> TeeProviderManifestEntry {
    TeeProviderManifestEntry {
        lane: DEFAULT_LOCAL_LANE.to_string(),
        provider: variant.provider.to_string(),
        source: TeeProviderSource {
            repo: "local".to_string(),
            commit: source_commit.to_string(),
        },
        image: TeeProviderImage {
            repository: DEFAULT_LOCAL_REPOSITORY.to_string(),
            tag: tag.to_string(),
            digest,
            sgx_edmm: Some(variant.edmm),
        },
        attestation,
    }
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
            sgx_edmm: None,
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

fn docker_build_local_sgx(
    context: &Path,
    dockerfile: &Path,
    image_ref: &str,
    enclave_key_path: &Path,
    enclave_key_sha256: &str,
    variant: LocalSgxVariant,
) -> Result<()> {
    let cmd = local_sgx_docker_build_command(
        context,
        dockerfile,
        image_ref,
        enclave_key_path,
        enclave_key_sha256,
        variant,
    );
    util::run(cmd)
}

fn local_sgx_docker_build_command(
    context: &Path,
    dockerfile: &Path,
    image_ref: &str,
    enclave_key_path: &Path,
    enclave_key_sha256: &str,
    variant: LocalSgxVariant,
) -> Command {
    docker_build_command(
        context,
        dockerfile,
        image_ref,
        &[
            format!("GRAMINE_ENCLAVE_KEY_SHA256={enclave_key_sha256}"),
            format!("SGX_EDMM_ENABLE={}", variant.edmm),
        ],
        Some(DockerBuildSecret {
            id: "gramine_enclave_key",
            path: enclave_key_path,
        }),
    )
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

fn ensure_remote_image_tag_unpublished(image_ref: &str) -> Result<()> {
    let output = docker_manifest_inspect_command(image_ref)
        .output()
        .with_context(|| format!("failed to inspect remote image tag {image_ref}"))?;
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        bail!("release destination image tag already exists: {image_ref}");
    }
    match classify_docker_manifest_inspect_failure(&output_text) {
        DockerManifestInspectFailure::Missing => Ok(()),
        DockerManifestInspectFailure::Blocking => bail!(
            "failed to confirm release destination image tag is unpublished: {image_ref}\n{}",
            output_text.trim_end(),
        ),
    }
}

fn ensure_artifact_registry_repository_immutable_tags(
    repository: &ArtifactRegistryRepository,
) -> Result<()> {
    ensure_gcloud()?;
    let output = artifact_registry_repository_describe_command(repository)
        .output()
        .with_context(|| {
            format!(
                "failed to inspect Artifact Registry repository {}",
                repository.display_name()
            )
        })?;
    if !output.status.success() {
        bail!(
            "failed to inspect Artifact Registry repository {}:\n{}",
            repository.display_name(),
            String::from_utf8_lossy(&output.stderr).trim_end(),
        );
    }
    if !artifact_registry_immutable_tags_enabled(&output.stdout)? {
        bail!(
            "Artifact Registry repository {} must enable immutable Docker tags before publishing \
             TEE provider release images",
            repository.display_name(),
        );
    }
    Ok(())
}

fn artifact_registry_repository_describe_command(
    repository: &ArtifactRegistryRepository,
) -> Command {
    let mut cmd = Command::new("gcloud");
    cmd.arg("artifacts")
        .arg("repositories")
        .arg("describe")
        .arg(&repository.repository)
        .arg("--project")
        .arg(&repository.project)
        .arg("--location")
        .arg(&repository.location)
        .arg("--format=json");
    cmd
}

fn artifact_registry_immutable_tags_enabled(raw: &[u8]) -> Result<bool> {
    let value: serde_json::Value =
        serde_json::from_slice(raw).context("parse Artifact Registry repository metadata")?;
    Ok(value
        .pointer("/dockerConfig/immutableTags")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false))
}

fn docker_manifest_inspect_command(image_ref: &str) -> Command {
    let mut cmd = Command::new("docker");
    cmd.arg("manifest").arg("inspect").arg(image_ref);
    cmd
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockerManifestInspectFailure {
    Missing,
    Blocking,
}

fn classify_docker_manifest_inspect_failure(output: &str) -> DockerManifestInspectFailure {
    if docker_manifest_inspect_reports_blocking_error(output) {
        return DockerManifestInspectFailure::Blocking;
    }
    if docker_manifest_inspect_reports_missing(output) {
        return DockerManifestInspectFailure::Missing;
    }
    DockerManifestInspectFailure::Blocking
}

fn docker_manifest_inspect_reports_blocking_error(output: &str) -> bool {
    let output = output.to_ascii_lowercase();
    [
        "authentication",
        "authenticate",
        "credential",
        "denied",
        "forbidden",
        "permission",
        "reauthentication",
        "unauthorized",
        "login",
        "service unavailable",
        "too many requests",
        "timeout",
        "timed out",
        "deadline",
        "request canceled",
        "connection",
        "temporary failure",
        "tls handshake",
        "certificate",
        "429",
        "500 internal server error",
        "502 bad gateway",
        "503",
        "504 gateway timeout",
    ]
    .iter()
    .any(|needle| output.contains(needle))
}

fn docker_manifest_inspect_reports_missing(output: &str) -> bool {
    let output = output.to_ascii_lowercase();
    output.contains("manifest unknown")
        || output.contains("no such manifest")
        || output.contains("name unknown")
        || output.contains("requested entity was not found")
        || (output.contains("manifest for") && output.contains("not found"))
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
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::process::Command;

    use super::{
        ArtifactRegistryRepository, DockerManifestInspectFailure, ENV_GCP_ENCLAVE_KEY_SECRET,
        ENV_RAIKO2_SGX_ENCLAVE_KEY_HOST, LOCAL_SGX_VARIANTS,
        artifact_registry_immutable_tags_enabled, artifact_registry_repository_describe_command,
        artifact_registry_repository_from_image_repository,
        classify_docker_manifest_inspect_failure, docker_manifest_inspect_command,
        external_provider_docker_build_command, external_source_checkout_dir, file_sha256_hex,
        gcp_secret_access_command, local_gramine_enclave_key_path, local_provider_image_ref,
        local_sgx_docker_build_command, local_sgx_manifest_entry, local_sgx_variant_tag,
        parse_attestation_json, release_destination_artifact_registry_repositories,
        release_destination_image_refs, resolve_gramine_enclave_key_from_values,
        validate_attestation_path, validate_local_sgx_entries, validate_release_tag,
    };
    use crate::release_tee_manifest::{TeeProviderAttestation, TeeProviderManifestEntry};
    use crate::tee_provider_lock::TeeProviderEntry;

    fn command_args(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn external_provider(repository: &str) -> TeeProviderEntry {
        TeeProviderEntry {
            repo: "https://github.com/taikoxyz/gaiko2.git".to_string(),
            commit: "abcdef1234567890".to_string(),
            provider: "gaiko2-sgxgeth".to_string(),
            lane: "sgxgeth".to_string(),
            image_name: "gaiko2-sgxgeth".to_string(),
            repository: repository.to_string(),
            dockerfile: "docker/Dockerfile.tee".to_string(),
            context: ".".to_string(),
            attestation_path: "/opt/gaiko2/etc/attestation.json".to_string(),
        }
    }

    fn valid_local_sgx_entries() -> Vec<TeeProviderManifestEntry> {
        LOCAL_SGX_VARIANTS
            .map(|variant| {
                let tag = local_sgx_variant_tag("v1.2.3", variant.edmm);
                local_sgx_manifest_entry(
                    variant,
                    &tag,
                    format!("example.invalid/raiko2-sgx@sha256:{tag}"),
                    "source-commit",
                    TeeProviderAttestation {
                        mr_enclave: format!("mr-enclave-{tag}"),
                        mr_signer: "shared-mr-signer".to_string(),
                        isv_prod_id: Some(0),
                        isv_svn: Some(0),
                        debug_enclave: Some(false),
                    },
                )
            })
            .into_iter()
            .collect()
    }

    #[test]
    fn release_tee_providers_builds_local_image_ref() {
        assert_eq!(
            local_provider_image_ref("v1.2.3", "us-docker.pkg.dev/evmchain/images/raiko2-sgx"),
            "us-docker.pkg.dev/evmchain/images/raiko2-sgx:v1.2.3"
        );
    }

    #[test]
    fn release_tee_providers_lists_all_destination_image_refs_before_push() {
        let mut providers = BTreeMap::new();
        providers.insert(
            "gaiko2".to_string(),
            external_provider("us-docker.pkg.dev/evmchain/images/gaiko2-sgxgeth"),
        );

        let refs = release_destination_image_refs("v1.2.3", &providers);

        assert_eq!(refs.len(), 3);
        assert!(refs.contains("us-docker.pkg.dev/evmchain/images/raiko2-sgx:v1.2.3"));
        assert!(refs.contains("us-docker.pkg.dev/evmchain/images/raiko2-sgx:v1.2.3-edmm"));
        assert!(refs.contains("us-docker.pkg.dev/evmchain/images/gaiko2-sgxgeth:v1.2.3"));
    }

    #[test]
    fn release_tee_providers_parses_artifact_registry_repository_from_image_path() {
        assert_eq!(
            artifact_registry_repository_from_image_repository(
                "us-docker.pkg.dev/evmchain/images/nested/raiko2-sgx"
            )
            .expect("parse Artifact Registry image path"),
            ArtifactRegistryRepository {
                location: "us".to_string(),
                project: "evmchain".to_string(),
                repository: "images".to_string(),
            }
        );
    }

    #[test]
    fn release_tee_providers_rejects_non_artifact_registry_repository() {
        let err = artifact_registry_repository_from_image_repository("ghcr.io/taikoxyz/raiko2-sgx")
            .expect_err("non Artifact Registry repo cannot provide enforced tag immutability");

        assert!(err.to_string().contains("LOCATION-docker.pkg.dev"));
    }

    #[test]
    fn release_tee_providers_deduplicates_destination_artifact_registry_repositories() {
        let mut providers = BTreeMap::new();
        providers.insert(
            "gaiko2".to_string(),
            external_provider("us-docker.pkg.dev/evmchain/images/gaiko2-sgxgeth"),
        );

        let repositories =
            release_destination_artifact_registry_repositories(&providers).expect("parse repos");

        assert_eq!(
            repositories.into_iter().collect::<Vec<_>>(),
            vec![ArtifactRegistryRepository {
                location: "us".to_string(),
                project: "evmchain".to_string(),
                repository: "images".to_string(),
            }]
        );
    }

    #[test]
    fn release_tee_providers_builds_artifact_registry_describe_command() {
        let repository = ArtifactRegistryRepository {
            location: "us".to_string(),
            project: "evmchain".to_string(),
            repository: "images".to_string(),
        };
        let command = artifact_registry_repository_describe_command(&repository);

        assert_eq!(command.get_program().to_string_lossy(), "gcloud");
        assert_eq!(
            command_args(&command),
            vec![
                "artifacts",
                "repositories",
                "describe",
                "images",
                "--project",
                "evmchain",
                "--location",
                "us",
                "--format=json",
            ]
        );
    }

    #[test]
    fn release_tee_providers_reads_artifact_registry_immutable_tag_setting() {
        assert!(
            artifact_registry_immutable_tags_enabled(br#"{"dockerConfig":{"immutableTags":true}}"#)
                .expect("parse enabled immutable tag flag")
        );
        assert!(
            !artifact_registry_immutable_tags_enabled(
                br#"{"dockerConfig":{"immutableTags":false}}"#
            )
            .expect("parse disabled immutable tag flag")
        );
        assert!(
            !artifact_registry_immutable_tags_enabled(br#"{}"#)
                .expect("missing immutable tag flag defaults to disabled")
        );
    }

    #[test]
    fn release_tee_providers_builds_manifest_inspect_command() {
        let command =
            docker_manifest_inspect_command("us-docker.pkg.dev/evmchain/images/raiko2-sgx:v1.2.3");

        assert_eq!(command.get_program().to_string_lossy(), "docker");
        assert_eq!(
            command_args(&command),
            vec![
                "manifest",
                "inspect",
                "us-docker.pkg.dev/evmchain/images/raiko2-sgx:v1.2.3",
            ]
        );
    }

    #[test]
    fn release_tee_providers_recognizes_missing_remote_manifests() {
        for output in [
            "manifest unknown: Failed to fetch",
            "no such manifest: us-docker.pkg.dev/example/image:v1",
            "name unknown: Repository does not exist",
            "requested entity was not found",
            "manifest for example/image:v1 not found",
        ] {
            assert_eq!(
                classify_docker_manifest_inspect_failure(output),
                DockerManifestInspectFailure::Missing,
                "should classify missing manifest: {output}"
            );
        }
    }

    #[test]
    fn release_tee_providers_does_not_treat_registry_errors_as_missing() {
        for output in [
            "denied: Permission \"artifactregistry.dockerimages.get\" denied",
            "error getting credentials - err: exec: \"docker-credential-gcloud\": executable file not found in $PATH",
            "503 service unavailable",
            "net/http: request canceled while waiting for connection",
        ] {
            assert_eq!(
                classify_docker_manifest_inspect_failure(output),
                DockerManifestInspectFailure::Blocking,
                "should fail closed for registry error: {output}"
            );
        }
    }

    #[test]
    fn release_tee_providers_mixed_registry_error_and_missing_manifest_blocks() {
        let output = "ERROR: (gcloud.auth) Reauthentication is needed.\n\
            no such manifest: us-docker.pkg.dev/example/image:v1";

        assert_eq!(
            classify_docker_manifest_inspect_failure(output),
            DockerManifestInspectFailure::Blocking
        );
    }

    #[test]
    fn release_tee_providers_local_sgx_uses_unsuffixed_and_edmm_tags() {
        assert_eq!(local_sgx_variant_tag("v1.2.3", false), "v1.2.3");
        assert_eq!(local_sgx_variant_tag("v1.2.3", true), "v1.2.3-edmm");
    }

    #[test]
    fn release_tee_providers_accepts_longest_local_sgx_release_tag() {
        let tag = format!("v{}", "a".repeat(122));
        assert_eq!(tag.len(), 123);
        assert_eq!(local_sgx_variant_tag(&tag, true).len(), 128);

        validate_release_tag(&tag).expect("derived EDMM tag fits OCI tag limit");
    }

    #[test]
    fn release_tee_providers_rejects_release_tag_when_edmm_suffix_overflows() {
        let tag = format!("v{}", "a".repeat(123));
        assert_eq!(tag.len(), 124);
        let err = validate_release_tag(&tag).expect_err("derived EDMM tag must exceed OCI limit");

        assert!(err.to_string().contains("raiko2-sgx-edmm"));
        assert!(err.to_string().contains("129 bytes > 128"));
    }

    #[test]
    fn release_tee_providers_rejects_invalid_oci_tag_characters() {
        let err = validate_release_tag("v1.2.3/evil").expect_err("slash is not valid in OCI tags");

        assert!(err.to_string().contains("outside [A-Za-z0-9_.-]"));
    }

    #[test]
    fn release_tee_providers_rejects_reserved_edmm_suffix() {
        let err =
            validate_release_tag("v1.2.3-edmm").expect_err("EDMM suffix namespace is reserved");

        assert!(err.to_string().contains("reserved local SGX EDMM suffix"));
    }

    #[test]
    fn release_tee_providers_local_sgx_build_commands_share_key_and_select_edmm() {
        let temp = tempfile::tempdir().expect("temp build paths");
        let context = temp.path().join("context");
        let dockerfile = context.join("Dockerfile.sgx");
        let enclave_key = temp.path().join("enclave-key.pem");
        let key_sha256 = "shared-key-sha256";

        let commands = LOCAL_SGX_VARIANTS.map(|variant| {
            let tag = local_sgx_variant_tag("v1.2.3", variant.edmm);
            local_sgx_docker_build_command(
                &context,
                &dockerfile,
                &local_provider_image_ref(&tag, "us-docker.pkg.dev/evmchain/images/raiko2-sgx"),
                &enclave_key,
                key_sha256,
                variant,
            )
        });
        let non_edmm_args = command_args(&commands[0]);
        let edmm_args = command_args(&commands[1]);
        let expected_secret = format!("id=gramine_enclave_key,src={}", enclave_key.display());
        let expected_key_hash = format!("GRAMINE_ENCLAVE_KEY_SHA256={key_sha256}");

        assert!(non_edmm_args.contains(&"SGX_EDMM_ENABLE=false".to_string()));
        assert!(edmm_args.contains(&"SGX_EDMM_ENABLE=true".to_string()));
        for args in [&non_edmm_args, &edmm_args] {
            assert!(args.contains(&expected_key_hash));
            assert!(args.contains(&expected_secret));
        }
    }

    #[test]
    fn release_tee_providers_local_sgx_manifest_entries_are_ordered_and_explicit() {
        let entries = valid_local_sgx_entries();

        assert_eq!(entries[0].provider, "raiko2-sgx");
        assert_eq!(entries[1].provider, "raiko2-sgx-edmm");
        assert_eq!(entries[0].lane, "sgx");
        assert_eq!(entries[1].lane, "sgx");
        assert_eq!(entries[0].image.tag, "v1.2.3");
        assert_eq!(entries[1].image.tag, "v1.2.3-edmm");
        assert_eq!(entries[0].image.sgx_edmm, Some(false));
        assert_eq!(entries[1].image.sgx_edmm, Some(true));
    }

    #[test]
    fn release_tee_providers_local_sgx_invariants_accept_valid_pair() {
        validate_local_sgx_entries(&valid_local_sgx_entries(), false)
            .expect("valid local SGX variants");
    }

    #[test]
    fn release_tee_providers_local_sgx_invariants_require_exactly_two_entries() {
        let entries = valid_local_sgx_entries();
        let err = validate_local_sgx_entries(&entries[..1], false)
            .expect_err("missing variant must fail");

        assert!(err.to_string().contains("expected exactly two entries"));
    }

    #[test]
    fn release_tee_providers_local_sgx_invariants_require_distinct_tags() {
        let mut entries = valid_local_sgx_entries();
        entries[1].image.tag = entries[0].image.tag.clone();
        let err = validate_local_sgx_entries(&entries, false)
            .expect_err("matching variant tags must fail");

        assert!(err.to_string().contains("image tags must differ"));
    }

    #[test]
    fn release_tee_providers_local_sgx_invariants_require_distinct_mrenclave() {
        let mut entries = valid_local_sgx_entries();
        entries[1].attestation.mr_enclave = entries[0].attestation.mr_enclave.clone();
        let err = validate_local_sgx_entries(&entries, false)
            .expect_err("matching MRENCLAVE values must fail");

        assert!(err.to_string().contains("MRENCLAVE values must differ"));
    }

    #[test]
    fn release_tee_providers_local_sgx_invariants_require_matching_mrsigner() {
        let mut entries = valid_local_sgx_entries();
        entries[1].attestation.mr_signer = "different-mr-signer".to_string();
        let err = validate_local_sgx_entries(&entries, false)
            .expect_err("different MRSIGNER values must fail");

        assert!(err.to_string().contains("MRSIGNER values must match"));
    }

    #[test]
    fn release_tee_providers_local_sgx_invariants_require_distinct_pushed_digests() {
        let mut entries = valid_local_sgx_entries();
        entries[1].image.digest = entries[0].image.digest.clone();
        let err = validate_local_sgx_entries(&entries, false)
            .expect_err("matching pushed digests must fail");

        assert!(err.to_string().contains("pushed image digests must differ"));
    }

    #[test]
    fn release_tee_providers_local_sgx_invariants_skip_digest_check_without_push() {
        let mut entries = valid_local_sgx_entries();
        entries[1].image.digest = entries[0].image.digest.clone();

        validate_local_sgx_entries(&entries, true).expect("no-push values are image tag refs");
    }

    #[test]
    fn release_tee_providers_renders_edmm_build_argument() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask crate has a repository parent");
        let dockerfile = std::fs::read_to_string(repo_root.join("Dockerfile.sgx"))
            .expect("read repository SGX Dockerfile");
        let manifest =
            std::fs::read_to_string(repo_root.join("docker/raiko2-sgx-prover.manifest.template"))
                .expect("read repository SGX manifest template");

        assert!(dockerfile.contains("ARG SGX_EDMM_ENABLE=false"));
        assert!(
            dockerfile.contains(r#"-Dedmm_enable="${SGX_EDMM_ENABLE}""#),
            "Dockerfile must pass SGX_EDMM_ENABLE to gramine-manifest"
        );
        assert!(manifest.contains("sgx.edmm_enable = {{ edmm_enable }}"));
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
