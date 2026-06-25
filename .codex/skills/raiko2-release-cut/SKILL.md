---
name: raiko2-release-cut
description: Use when cutting a versioned raiko2 ZK source release, including tag creation, runtime image publication, guest digest export, and GitHub Release creation.
---

# Raiko2 Release Cut

## Source Of Truth

- `docs/operations.md` -> `Source Releases`

## Scope

Use this skill to:

- cut a release tag from `main`
- publish the versioned `raiko2` runtime image
- export guest digests
- create release notes and a GitHub Release

Do not use this skill for deployment, rollout, or `register-image --apply`.

## Required Outputs

- git tag: `vX.Y.Z`
- image tag: `vX.Y.Z`
- release notes markdown
- release manifest JSON
- guest digests summary JSON
- Shasta guest ELF/VK assets

## Release Notes

Keep notes focused on changes since the previous release and include ZK guest digests:

- `risc0` proposal and aggregation `image_id`
- `sp1` proposal and aggregation `vk_bn254`
- `sp1` proposal and aggregation `vk_hash_bytes`

## Flow

1. Start from a clean checkout of `main` or the explicit release commit.
2. Run `docs/operations.md` -> `Source Releases`.
3. Publish one runtime image containing both guest backends.
4. Export guest digests from the final release tree.
5. Upload `release-manifest-*.json`, `guest-digests-summary.json`, and Shasta guest ELF/VK assets to the GitHub Release.

## Report

Summarize:

- release tag
- release commit SHA
- runtime image tag and digest
- manifest location
- guest digest asset location
- guest ELF/VK assets
- whether the GitHub Release was created successfully
