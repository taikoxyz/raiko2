---
name: raiko2-image-release
description: Use when building and publishing a raiko2 runtime image from this repository with just release-image or xtask release-image. Use when Codex must keep the workflow limited to guest ELF refresh, image build/push, digest capture, and optional register-image checks, without any kubectl, GKE, or rollout steps.
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
