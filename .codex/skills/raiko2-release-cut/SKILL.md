---
name: raiko2-release-cut
description: Use when cutting a versioned raiko2 source release from this repository, including git tag creation, publishing the runtime image, exporting guest digests, and creating a GitHub Release with release notes and manifest artifacts. Use when the workflow must stop before deployment or register-image apply.
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
- collect image digests and guest digests
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

The release manifest must include:

- version
- tag
- git SHA
- runtime image digest reference
- exported guest digests

## Guardrails

- Start from a clean checkout of `main` or an explicit release commit.
- Publish one runtime image that includes both guest backends.
- Record immutable digest references, not just mutable image tags.
- Use the manifest helper in `scripts/release/write_release_manifest.py`.
- Stop before rollout or `register-image --apply` unless the user explicitly asks for that as a
  separate task.

## Reporting

The user does not see raw command output. Always summarize:

- release tag
- release commit SHA
- runtime image tag and digest
- manifest location
- whether the GitHub Release was created successfully
