use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

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

/// Result of asking the registry whether an image tag exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteTagLookup {
    /// Every registry host answered 404 for the tag (or for the whole package).
    Absent,
    Exists,
    /// The lookup failed, so absence is not proven. Releases must stop.
    Inconclusive,
}

/// Text that means the lookup itself failed (auth, rate limit, server or network errors). Matched
/// after the image ref has been removed from the output, so a tag such as `release-login-fix`
/// cannot trip it. Checked before the not-found line: output that mentions both fails closed.
const REMOTE_TAG_LOOKUP_FAILURE_MARKERS: &[&str] = &[
    "authenticat",
    "authoriz",
    "credential",
    "denied",
    "forbidden",
    "permission",
    "login",
    "unauthorized",
    "unexpected status",
    "service unavailable",
    "too many requests",
    "timeout",
    "timed out",
    "deadline",
    "request canceled",
    "connection",
    "temporary failure",
    "network is unreachable",
    "no route to host",
    "dial tcp",
    "tls handshake",
    "certificate",
    "unexpected eof",
    "internal server error",
    "bad gateway",
    "gateway timeout",
    "429",
    "500",
    "502",
    "503",
    "504",
];

/// `docker buildx imagetools inspect <ref> --format '{{json .Manifest}}'`.
///
/// Unlike `docker manifest inspect`, which reports a manifest-endpoint 401 or an unparseable 4xx as
/// a plain `no such manifest`, imagetools goes through containerd's resolver: it prints
/// `<ref>: not found` only when every registry host answered 404, and reports any other status
/// (`unexpected status from HEAD request to …: 403 Forbidden`, authorization failures, …) as such.
pub(crate) fn remote_tag_inspect_command(image_ref: &str) -> Command {
    let mut cmd = Command::new("docker");
    cmd.arg("buildx")
        .arg("imagetools")
        .arg("inspect")
        .arg(image_ref)
        .arg("--format")
        .arg("{{json .Manifest}}");
    cmd
}

/// Classify the output of [`remote_tag_inspect_command`] for `image_ref`.
pub(crate) fn classify_remote_tag_lookup(
    image_ref: &str,
    lookup_succeeded: bool,
    output: &str,
) -> RemoteTagLookup {
    if lookup_succeeded {
        return RemoteTagLookup::Exists;
    }
    let without_ref = output.replace(image_ref, "").to_ascii_lowercase();
    if REMOTE_TAG_LOOKUP_FAILURE_MARKERS
        .iter()
        .any(|marker| without_ref.contains(marker))
    {
        return RemoteTagLookup::Inconclusive;
    }
    let not_found = format!("{image_ref}: not found");
    let reports_not_found = output.lines().any(|line| {
        let line = line.trim();
        line.strip_prefix("ERROR:").map_or(line, str::trim_start) == not_found
    });
    if reports_not_found {
        RemoteTagLookup::Absent
    } else {
        RemoteTagLookup::Inconclusive
    }
}

/// Fail closed unless the registry conclusively reports that `image_ref` does not exist yet.
/// Releases never overwrite a published tag.
pub(crate) fn ensure_remote_tag_absent(image_ref: &str) -> Result<()> {
    let output = remote_tag_inspect_command(image_ref)
        .output()
        .with_context(|| format!("failed to look up image tag {image_ref}"))?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    match classify_remote_tag_lookup(image_ref, output.status.success(), &combined) {
        RemoteTagLookup::Absent => Ok(()),
        RemoteTagLookup::Exists => bail!(
            "image tag {image_ref} already exists; releases never overwrite a published tag, use a new tag"
        ),
        RemoteTagLookup::Inconclusive => bail!(
            "could not confirm that image tag {image_ref} is absent; refusing to publish:\n{}",
            combined.trim_end()
        ),
    }
}

