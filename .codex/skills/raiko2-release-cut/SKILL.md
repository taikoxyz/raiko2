---
name: raiko2-release-cut
description: Use when cutting a versioned raiko2 ZK/runtime source release from this repository, including git tag creation, publishing the runtime image, exporting guest digests, and creating a GitHub Release with release notes and manifest artifacts. TEE provider metadata is a separate optional flow. Use when the workflow must stop before deployment or register-image --apply.
---

# Raiko2 Release Cut

## Overview

Use this skill for source releases such as `vX.Y.Z`.

Public source of truth:

- `docs/operations.md` -> `Source Releases`
- `docs/operations.md` -> `Release TEE Provider Metadata`, only when TEE output is explicitly requested

Follow the runbook instead of reconstructing an ad-hoc release process.

## When To Use

Use this skill when the user asks to:

- cut a release tag
- publish the versioned runtime image with both `risc0` and `sp1` guest ELFs
- collect runtime image digest and guest digests
- create release notes and a GitHub Release
- collect TEE provider image digests and attestation metadata, only when explicitly requested

Do not use this skill for:

- image-only publishing without a source release
- deployment, rollout, or environment changes
- automatic `register-image --apply`

For image-only publication, use `$raiko2-image-release`.

## Required ZK Runtime Outputs

Every ZK/runtime release cut must produce:

- git tag: `vX.Y.Z`
- image tag: `vX.Y.Z`
- release notes markdown
- release manifest JSON
- guest digests summary JSON

Release manifest must include:

- version
- tag
- git_sha
- runtime image digest reference
- exported guest digests

Release notes must include human-readable ZK guest digests:

- `risc0` proposal and aggregation `image_id`
- `sp1` proposal and aggregation `vk_bn254`
- `sp1` proposal and aggregation `vk_hash_bytes`

TEE provider metadata must not be included in a ZK/runtime-only release.

## Optional TEE Outputs

Only when explicitly requested, run the TEE provider metadata flow and upload:

- `tee-attestation-manifest-vX.Y.Z.json`
- `raiko2-sgx` pushed image digest
- `raiko2-sgx` `mr_enclave` and `mr_signer`
- each pinned external TEE provider pushed image digest
- each pinned external TEE provider source commit, `mr_enclave`, and `mr_signer`

## Required Order

Do not create the GitHub Release before requested release paths complete:

1. Run source runtime release flow from `docs/operations.md` -> `Source Releases`.
2. If TEE output was explicitly requested, run:
   `cargo run -r -p xtask -- release-tee-providers --tag vX.Y.Z`
   with `GCP_ENCLAVE_KEY_*` set as documented in `docs/operations.md`.
3. Verify requested image refs exist in registry:
   - `us-docker.pkg.dev/evmchain/images/raiko2:vX.Y.Z`
   - TEE only when requested: `us-docker.pkg.dev/evmchain/images/raiko2-sgx:vX.Y.Z`
   - TEE only when requested: every provider repository listed in `release/providers.toml` at `:vX.Y.Z`
4. Build release notes from fresh runtime digest and guest digest summary.

## Guardrails

- Start from a clean checkout of `main` or an explicit release commit.
- Publish one runtime image that includes both guest backends.
- Record immutable digest references, not just mutable image tags.
- Use `scripts/release/write_release_manifest.py` for the ZK/runtime release manifest.
- Upload `release-manifest-*.json` and `guest-digests-summary.json` to the GitHub Release.
- Upload `tee-attestation-manifest-*.json` only for explicit TEE provider metadata releases.
- Do not assume `release-image` publishes SGX provider images; it only publishes the main runtime image.
- Stop before rollout or `register-image --apply` unless the user explicitly asks for a separate task.

## Reporting

The user does not need raw command output. Always summarize:

- release tag
- release commit SHA
- runtime image tag and digest
- release manifest asset location
- guest digest asset location
- TEE attestation manifest asset location, only when TEE output was requested
- whether the GitHub Release was created successfully
