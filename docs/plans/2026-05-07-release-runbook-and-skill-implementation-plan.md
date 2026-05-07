# Release Runbook And Skill Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a public source-release runbook plus a thin `raiko2` release skill that records image
digests and guest digests for each release.

**Architecture:** Keep `docs/operations.md` as the public source of truth for the release flow, add a
small manifest helper script for deterministic machine-readable output, and add a thin agent skill
that references the runbook instead of re-documenting it.

**Tech Stack:** Markdown docs, Python helper script, GitHub CLI examples, Codex skill metadata

---

### Task 1: Add the public source-release runbook

**Files:**
- Modify: `docs/operations.md`

**Step 1: Add a `Source Releases` section**

Document the stable release sequence:

- verify clean `main`
- identify release commit SHA
- publish `risc0` and `sp1` runtime images
- export guest digests
- build release manifest
- create git tag and GitHub Release

**Step 2: Include exact commands**

Use concrete commands for:

- `just release-image risc0 <tag>`
- `just release-image sp1 <tag>`
- `cargo run -p xtask-build-guest --bin guest-digests -- --output <path>`
- manifest helper invocation
- `gh release create ...`

**Step 3: Add release artifact expectations**

List the expected release outputs:

- `vX.Y.Z`
- `vX.Y.Z-risc0`
- `vX.Y.Z-sp1`
- `release-manifest-vX.Y.Z.json`
- `release-notes-vX.Y.Z.md`

### Task 2: Add a deterministic manifest helper

**Files:**
- Create: `scripts/release/write_release_manifest.py`

**Step 1: Define script inputs**

Accept:

- `--version`
- `--tag`
- `--git-sha`
- `--risc0-image`
- `--sp1-image`
- `--guest-digests`
- `--output`

**Step 2: Write minimal implementation**

Load the guest digest JSON and emit a single manifest JSON containing:

- release metadata
- image metadata
- embedded `guest_digests` summary

**Step 3: Add deterministic formatting**

Write compact but stable JSON with:

- sorted keys
- trailing newline
- UTF-8 encoding

### Task 3: Add the release skill

**Files:**
- Create: `.codex/skills/raiko2-release-cut/SKILL.md`
- Create: `.codex/skills/raiko2-release-cut/agents/openai.yaml`

**Step 1: Write concise skill metadata**

Trigger conditions should cover:

- source release
- git tag + GitHub Release
- image digest capture
- guest digest recording

**Step 2: Keep the skill thin**

Reference:

- `docs/operations.md`
- existing `raiko2-image-release` flow boundaries

The skill should emphasize:

- no rollout
- no `register-image --apply` by default
- both backends are required

**Step 3: Add deterministic UI metadata**

Create `agents/openai.yaml` with:

- display name
- short description
- default prompt

### Task 4: Validate docs and helper behavior

**Files:**
- Verify: `docs/operations.md`
- Verify: `scripts/release/write_release_manifest.py`
- Verify: `.codex/skills/raiko2-release-cut/*`

**Step 1: Validate markdown and diff hygiene**

Run:

```bash
git diff --check
```

Expected: no whitespace or patch-format issues

**Step 2: Validate helper script on sample inputs**

Run the script against a real guest digest summary from:

```bash
target/guest-digests/summary.json
```

or a temporary fixture JSON and confirm:

- output JSON parses
- required top-level keys are present

**Step 3: Validate skill presence**

Check that:

- `SKILL.md` exists
- `agents/openai.yaml` exists
- the skill text does not duplicate the whole runbook

### Task 5: Finalize branch

**Files:**
- Stage all files from Tasks 1-4

**Step 1: Review final diff**

Ensure the runbook, script, and skill all agree on:

- both backends required
- release artifact names
- no rollout/apply semantics

**Step 2: Commit**

Suggested commit:

```bash
git commit -m "docs: add source release runbook"
```

**Step 3: Push branch**

Push the feature branch and prepare a PR description summarizing:

- public runbook
- manifest helper
- thin release skill
