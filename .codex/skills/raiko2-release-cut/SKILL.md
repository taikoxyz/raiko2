---
name: raiko2-release-cut
description: Use when cutting a versioned raiko2 release from this repository, including the full ZK/runtime and TEE provider outputs, git tag creation, release notes, manifest assets, and GitHub Release creation. The current default profile is full; narrower profiles are documented for future configuration. Use when the workflow must stop before deployment or register-image --apply.
---

# Raiko2 Release Cut

## Overview

Use this skill for versioned releases such as `vX.Y.Z`. The current default release profile is
`full`: publish the ZK/runtime image and guest digest assets, then publish both local SGX variants
and every pinned external TEE provider from `release/providers.toml`.

Public source of truth:

- `docs/operations.md` -> `Source Releases`
- `docs/operations.md` -> `Release TEE Provider Metadata`

Follow the runbook instead of reconstructing an ad-hoc release process.

## When To Use

Use this skill when the user asks to:

- cut a release tag
- publish the versioned runtime image with both `risc0` and `sp1` guest ELFs
- collect runtime image digest and guest digests
- create release notes and a GitHub Release
- collect TEE provider image digests and attestation metadata for the default full profile

Do not use this skill for:

- image-only publishing without a source release
- deployment, rollout, or environment changes
- automatic `register-image --apply`

For image-only publication, use `$raiko2-image-release`.

## Release Profiles

The current default is `full` and must include all outputs in this document. A future implementation
may add explicit `zk-only` and `tee-only` configuration, but no such CLI switch exists yet. Until
then, an agent must treat an ordinary release-cut request as `full`; if the user explicitly asks for
only one lane, confirm that narrowed scope before omitting the other lane.

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

For the current `full` profile, the release notes also include the TEE provider section below. A
deliberately narrowed `zk-only` release must say so in its notes and must not claim to contain TEE
provider metadata.

## Required TEE Outputs For Full Profile

For the default `full` profile, run the TEE provider metadata flow and upload:

- `tee-attestation-manifest-vX.Y.Z.json`
- `raiko2-sgx` non-EDMM pushed image digest at `:vX.Y.Z`
- `raiko2-sgx-edmm` EDMM pushed image digest at `:vX.Y.Z-edmm`
- `raiko2-sgx` `mr_enclave` and `mr_signer`
- `raiko2-sgx-edmm` `mr_enclave` and `mr_signer`
- each pinned external TEE provider pushed image digest
- each pinned external TEE provider source commit, `mr_enclave`, and `mr_signer`

## Required Order

Do not create the GitHub Release before requested release paths complete:

1. Run the source runtime build, guest digest export, and release manifest steps from
   `docs/operations.md` -> `Source Releases`.
2. For the default `full` profile, before writing release notes or creating the GitHub Release, run
   the `Release - TEE provider images` GitHub Actions workflow, or an equivalent controlled
   `cargo run -r -p xtask --no-default-features --features tee-provider-release -- release-tee-providers --tag vX.Y.Z`
   operation with `GCP_ENCLAVE_KEY_*` set as documented in `docs/operations.md`. Publishing must
   fail closed unless destination tags are absent and post-push remote tag digest reconciliation
   succeeds; use `--no-push` only for local reproduction/smoke checks.
3. Verify requested image refs exist in registry:
   - `us-docker.pkg.dev/evmchain/images/raiko2:vX.Y.Z`
   - `us-docker.pkg.dev/evmchain/images/raiko2-sgx:vX.Y.Z`
   - `us-docker.pkg.dev/evmchain/images/raiko2-sgx:vX.Y.Z-edmm`
   - every external provider repository listed in `release/providers.toml` at `:vX.Y.Z`
4. Build one release note from the fresh runtime digest, guest digest summary, and TEE
   attestation manifest. Include both reproduce sections from `docs/operations.md`.

## Artifact Reproducibility Checks

Do not compare generated release JSON files byte-for-byte. `guest-digests-summary.json` includes
`created_at_unix`, and `tee-attestation-manifest-*.json` includes `generated_at`.

- ZK: `guest-digests` hashes the current ELF/VK artifacts; it does not rebuild guests from source.
  To prove release digests from source, first run `just build-guest all --force`, then run
  `guest-digests`. Compare a sorted `.digests` projection, not the whole JSON file.
- TEE: `release-tee-providers --no-push` performs a full local rebuild without registry publication;
  it still builds local SGX images, external provider images, and local output state, and may emit
  mutable tag refs. With the official signing key, compare a sorted `{lane, provider, source,
  attestation}` projection. With `RAIKO2_SGX_ENCLAVE_KEY_HOST` and a disposable local key, compare
  the same projection after deleting `attestation.mr_signer` from both manifests.
- Registry: verify published immutable image digests separately with `docker buildx imagetools
  inspect` or equivalent for every released runtime and TEE image tag, then record `@sha256:...`
  refs.

Use `docs/operations.md` -> `Reproduce ZK Guest Digests` and `Release TEE Provider Metadata` for
the exact commands.

## Guardrails

- Start from a clean checkout of `main` or an explicit release commit.
- Publish one runtime image that includes both guest backends.
- Record immutable digest references, not just mutable image tags.
- Use `scripts/release/write_release_manifest.py` for the ZK/runtime release manifest.
- Upload `release-manifest-*.json` and `guest-digests-summary.json` to the GitHub Release.
- Upload `tee-attestation-manifest-*.json` for the default `full` profile.
- Do not assume `release-image` publishes SGX provider images; it only publishes the main runtime image.
- Stop before rollout or `register-image --apply` unless the user explicitly asks for a separate task.

## Reporting

The user does not need raw command output. Always summarize:

- release tag
- release commit SHA
- runtime image tag and digest
- release manifest asset location
- guest digest asset location
- TEE attestation manifest asset location for the `full` profile
- the ZK and TEE reproduce commands or documentation anchors used in the release notes
- whether the GitHub Release was created successfully
