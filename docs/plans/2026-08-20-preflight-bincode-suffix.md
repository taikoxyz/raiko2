# Canonical Preflight Bincode Suffix Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rename canonical preflight cache content objects from `.bin` to `.preflight.bincode`.

**Architecture:** Keep the canonical preflight cache schema and payload unchanged and alter only the
GCS object-name constructor. Lock the storage contract with a focused test and update operator-facing
documentation that describes the object layout.

**Tech Stack:** Rust, Tokio tests, Google Cloud Storage object naming, Markdown documentation.

---

### Task 1: Lock the new object suffix with a failing test

**Files:**
- Modify: `crates/runtime/src/artifact_store/gcs_tests.rs`

1. Extend the canonical preflight publication test to assert that the content object ends with
   `.preflight.bincode` and does not end with `.bin`.
2. Run the focused test and confirm it fails against the existing `.bin` implementation.

### Task 2: Rename canonical preflight content objects

**Files:**
- Modify: `crates/runtime/src/artifact_store/gcs.rs`

1. Change `canonical_preflight_content_name` to emit `<hash>.preflight.bincode`.
2. Run the focused test and confirm it passes.
3. Run the complete `raiko2-runtime` test suite.

### Task 3: Align documentation and verify the patch

**Files:**
- Modify: `docs/API.md`
- Modify: `docs/architecture.md`

1. Document the canonical preflight content suffix alongside the existing proof layout.
2. Run `cargo fmt --all -- --check`.
3. Run the focused runtime tests again and inspect the final diff for path hygiene and unrelated
   changes.
