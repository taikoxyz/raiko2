# Guest Digests Export Implementation Plan

> Implement this plan task-by-task, validating each step before moving to the next.

**Goal:** Add a local `xtask-build-guest` `guest-digests` command and wire it into `guest-elf-consistency` so GitHub Actions exposes the current Shasta guest registration digests as a summary artifact.

**Architecture:** Introduce a small offline digest-export module in `xtask-build-guest` that loads the current checked-in guest ELFs and computes the same Shasta registration digests needed by verifier contracts. Then call it from the existing self-hosted `guest-elf-consistency` job after the rebuild step, upload the JSON artifact, render a short Markdown summary, and keep the existing ELF drift gate unchanged.

**Tech Stack:** Rust (`xtask-build-guest`, `serde_json`, `risc0-zkvm`, `sp1-sdk`), GitHub Actions, existing guest ELF loaders from `raiko2-guests`

---

### Task 1: Add failing digest-export tests

**Files:**
- Create: `xtask/build-guest/src/guest_digests.rs`
- Create: `xtask/build-guest/src/bin/guest-digests.rs`
- Modify: `xtask/build-guest/Cargo.toml`

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

Run: `cargo test -p xtask-build-guest guest_digests`

Expected: compile failure because the lightweight `guest-digests` command/module does not exist yet.

### Task 2: Implement the offline digest exporter

**Files:**
- Create: `xtask/build-guest/src/guest_digests.rs`
- Create: `xtask/build-guest/src/bin/guest-digests.rs`
- Modify: `xtask/build-guest/Cargo.toml`
- Modify: `xtask/src/main.rs`

**Step 1: Add CLI entrypoint**

Add a new lightweight `guest-digests` binary in `xtask-build-guest`, and keep the `xtask`
subcommand as a thin delegate if the human-facing CLI should remain available.

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

Run: `cargo test -p xtask-build-guest guest_digests`

Expected: PASS

### Task 3: Integrate digest export into GitHub Actions

**Files:**
- Modify: `.github/workflows/ci.yml`

**Step 1: Add digest generation after guest rebuild**

Run:

`cargo run -p xtask-build-guest --bin guest-digests -- --output target/guest-digests/summary.json`

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

Run: `cargo test -p xtask-build-guest guest_digests`

Expected: PASS

**Step 2: Parse workflow YAML**

Run: `python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/ci.yml"))'`

Expected: PASS

**Step 3: Run formatting sanity check**

Run: `git diff --check`

Expected: PASS

**Step 4: Commit**

```bash
git add xtask/build-guest/Cargo.toml xtask/build-guest/src/guest_digests.rs xtask/build-guest/src/bin/guest-digests.rs xtask/src/main.rs .github/workflows/ci.yml docs/plans/2026-05-06-guest-digests-design.md docs/plans/2026-05-06-guest-digests-implementation-plan.md
git commit -m "ci: publish guest digest summaries"
```
