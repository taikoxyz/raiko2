# Guest Digests Export Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a local `xtask guest-digests` command and wire it into `guest-elf-consistency` so GitHub Actions exposes the current Shasta guest registration digests as a summary artifact.

**Architecture:** Introduce a small offline digest-export module in `xtask` that loads the current checked-in guest ELFs and computes the same Shasta registration digests needed by verifier contracts. Then call it from the existing self-hosted `guest-elf-consistency` job after the rebuild step, upload the JSON artifact, render a short Markdown summary, and keep the existing ELF drift gate unchanged.

**Tech Stack:** Rust (`xtask`, `serde_json`, `risc0-zkvm`, `sp1-sdk`), GitHub Actions, existing guest ELF loaders from `raiko2-guests`

---

### Task 1: Add failing digest-export tests

**Files:**
- Create: `xtask/src/guest_digests.rs`
- Modify: `xtask/src/main.rs`

**Step 1: Write the failing tests**

Add tests that expect:

- a `risc0` digest export containing:
  - `risc0_shasta_proposal`
  - `risc0_shasta_aggregation`
  - `risc0_shasta_boundless_aggregation`
- an `sp1` digest export containing:
  - `sp1_shasta_proposal`
  - `sp1_shasta_aggregation`
- the expected digest-source counts:
  - `risc0`: three `image_id`
  - `sp1`: two `vk_bn254` plus two `vk_hash_bytes`

**Step 2: Run tests to verify they fail**

Run: `cargo test -p xtask guest_digests`

Expected: compile failure because `guest_digests` command/module does not exist yet.

### Task 2: Implement the offline digest exporter

**Files:**
- Create: `xtask/src/guest_digests.rs`
- Modify: `xtask/src/main.rs`

**Step 1: Add CLI entrypoint**

Add a new `GuestDigests` subcommand in `xtask/src/main.rs`.

**Step 2: Implement minimal data model**

Create:

- proof-system enum
- stage enum
- digest-source enum
- digest-entry struct
- summary-file struct
- CLI args struct with optional `--output`

**Step 3: Implement digest collection**

Load ELFs from `crates/guests/elf` using existing guest loaders and compute:

- `risc0` image ids with `compute_image_id`
- `sp1` verification-key digests with `ProverClient::setup(...).1`

**Step 4: Implement JSON output**

Write a stable pretty-printed JSON summary file to the requested output path.

**Step 5: Re-run focused tests**

Run: `cargo test -p xtask guest_digests`

Expected: PASS

### Task 3: Integrate digest export into GitHub Actions

**Files:**
- Modify: `.github/workflows/ci.yml`

**Step 1: Add digest generation after guest rebuild**

Run:

`cargo run -p xtask -- guest-digests --output target/guest-digests/summary.json`

**Step 2: Upload artifact**

Upload `target/guest-digests/summary.json` as a short-retention artifact.

**Step 3: Render GitHub job summary**

Convert the JSON entries into a readable Markdown table in `GITHUB_STEP_SUMMARY`.

**Step 4: Keep ELF drift check last**

Leave the existing dirty-tree gate after digest export so a failing consistency check still leaves
the audit artifact visible.

### Task 4: Verify the full change

**Files:**
- Modify: none

**Step 1: Run focused Rust tests**

Run: `cargo test -p xtask guest_digests`

Expected: PASS

**Step 2: Parse workflow YAML**

Run: `python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/ci.yml"))'`

Expected: PASS

**Step 3: Run formatting sanity check**

Run: `git diff --check`

Expected: PASS

**Step 4: Commit**

```bash
git add xtask/src/main.rs xtask/src/guest_digests.rs .github/workflows/ci.yml docs/plans/2026-05-06-guest-digests-design.md docs/plans/2026-05-06-guest-digests-implementation-plan.md
git commit -m "ci: publish guest digest summaries"
```