/// Resolve the `<repository>@sha256:…` digest docker recorded for a pushed local image.
pub(crate) fn resolve_repo_digest(image_ref: &str, repository: &str) -> Result<String> {
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

/// Lookups of a freshly pushed tag before giving up. Only lookup errors are retried; a digest
/// mismatch fails at once.
const REMOTE_DIGEST_VERIFY_ATTEMPTS: u32 = 5;
const REMOTE_DIGEST_VERIFY_BACKOFF: Duration = Duration::from_secs(5);

/// Verify after a push that the remote tag resolves to the digest that was just pushed.
pub(crate) fn ensure_remote_tag_digest_matches_expected(
    image_ref: &str,
    expected_digest: &str,
) -> Result<()> {
    let repository = expected_digest
        .split_once("@sha256:")
        .map(|(repository, _)| repository)
        .ok_or_else(|| anyhow!("expected pushed digest is not a sha256 repository digest"))?;
    verify_remote_tag_digest(
        image_ref,
        expected_digest,
        REMOTE_DIGEST_VERIFY_ATTEMPTS,
        || resolve_remote_tag_digest(image_ref, repository),
        |attempt| thread::sleep(REMOTE_DIGEST_VERIFY_BACKOFF * attempt),
    )
}

fn verify_remote_tag_digest(
    image_ref: &str,
    expected_digest: &str,
    attempts: u32,
    mut lookup: impl FnMut() -> Result<String>,
    mut backoff: impl FnMut(u32),
) -> Result<()> {
    let mut last_error = None;
    for attempt in 1..=attempts {
        match lookup() {
            Ok(actual_digest) if actual_digest == expected_digest => return Ok(()),
            Ok(actual_digest) => bail!(
                "remote image tag digest mismatch for {image_ref}: expected {expected_digest}, got {actual_digest}",
            ),
            Err(err) => {
                eprintln!(
                    "[WARN] remote digest lookup {attempt}/{attempts} for {image_ref} failed: {err:#}"
                );
                last_error = Some(err);
                if attempt < attempts {
                    backoff(attempt);
                }
            }
        }
    }
    let last_error = last_error.map_or_else(
        || "no lookup attempted".to_string(),
        |err| format!("{err:#}"),
    );
    bail!(
        "image tag {image_ref} WAS pushed (local digest {expected_digest}), but its remote digest \
         could not be verified after {attempts} attempts: {last_error}\n\
         Do not reuse this tag. Verify it with `docker buildx imagetools inspect {image_ref}`."
    )
}

fn resolve_remote_tag_digest(image_ref: &str, repository: &str) -> Result<String> {
    let output = remote_tag_inspect_command(image_ref)
        .output()
        .with_context(|| format!("failed to inspect remote image tag digest {image_ref}"))?;
    if !output.status.success() {
        let output_text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        bail!(
            "docker buildx imagetools inspect failed for {image_ref}\n{}",
            output_text.trim_end(),
        );
    }

    let manifest_digest = parse_remote_tag_manifest_digest(
        std::str::from_utf8(&output.stdout).context("remote image manifest is not utf-8")?,
    )?;
    Ok(format!("{repository}@{manifest_digest}"))
}

fn parse_remote_tag_manifest_digest(output: &str) -> Result<String> {
    let manifest: serde_json::Value =
        serde_json::from_str(output).context("failed to parse remote image manifest")?;
    let digest = manifest
        .get("digest")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing remote manifest digest"))?;
    if digest
        .strip_prefix("sha256:")
        .is_none_or(|value| value.is_empty())
    {
        bail!("remote manifest digest must be sha256: {digest}");
    }
    Ok(digest.to_string())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use anyhow::anyhow;

    use super::{
        RemoteTagLookup, classify_remote_tag_lookup, parse_remote_tag_manifest_digest,
        remote_tag_inspect_command, verify_remote_tag_digest,
    };

    const IMAGE_REF: &str = "us-docker.pkg.dev/evmchain/images/raiko2-cuda:release-1";

    fn classify(image_ref: &str, output: &str) -> RemoteTagLookup {
        classify_remote_tag_lookup(image_ref, false, output)
    }

    #[test]
    fn remote_tag_inspect_uses_status_preserving_imagetools() {
        let command = remote_tag_inspect_command(IMAGE_REF);
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        assert_eq!(command.get_program().to_string_lossy(), "docker");
        assert_eq!(
            args,
            vec![
                "buildx",
                "imagetools",
                "inspect",
                IMAGE_REF,
                "--format",
                "{{json .Manifest}}",
            ]
        );
    }

    #[test]
    fn remote_tag_lookup_success_means_tag_exists() {
        assert_eq!(
            classify_remote_tag_lookup(IMAGE_REF, true, "{\"digest\":\"sha256:abc\"}"),
            RemoteTagLookup::Exists
        );
    }

    #[test]
    fn remote_tag_lookup_accepts_only_the_resolver_not_found_line() {
        // Exact output of `docker buildx imagetools inspect` (buildx v0.19) for a missing tag and
        // for a package that was never pushed.
        for (image_ref, output) in [
            (IMAGE_REF, format!("ERROR: {IMAGE_REF}: not found\n")),
            (
                "us-docker.pkg.dev/evmchain/images/no-such-package:v1",
                "ERROR: us-docker.pkg.dev/evmchain/images/no-such-package:v1: not found\n"
                    .to_string(),
            ),
        ] {
            assert_eq!(
                classify(image_ref, &output),
                RemoteTagLookup::Absent,
                "{output}"
            );
        }
    }

    #[test]
    fn remote_tag_lookup_ignores_marker_words_inside_the_image_ref() {
        for image_ref in [
            "us-docker.pkg.dev/evmchain/images/raiko2-cuda:release-login-fix",
            "us-docker.pkg.dev/evmchain/images/raiko2:fix-timeout-permission-denied",
            "us-docker.pkg.dev/evmchain/images/connection-pool:v503",
        ] {
            let output = format!("ERROR: {image_ref}: not found\n");
            assert_eq!(
                classify(image_ref, &output),
                RemoteTagLookup::Absent,
                "{output}"
            );
        }
    }

    #[test]
    fn remote_tag_lookup_fails_closed_on_lookup_errors() {
        for output in [
            // Outputs observed from imagetools against registries returning these statuses.
            "ERROR: pull access denied, repository does not exist or may require authorization: authorization failed: no basic auth credentials",
            "ERROR: unexpected status from HEAD request to https://us-docker.pkg.dev/v2/evmchain/images/raiko2-cuda/manifests/release-1: 403 Forbidden",
            "ERROR: unexpected status from HEAD request to https://us-docker.pkg.dev/v2/evmchain/images/raiko2-cuda/manifests/release-1: 429 Too Many Requests",
            "ERROR: unexpected status from HEAD request to https://us-docker.pkg.dev/v2/evmchain/images/raiko2-cuda/manifests/release-1: 503 Service Unavailable",
            "ERROR: failed to do request: Head \"https://us-docker.pkg.dev/v2/\": dial tcp: lookup us-docker.pkg.dev: no such host",
            "ERROR: failed to do request: net/http: TLS handshake timeout",
            "error getting credentials - err: exec: \"docker-credential-gcloud\": executable file not found in $PATH",
            "",
            "something nobody anticipated",
            // `docker manifest inspect` wording is not accepted: it hides 401s behind it.
            "no such manifest: us-docker.pkg.dev/evmchain/images/raiko2-cuda:release-1",
            "manifest unknown: Failed to fetch \"release-1\"",
            // A different ref reported as not found is not proof for this one.
            "ERROR: us-docker.pkg.dev/evmchain/images/raiko2-cuda:release-10: not found",
        ] {
            assert_eq!(
                classify(IMAGE_REF, output),
                RemoteTagLookup::Inconclusive,
                "{output}"
            );
        }
    }

    #[test]
    fn remote_tag_lookup_credential_error_with_not_found_fails_closed() {
        // A failing credential helper followed by an anonymous lookup must not count as absent.
        let output = format!(
            "ERROR: (gcloud.auth) Reauthentication is needed.\nERROR: {IMAGE_REF}: not found\n"
        );

        assert_eq!(classify(IMAGE_REF, &output), RemoteTagLookup::Inconclusive);
    }

    #[test]
    fn parse_remote_tag_manifest_digest_reads_sha256_digest() {
        assert_eq!(
            parse_remote_tag_manifest_digest(r#"{"digest":"sha256:abc123","mediaType":"x"}"#)
                .expect("digest should parse"),
            "sha256:abc123"
        );
    }

    #[test]
    fn parse_remote_tag_manifest_digest_rejects_missing_or_empty_digest() {
        for output in [r#"{"mediaType":"x"}"#, r#"{"digest":"sha256:"}"#] {
            assert!(
                parse_remote_tag_manifest_digest(output).is_err(),
                "{output}"
            );
        }
    }

    const PUSHED_DIGEST: &str = "us-docker.pkg.dev/evmchain/images/raiko2-cuda@sha256:aaaa";

    #[test]
    fn remote_digest_verification_retries_lookup_errors() {
        let calls = Cell::new(0);
        let backoffs = Cell::new(0);
        let result = verify_remote_tag_digest(
            IMAGE_REF,
            PUSHED_DIGEST,
            5,
            || {
                calls.set(calls.get() + 1);
                if calls.get() < 3 {
                    Err(anyhow!("503 Service Unavailable"))
                } else {
                    Ok(PUSHED_DIGEST.to_string())
                }
            },
            |_| backoffs.set(backoffs.get() + 1),
        );

        assert!(result.is_ok(), "{result:?}");
        assert_eq!(calls.get(), 3);
        assert_eq!(backoffs.get(), 2);
    }

    #[test]
    fn remote_digest_verification_never_retries_a_mismatch() {
        let calls = Cell::new(0);
        let err = verify_remote_tag_digest(
            IMAGE_REF,
            PUSHED_DIGEST,
            5,
            || {
                calls.set(calls.get() + 1);
                Ok("us-docker.pkg.dev/evmchain/images/raiko2-cuda@sha256:bbbb".to_string())
            },
            |_| panic!("a digest mismatch must not be retried"),
        )
        .expect_err("a digest mismatch must fail");

        assert_eq!(calls.get(), 1);
        assert!(err.to_string().contains("digest mismatch"), "{err}");
    }

    #[test]
    fn remote_digest_verification_reports_the_pushed_tag_when_lookups_keep_failing() {
        let calls = Cell::new(0);
        let err = verify_remote_tag_digest(
            IMAGE_REF,
            PUSHED_DIGEST,
            3,
            || {
                calls.set(calls.get() + 1);
                Err(anyhow!("dial tcp: i/o timeout"))
            },
            |_| {},
        )
        .expect_err("persistent lookup errors must fail");
        let message = err.to_string();

        assert_eq!(calls.get(), 3);
        assert!(
            message.contains(&format!("{IMAGE_REF} WAS pushed")),
            "{message}"
        );
        assert!(message.contains(PUSHED_DIGEST), "{message}");
        assert!(message.contains("Do not reuse this tag"), "{message}");
        assert!(message.contains("dial tcp: i/o timeout"), "{message}");
    }
}
