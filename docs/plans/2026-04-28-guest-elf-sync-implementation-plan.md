# Guest ELF Sync Workflow Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a manual workflow that rebuilds guest ELFs and commits updated ELF files back to the selected branch without retriggering heavy CI on ELF-only commits.

**Architecture:** Introduce a standalone `workflow_dispatch` GitHub Actions workflow for guest ELF refreshes, then teach the main CI filters to ignore `crates/guests/elf/**` so sync commits do not fan out into redundant Rust and zk build jobs.

**Tech Stack:** GitHub Actions, `actions/checkout`, `actions/upload-artifact`, repository `just build-guest` entrypoints, git CLI inside Actions.

---

### Task 1: Update main CI filters to ignore ELF-only sync commits

**Files:**
- Modify: `.github/workflows/ci.yml`

**Step 1: Update the `rust` and `zk` filters**

Add exclusions for `crates/guests/elf/**` so an ELF-only commit does not trigger the heavy lanes.

**Step 2: Keep workflow-only behavior intact**

Preserve existing `.github/workflows/ci.yml` handling so workflow edits still run the intended checks.

**Step 3: Verify YAML syntax**

Run: `python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/ci.yml"))'`
Expected: no output, exit code 0

### Task 2: Add the manual sync workflow

**Files:**
- Create: `.github/workflows/sync-guest-elf.yml`

**Step 1: Define dispatch inputs**

Add `workflow_dispatch` inputs:
- `backend` with `risc0`, `sp1`, `all`
- `ref` as optional branch name override

**Step 2: Resolve target branch**

Use the provided `ref` when non-empty; otherwise fall back to `${{ github.ref_name }}` so dispatching from a branch updates that same branch by default.

**Step 3: Build selected guest ELFs**

Check out the target branch and run `just build-guest <backend>`.

**Step 4: Upload generated ELFs**

Upload the corresponding ELF files as artifacts with short retention for audit/debugging.

### Task 3: Commit and push ELF updates when needed

**Files:**
- Modify: `.github/workflows/sync-guest-elf.yml`

**Step 1: Detect whether `crates/guests/elf` changed**

Use `git diff --quiet -- crates/guests/elf` or equivalent shell logic.

**Step 2: Commit only when changed**

Configure bot identity in the workflow, stage `crates/guests/elf`, and create a focused commit such as:

`chore: sync guest elf outputs`

**Step 3: Push back to the target branch**

Push the generated commit to the resolved branch ref.

**Step 4: Make the no-op case explicit**

Emit a short log message and succeed without commit when no ELF bytes changed.

### Task 4: Validate workflow files locally

**Files:**
- Modify: `.github/workflows/ci.yml`
- Create: `.github/workflows/sync-guest-elf.yml`

**Step 1: Validate both workflow YAML files**

Run:

`python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/ci.yml")); yaml.safe_load(open(".github/workflows/sync-guest-elf.yml"))'`

Expected: no output, exit code 0

**Step 2: Check diff formatting**

Run: `git diff --check`
Expected: no output, exit code 0

### Task 5: Review final branch state

**Files:**
- Modify: `.github/workflows/ci.yml`
- Create: `.github/workflows/sync-guest-elf.yml`

**Step 1: Inspect staged diff summary**

Run: `git diff --stat`

**Step 2: Commit the workflow changes**

Use a Conventional Commit such as:

`ci: add manual guest elf sync workflow`

**Step 3: Push the branch**

Run: `git push origin feat/sync-guest-elf`

**Step 4: Report exact validation commands**

Include the YAML validation commands and note that end-to-end workflow execution still needs GitHub Actions to confirm branch write behavior.
