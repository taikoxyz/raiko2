# Mixed Host And Released Guest Image Design

## Goal

Define one deterministic operation for publishing a host image from a selected host revision while
packaging the complete RISC0 and SP1 guest artifact set from an existing raiko2 release.

## Problem

`release-image host` intentionally packages the existing `crates/guests/elf` directory without
rebuilding it, but it also rejects dirty worktrees. The current documentation does not explain how
to create a clean, auditable host/guest composition revision. It also does not separate artifact
identity checks from host/guest protocol and soundness compatibility.

Without an explicit operation, an operator may:

- download only ELF/VK release assets and omit their provenance manifests;
- mix artifact families or releases;
- mistake digest or SP1 VK consistency for host/guest compatibility;
- bypass the clean-tree guard instead of recording the composition in Git;
- publish an image whose host revision and guest release cannot be reconstructed.

## Design

Use a named composition branch created from the selected host revision. Restore the entire
`crates/guests/elf` directory from the selected release tag, preserving ELF, VK, and provenance
files as one release-owned set. Commit only that directory before invoking `release-image host
--skip-guest-refresh`.

The operation has four independent gates:

1. **Input identity:** resolve and record the host commit, selected guest release tag, and release
   commit. Fetch only that tag into a dedicated local ref so unrelated tag conflicts are irrelevant.
2. **Artifact identity:** require the restored directory to match the release tag and published
   Shasta asset inventory exactly, compare every published byte, and validate both backend
   provenance manifests plus their separate complete artifact inventories and hashes and recompute
   Shasta SP1 VKs without source closure.
3. **Compatibility:** only after artifact identity succeeds, run the source-closure check. It covers
   guest build inputs, not all host-side pipeline/prover construction. Every composition therefore
   requires proposal and aggregation regressions on the selected release artifacts. A source
   fingerprint mismatch additionally requires explicit guest-facing diff review and soundness
   approval. The reviewed exception applies only to source drift.
4. **Published-image identity:** require server-side immutable tags and a fail-closed unused-tag
   preflight, then verify the tag resolves to the captured digest, the OCI revision label matches,
   and the packaged artifacts match before reporting the immutable image digest.

The operation stops at image publication. It does not register verifier digests, deploy the image,
or perform Kubernetes operations.

## Source Of Truth

- `.codex/skills/raiko2-image-release/SKILL.md` contains the short mandatory agent workflow and stop
  conditions.
- `docs/operations.md` contains the complete operator SOP and commands.
- The composition commit records the exact host source plus released guest artifact pairing.

## Verification

- Validate the skill structure with the repository-independent skill validator.
- Run a baseline scenario against the old skill and the same scenario against the updated skill.
- Check Markdown command references against current `justfile`, `xtask`, and Dockerfile behavior.
- Run `git diff --check` and confirm no path or example contains machine-specific or identifying
  information.
