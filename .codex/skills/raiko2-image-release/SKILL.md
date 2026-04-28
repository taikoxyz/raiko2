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

## Guest ELF Rule

`release-image` ensures the checked-in guest ELF artifacts are current before building the runtime
image.

If that refresh leaves tracked changes in `crates/guests/elf`, `release-image` stops before
publishing. In that case:

1. review the updated tracked artifacts
2. commit them
3. rerun `release-image`

Do not bypass this by using ad-hoc `docker build`. The published image must match committed release
artifacts.

## Optional Register Check

After building an image, you may check whether verifier trust-list registration is needed:

```bash
cargo run -r -p xtask -- register-image --profile hoodi-shasta --backend <backend>
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
