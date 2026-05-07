# Release Runbook And Skill Design

## Goal

Define a repeatable `raiko2` release process that humans and agents can both follow for source
releases. The process must cover:

- tagging a release from a clean `main` commit
- publishing both runtime image backends (`risc0` and `sp1`)
- recording immutable image digests
- recording guest digests / image IDs
- creating a GitHub Release with both human-readable notes and machine-readable metadata

## Scope

This design introduces release documentation and an agent skill. It does not automate the full
release cut inside GitHub Actions yet, and it does not broadcast `register-image --apply`.

## Recommended Shape

Use a public runbook as the source of truth and a thin skill as the agent-facing execution wrapper.

### Public Runbook

Keep the stable release flow in `docs/operations.md` under a dedicated release section.

The runbook should define:

1. release prerequisites
2. how to choose the release commit
3. how to build and publish `risc0` and `sp1` runtime images
4. how to export guest digests
5. how to assemble a release manifest
6. how to create the git tag and GitHub Release

This keeps the process visible to maintainers and external contributors without requiring them to
read internal skill files.

### Thin Skill

Add a dedicated release skill that references the runbook instead of duplicating it.

The skill should enforce these guardrails:

- start from a clean checkout and explicit release commit
- publish both backends
- record image digests and guest digests
- avoid rollout steps, cluster changes, and `register-image --apply` by default

This keeps the skill short and lets the docs remain the canonical flow.

## Release Artifacts

Each release should produce:

- git tag: `vX.Y.Z`
- two image tags:
  - `vX.Y.Z-risc0`
  - `vX.Y.Z-sp1`
- one machine-readable manifest:
  - `release-manifest-vX.Y.Z.json`
- one human-readable notes document:
  - `release-notes-vX.Y.Z.md`

The release manifest should contain:

- `version`
- `tag`
- `git_sha`
- `images`
  - `backend`
  - `tag`
  - `digest_ref`
- `guest_digests`
  - exported directly from the existing `guest-digests` summary

## Suggested Repository Additions

### 1. Runbook Updates

Extend `docs/operations.md` with:

- a `Source Releases` section
- explicit commands for:
  - `just release-image risc0 <tag>`
  - `just release-image sp1 <tag>`
  - `cargo run -p xtask-build-guest --bin guest-digests -- --output <path>`
  - `gh release create ...`

### 2. Release Manifest Helper

Add a small deterministic helper script under `scripts/` that builds the final release manifest from:

- version/tag/SHA
- two image refs + digests
- guest digest summary JSON

This removes error-prone manual JSON editing during the release cut.

### 3. Release Skill

Create a new skill, separate from `raiko2-image-release`, because the image-only skill intentionally
stops at image publication.

Recommended name:

- `raiko2-release-cut`

Responsibilities:

- follow the runbook for a source release
- use the image-only skill for image publication semantics
- produce release notes and release manifest
- stop before deployment or on-chain registration apply

## Why Not Full Automation Yet

The first stable release path should remain inspectable and operator-driven.

Reasons to avoid full automation right now:

- release provenance is easier to verify when the cut is explicit
- image publication, digest capture, and GitHub Release creation are already supported by existing
  commands and `gh`
- this repository does not yet have a mature release workflow for source releases

Once the runbook is stable across a few cuts, the same steps can be promoted into a release
workflow.

## Non-Goals

- automating deployment or rollout
- automatically applying `register-image`
- writing release-only metadata back into source control
- changing version numbers during the release cut when `main` is already on the target version

## Validation Strategy

The implementation should be validated by:

- checking that the new runbook references only commands that already exist in the repo
- validating the manifest helper on sample input
- ensuring the new skill is concise, points at the runbook, and does not duplicate the release
  procedure
