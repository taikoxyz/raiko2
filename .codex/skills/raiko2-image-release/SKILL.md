---
name: raiko2-image-release
description: Use when building and publishing a raiko2 runtime image from this repository with just release-image or xtask release-image.
---

# Raiko2 Image Release

## Scope

Use this skill for image-only publishing:

- `just release-image <backend> <tag>`
- `cargo run -r -p xtask -- release-image <backend> --tag <tag> --repository <repo>`

Do not add deployment, rollout, namespace, or `kubectl` commands.

## Preconditions

- repo root is `raiko2`
- worktree is clean before release starts
- backend is `risc0`, `sp1`, or `all`
- tag is non-empty

Default repository:

- `us-docker.pkg.dev/evmchain/images/raiko2`

## Flow

1. Check worktree state: `git status --short`
2. Run `just release-image <backend> <tag>`
3. If needed, use direct `xtask`: `cargo run -r -p xtask -- release-image <backend> --tag <tag> --repository us-docker.pkg.dev/evmchain/images/raiko2`
4. Capture `[INFO] Image pushed: <repository>@sha256:...`
5. Report the exact digest.

## Guest ELFs

For unreleased testing:

```bash
just build-guest all
```

Download released guest assets:

```bash
cargo run -r -p xtask -- download-guest-elves --tag <tag> --backend all --dir crates/guests/elf
```

Do not bypass `release-image` with ad-hoc `docker build`.
