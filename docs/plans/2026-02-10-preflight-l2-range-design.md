# Preflight L2 Range Support — Design & Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `preflight` accept an explicit L2 block span (start:end) alongside `proposal_id`, fetch that range, validate it matches the proposal, and embed it into `GuestInput`. This replaces the old assumption that `proposal_id == block number` and aligns with the legacy raiko flow where the block set was verified.

**Architecture:** Extend the CLI to take `--l2-start` / `--l2-end`. Carry these into `ProofRequest`/`ProverConfig`, update provider/pipeline to fetch the supplied block range, and validate the blocks (proposal_id in extradata, contiguous, shared anchor where available). Discovery stays external.

**Tech Stack:** Rust (bin/preflight, pipeline/provider), clap, serde; optional small Python test harness changes later.

---

## Tasks

### Task 1: CLI & request plumbing
**Files:** `bin/preflight/src/main.rs`, `crates/primitives/src/proof.rs` (if ProofRequest needs fields) or `ProverConfig` usage.  
Steps:
1. Add required flags `--l2-start <u64>` and `--l2-end <u64>`; validate start ≤ end.
2. Pass these into the proof context (either extend `ProofRequest` with `l2_block_start/end` or encode into `ProverConfig`).
3. Update help/usage text.

### Task 2: Provider/pipeline block fetching
**Files:** `crates/provider/src/...` (NetworkProvider), `crates/pipeline/src/...` (ShastaSpec build_guest_input path).  
Steps:
1. Accept the supplied start/end in the provider entry point used by preflight.
2. Fetch blocks [start, end] inclusive; reuse existing block-fetch logic (with witnesses if `--debug-witness` is on).
3. Ensure fetched blocks replace the prior single-block assumption; pass into GuestInput witnesses list.

### Task 3: Validation of supplied range
**Files:** same as Task 2 (or helper module).  
Checks (fail fast with clear errors):
- Extradata proposal_id matches the CLI `proposal_id` for every block.
- Blocks are contiguous and ordered.
- (Optional) If anchor/transition data present, ensure consistency of anchor_number across the group (mirrors legacy raiko check).
Add unit tests for the validator with small synthetic blocks.

### Task 4: GuestInput construction updates
**Files:** `crates/pipeline/src/...` (manifest/GuestInput build).  
Steps:
1. Ensure GuestInput includes all fetched blocks/witnesses in order.
2. If any “first/last block” assumptions exist, update them to use the supplied range.

### Task 5: Regression harness compatibility (minimal)
**Files:** `script/regression/shasta_regression.py` (later PR optional).  
Steps (deferred or small patch): allow passing start/end when invoking preflight; otherwise document that callers must supply them.

### Task 6: Documentation
**Files:** `docs/README.md` (brief), `docs/plans/2026-02-10-preflight-l2-range-design.md` (this file).  
Steps:
1. Note new CLI flags and the assumption that block discovery is external.
2. Describe validation behavior and expected inputs.

### Task 7: Verification
Commands:
- `cargo fmt`
-, `cargo clippy --workspace -- -D warnings`
- `cargo test -p preflight` (add new tests)
- Any new validator unit tests.

---

## Notes from legacy raiko (reference)
- Proposal discovery was external; block grouping was verified by checking extradata proposal_id and anchor_number consistency.
- No reth driver was used; full block fetch per block was acceptable for small ranges.
- We mirror the validation, not the discovery, and keep discovery external for now.

