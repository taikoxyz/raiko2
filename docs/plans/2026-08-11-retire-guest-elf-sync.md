# Retire Guest ELF Sync Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove the low-use guest ELF sync workflow so the dedicated self-hosted runner can be retired.

**Architecture:** Guest ELF refreshes remain an explicit local or release-build operation through
`just build-guest`. The read-only build and write-enabled push workflow is removed rather than moved
to the TEE signing runner or a GitHub-hosted runner with insufficient build storage.

**Tech Stack:** GitHub Actions, Rust xtask, Markdown.

---

### Task 1: Remove the obsolete workflow

**Files:**
- Delete: `.github/workflows/sync-guest-elf.yml`

1. Delete the manual workflow.
2. Confirm no active workflow still selects the retired `raiko2` self-hosted runner label.

### Task 2: Update active operator guidance

**Files:**
- Modify: `docs/development.md`

1. Remove the manual workflow from the current guest build instructions.
2. Leave guest fingerprint inputs unchanged so retiring the workflow does not force an unrelated
   guest ELF rebuild. The legacy xtask remediation text can be updated with the next intentional
   guest artifact refresh.

### Task 3: Verify the retirement

1. Run `cargo fmt --all -- --check`.
2. Run `cargo test -p xtask-build-guest`.
3. Search active workflow and source files for the retired runner label and workflow name.
4. Review the final diff for unrelated changes and non-portable paths.
