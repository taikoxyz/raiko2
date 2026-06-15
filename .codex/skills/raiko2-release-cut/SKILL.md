---
name: raiko2-release-cut
description: Use when cutting a versioned raiko2 source release from this repository, including git tag creation, publishing runtime and TEE provider images, exporting guest digests and SGX attestation metadata, and creating a GitHub Release with release notes and manifest artifacts. Use when the workflow must stop before deployment or register-image apply.
---

# Raiko2 Release Cut

## Overview

Use this skill for source releases such as `v0.1.0`.

The public source of truth is:

- `docs/operations.md` → `Source Releases`

Follow that runbook instead of reconstructing an ad-hoc release process.

## When To Use

Use this skill when the user asks to:

- cut a release tag from `main`
- publish a versioned runtime image that includes both `risc0` and `sp1` guest ELFs
- publish versioned TEE provider images for `raiko2-sgx` and pinned external providers
- collect image digests and guest digests
- collect SGX provider attestation metadata
- create release notes and a GitHub Release

Do not use this skill for:

- image-only publishing without a source release
- deployment, rollout, or environment changes
- automatic `register-image --apply`

For image-only publication, use `$raiko2-image-release`.

## Required Release Outputs

Every release cut must produce:

- git tag: `vX.Y.Z`
- image tag: `vX.Y.Z`
- release notes markdown
- release manifest JSON
- guest digests summary JSON
- TEE attestation manifest JSON

The release manifest must include:

- version
- tag
- git_sha
- runtime image digest reference
- exported guest digests

The release notes must include the human-readable ZK guest digests:

- `risc0` proposal and aggregation `image_id`
- `sp1` proposal and aggregation `vk_bn254`
- `sp1` proposal and aggregation `vk_hash_bytes`

The release notes must also include the human-readable SGX provider data:

- `raiko2-sgx` pushed image digest
- `raiko2-sgx` `mr_enclave` and `mr_signer`
- each pinned external TEE provider pushed image digest
- each pinned external TEE provider source commit, `mr_enclave`, and `mr_signer`

Upload `tee-attestation-manifest-vX.Y.Z.json` to the GitHub Release together with the runtime
release manifest and guest digest summary.

## Required Order

Do not create the GitHub Release before both release paths complete:

1. Run the source runtime release flow from `docs/operations.md` → `Source Releases`.
2. Run the TEE provider release flow:
   `cargo run -r -p xtask -- release-tee-providers --tag vX.Y.Z`
3. Verify all three image refs exist in the registry:
   - `us-docker.pkg.dev/evmchain/images/raiko2:vX.Y.Z`
   - `us-docker.pkg.dev/evmchain/images/raiko2-sgx:vX.Y.Z`
   - every provider repository listed in `release/providers.toml` at `:vX.Y.Z`
4. Build release notes from the fresh runtime digest, guest digest summary, and
   `target/releases/vX.Y.Z/tee-attestation-manifest-vX.Y.Z.json`.

## Guardrails

- Start from a clean checkout of `main` or an explicit release commit.
- Publish one runtime image that includes both guest backends.
- Record immutable digest references, not just mutable image tags.
- Use the manifest helper in `scripts/release/write_release_manifest.py`.
- Upload `release-manifest-*.json`, `guest-digests-summary.json`, and
  `tee-attestation-manifest-*.json` to the GitHub Release.
- Do not assume `release-image` publishes SGX provider images; it only publishes the main runtime image. `release-tee-providers` is mandatory for release tags.
- Stop before rollout or `register-image --apply` unless the user explicitly asks for that as a
  separate task.

## Reporting

The user does not see raw command output. Always summarize:

- release tag
- release commit SHA
- runtime image tag and digest
- `raiko2-sgx` image digest and SGX measurements
- external TEE provider image digests, source commits, and SGX measurements
- manifest location
- guest digest asset location
- TEE attestation manifest asset location
- whether the GitHub Release was created successfully
