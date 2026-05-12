# Shared Shasta Fixture Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the ignored repo-root `test.json` with a tracked shared Shasta fixture that keeps
fixture-backed code paths buildable and testable from a clean checkout.

**Architecture:** Add one repo-owned JSON fixture under `tests/fixtures/`, generate its contents
from the in-tree `GuestInput` types so the schema stays valid, and point all existing fixture
consumers at that shared path. Keep docs explicit that this file is a checked-in sample, while
`preflight` remains only a way to regenerate similar input offline.

**Tech Stack:** Rust workspace crates, `serde_json`, `cargo`, markdown docs.

---

### Task 1: Prove the current dependency is broken

**Files:**
- Modify: none
- Test: `crates/primitives-shasta/tests/guest_input_bincode_roundtrip.rs`

**Step 1: Run the focused failing test**

Run: `cargo test -p raiko2-primitives-shasta --test guest_input_bincode_roundtrip guest_input_json_to_bincode_roundtrip_test_json`
Expected: FAIL because `../../../test.json` does not exist.

**Step 2: Run the focused compile check**

Run: `cargo check -p raiko2 --bin raiko2`
Expected: FAIL because `bin/raiko2/src/server/fixture.rs` cannot read `../../../../test.json`.

### Task 2: Add the tracked shared fixture

**Files:**
- Create: `tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json`

**Step 1: Generate a valid `GuestInput` via preflight**

- Use the checked-in `preflight` tool against a known-good Taiko mainnet tuple.
- Record the exact tuple used so the fixture can be regenerated later:
  - `proposal_id = 2222`
  - `l2_start = 5412225`
  - `l2_end = 5412416`
  - `l1_inclusion_block_number = 24862953`
  - `last_anchor_block_number = 24862885`
  - `l2_chain_id = 167000`

**Step 2: Serialize it into the tracked fixture path**

Run `preflight` so it writes
`tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json`.

### Task 3: Repoint code and docs

**Files:**
- Modify: `bin/raiko2/src/server/fixture.rs`
- Modify: `crates/primitives-shasta/tests/guest_input_bincode_roundtrip.rs`
- Modify: `docs/development.md`

**Step 1: Update code references**

- Replace both `include_str!(...test.json)` paths with the new shared fixture path.
- Rename helper text/comments from `test.json` to
  `shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json` where useful.

**Step 2: Update developer docs**

- Replace `--input ./test.json` examples with
  `--input ./tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json`.
- Clarify that the checked-in fixture is a repo-managed sample, not an ignored local artifact.

### Task 4: Verify the new flow

**Files:**
- Modify: none
- Test: `tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json`

**Step 1: Format**

Run: `cargo fmt --all`
Expected: PASS

**Step 2: Re-run the focused roundtrip test**

Run: `cargo test -p raiko2-primitives-shasta --test guest_input_bincode_roundtrip guest_input_json_to_bincode_roundtrip_shared_fixture`
Expected: PASS

**Step 3: Re-run the binary compile check**

Run: `cargo check -p raiko2 --bin raiko2`
Expected: PASS
