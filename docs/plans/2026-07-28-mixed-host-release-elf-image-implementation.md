# Mixed Host And Released Guest Image SOP Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Document and enforce a deterministic SOP for publishing a host image with guest artifacts
from a selected raiko2 release.

**Architecture:** Keep the complete command sequence in `docs/operations.md` and the mandatory
decision gates in `raiko2-image-release/SKILL.md`. Use a clean composition branch as the auditable
source revision, preserve the release artifact directory as one unit, and stop before publication
when compatibility is not established.

**Tech Stack:** Markdown, Git worktrees, GitHub Releases, Cargo xtask, just, Docker Buildx.

---

### Task 1: Add The Operator SOP

**Files:**
- Modify: `docs/operations.md`

**Step 1: Document fixed inputs and the composition branch**

Add commands that resolve `HOST_REF`, `ELF_TAG`, `HOST_SHA`, and `ELF_SHA`, fetch only the selected
remote tag into a dedicated ref, create a clean `main-<host>-elf-<release>` worktree branch, and
restore the complete `crates/guests/elf` directory from the release tag.

**Step 2: Document independent validation gates**

Require:

- release tag and GitHub Release identity;
- tag-directory equality and exact published Shasta asset inventory and byte equality;
- artifact-only provenance, per-backend inventory, hash validation, and Shasta SP1 VK recomputation;
- source-closure check after artifact-only validation;
- mandatory proposal and aggregation regressions because source closure excludes host-side
  pipeline/prover construction;
- explicit source-only reviewed exception handling for guest-facing source drift.

State that digest and SP1 VK checks do not prove host/guest protocol or soundness compatibility.

**Step 3: Document build and post-push verification**

Commit only `crates/guests/elf`, require registry-side immutable tags, fail closed unless the image
tag is conclusively absent, run `just release-image host ... --skip-guest-refresh`, capture the
immutable digest, and verify tag-to-digest resolution, the image revision label, and packaged
artifact hashes.

### Task 2: Update The Agent Skill

**Files:**
- Modify: `.codex/skills/raiko2-image-release/SKILL.md`

**Step 1: Add the mixed-version trigger**

Extend the skill description so requests for a current/new host image with guest ELF/VK from an
existing release reliably load the skill.

**Step 2: Add a concise mandatory workflow**

Require the selected-tag ref, composition branch, exact release asset set, artifact-only provenance
and SP1 VK validation for both backends, mandatory proposal and aggregation regressions,
source-drift compatibility gate, clean commit, immutable registry tags, fail-closed tag absence,
host build without refresh, and immutable output report. Link the full commands to
`docs/operations.md`.

**Step 3: Add explicit stop conditions**

Forbid partial artifact replacement, provenance regeneration against old binaries, dirty-tree
bypasses, treating digest equality as compatibility, and publication after an unapproved
guest-facing drift.

### Task 3: Verify The Skill And Documentation

**Files:**
- Test: `.codex/skills/raiko2-image-release/SKILL.md`
- Test: `docs/operations.md`

**Step 1: Validate skill structure**

Run:

```bash
python3 "${CODEX_HOME:-$HOME/.codex}/skills/.system/skill-creator/scripts/quick_validate.py" \
  .codex/skills/raiko2-image-release
```

Expected: validation succeeds.

**Step 2: Forward-test the skill**

Ask a fresh agent to use the updated skill to publish a host image from current `origin/main` with
all RISC0/SP1 artifacts from `vX.Y.Z`, without modifying files or deploying. Confirm its SOP:

- creates a named composition branch rather than a detached-only revision;
- restores provenance with ELF/VK files;
- exact-set checks the GitHub Release assets and validates both provenance manifests before source
  closure;
- distinguishes artifact identity from compatibility;
- always requires proposal and aggregation regressions because host-side construction is outside the
  source-closure fingerprint;
- stops on unresolved guest-facing drift;
- refuses mutable repositories, existing tags, and inconclusive registry errors;
- verifies the published tag resolves to the captured digest;
- ends after image publication and verification.

**Step 3: Run static documentation checks**

Run:

```bash
git diff --check origin/main...HEAD
rg -n "main-.*-elf-|skip-guest-refresh|provenance" \
  .codex/skills/raiko2-image-release/SKILL.md docs/operations.md
```

Expected: no hardcoded user paths; required SOP terms are present.

**Step 4: Commit**

```bash
git add .codex/skills/raiko2-image-release/SKILL.md docs/operations.md \
  docs/plans/2026-07-28-mixed-host-release-elf-image-implementation.md
git commit -m "docs(release): add mixed host guest image SOP"
```
