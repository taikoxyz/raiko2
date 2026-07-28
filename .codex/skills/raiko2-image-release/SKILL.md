---
name: raiko2-image-release
description: Use when building or publishing a raiko2 runtime image, including a host-only source revision paired with guest ELF/VK artifacts from an existing release, without deployment or rollout work.
---

# Raiko2 Image Release

## Overview

Use this skill for image publishing only. This repository does not own Tolba or GKE rollout.

Stay on the canonical release path:

- `just release-image <backend> <tag>`
- direct fallback: `cargo run -r -p xtask -- release-image <backend> --tag <tag> --repository <repo>`

Do not add `kubectl`, deployment, namespace, or rollout commands.

## When To Use

Use this skill when the user asks to:

- build a release image from the current `raiko2` source tree
- push a runtime image to the registry
- capture the immutable image digest for downstream deployment
- check whether guest registration is needed after a new image build

Do not use this skill for:

- Tolba deployment
- Kubernetes rollout
- smoke checks against a live service
- infra changes outside image build and guest registration
- TEE provider attestation export across multiple provider images

## Preconditions

Before running `release-image`:

- confirm you are in the `raiko2` repo root
- keep the worktree clean before the release starts
- choose a concrete backend: `risc0`, `sp1`, or `all`
- choose a non-empty tag

Default repository:

- `us-docker.pkg.dev/evmchain/images/raiko2`

## Canonical Flow

1. Check worktree state:
   `git status --short`
2. Run the preferred wrapper:
   `just release-image <backend> <tag>`
3. If `just` is not appropriate, use direct `xtask`:
   `cargo run -r -p xtask -- release-image <backend> --tag <tag> --repository us-docker.pkg.dev/evmchain/images/raiko2`
4. Capture the pushed digest from the output line:
   `[INFO] Image pushed: <repository>@sha256:...`
5. Report the exact digest back to the user.

For TEE-backed provider image attestation capture, use the dedicated `xtask` flow instead:

- `cargo run -r -p xtask -- release-tee-providers --tag <tag>`

Pushed TEE provider release runs require target Artifact Registry Docker repositories with immutable
tags enabled. Use `--no-push` for local smoke/reproduction checks that must not publish registry
tags.

That flow owns:

- external provider source pin resolution
- multi-provider TEE image build/push
- `mr_enclave` / `mr_signer` export
- unified `tee-attestation-manifest-<tag>.json`

## Guest ELF Rule

`release-image` refreshes guest ELF artifacts by default for non-host runtime images. Host image
publication and guest ELF publication are decoupled. For unreleased testing, build local ELFs with `just build-guest all` or
`cargo run -r -p xtask -- build-guest all`. Published guest ELFs are GitHub Release assets and can be
downloaded separately:

```bash
cargo run -r -p xtask -- download-guest-elves --tag <tag> --backend all --dir crates/guests/elf
```

Use `--skip-guest-refresh` only when intentionally composing an image from already committed guest
ELFs. Use `--refresh-guest-elves <risc0|sp1|all>` to override the default guest set. If a refresh leaves tracked
changes in `crates/guests/elf`, `release-image` stops before publishing; review and commit those
artifacts before retrying.

Do not bypass this by using ad-hoc `docker build`. The published image must match the selected
source revision and the guest ELF assets expected by the deployment.

## Host Image With Released Guest Artifacts

Use `docs/operations.md` -> `Compose A Host Image With Released Guest Artifacts` when a new host
revision must keep the RISC0 and SP1 programs from an existing release.

Required sequence:

1. Resolve the exact host commit and guest release commit.
2. Create a named composition branch and clean worktree from the host commit. Use a traceable name
   such as `main-<host-short-sha>-elf-<release-tag>`.
3. Restore the complete release-tag `crates/guests/elf` directory. Keep ELF, SP1 VK, lab artifacts,
   and both provenance manifests together; do not assemble a partial directory from downloads.
4. Verify the directory against the release tag, exact expected GitHub Release Shasta asset set, and
   every published asset byte.
5. Before checking source closure, validate each backend's provenance manifest and exact artifact
   inventory, every recorded artifact hash, and the Shasta SP1 ELF/VK pairs. A source mismatch must
   not mask an artifact, manifest, or SP1 failure.
6. Run the source-closure check. A passing source fingerprint permits the automatic lane. A source
   fingerprint mismatch does not prove incompatibility, but it stops the automatic lane until a
   reviewed guest-facing diff, proposal regression, aggregation regression, and soundness
   assessment explicitly approve the pairing. The exception is source-only and is available only
   after the artifact-only checks pass for both backends.
7. Commit only the artifact composition, record both source commits in the commit message, push the
   composition branch, and require a clean worktree.
8. Reconfirm the selected moving host ref still resolves to the frozen host commit. Require
   registry-side immutable tags, then fail closed unless the selected image tag is conclusively
   absent; authentication, network, or registry errors are not absence.
9. Run `release-image host ... --skip-guest-refresh`, capture the immutable digest, prove the
   registry tag resolves to that digest, then pull the digest and verify its OCI revision label and
   packaged artifacts against the composition commit.

Artifact hashes and SP1 ELF/VK consistency prove artifact identity, not host/guest protocol
compatibility or guest soundness. The old guest supplies the proof constraints, so host-side
hardening added after that release cannot replace a missing guest-side check.

Stop instead of publishing when:

- the release tag, GitHub Release, or artifact set cannot be resolved exactly;
- artifacts come from different releases or provenance is omitted/regenerated against old binaries;
- either backend fails artifact-only provenance, inventory, or digest validation;
- the composition changes files outside `crates/guests/elf`;
- guest-facing source drift lacks the explicit compatibility and soundness approval above;
- the worktree is dirty, the source branch is not reachable, registry-side tag immutability is not
  confirmed, or tag absence is inconclusive;
- post-push tag-to-digest, revision, or artifact verification fails.

Never hide artifact changes with Git index flags, create only a detached unreferenced composition
commit, or treat `--skip-guest-refresh` as a compatibility override.

## Optional Register Check

After building an image, you may check whether verifier trust-list registration is needed:

```bash
cargo run -r -p xtask -- register-image --profile <hoodi-shasta|mainnet-shasta> --backend <backend>
```

Default behavior is check-only. Do not broadcast registration transactions unless the user
explicitly asks for `--apply`.

## Output Handling

The user does not see raw command output. Always summarize:

- backend used
- image tag
- pushed digest
- whether guest refresh required a commit
- whether register-image check found pending registrations

## Boundaries

- Do not mention or reconstruct removed `release-image` flags such as `--namespace`,
  `--deployment`, or `--container`.
- Do not include Tolba rollout commands.
- Do not suggest `kubectl set image`.
- Do not claim the image is deployed anywhere. This skill ends at image publication and optional
  registration evaluation.
