# Taiko Stateless Consensus Validation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Use Alethia-Reth Taiko consensus (`TaikoBeaconConsensus`) + `TaikoEvmConfig` for stateless `validate_block` in `raiko2-stateless`, including Shasta-specific header rules and anchor transaction validation.

**Architecture:** Add a Taiko-specific consensus validation path inside `crates/stateless/src/validation.rs`. Build a minimal `TaikoBlockReader` from witness-provided ancestor headers so consensus can (optionally) read the grandparent timestamp. If grandparent data is missing, allow validation to continue (fallback behavior in Alethia consensus).

**Tech Stack:** Rust 2024 workspace, `reth-*` crates (v1.9.3), Alethia-Reth crates (`alethia-reth-consensus`, `alethia-reth-node`), unit tests via `cargo test`.

## Task 1: Stateless consensus (Taiko)

**Files:**
- Modify: `crates/stateless/src/validation.rs`
- Modify: `crates/stateless/Cargo.toml`
- Modify: `Cargo.toml` (workspace dependency)
- Test: `crates/stateless/src/validation.rs` (unit tests in-module)

**Step 1: Write the failing test**

- Add a unit test that calls `raiko2_stateless::validate_block` with:
  - `TaikoChainSpec` set to `TAIKO_DEVNET` (Shasta active at timestamp 0).
  - A block header whose timestamp is **equal** to its parent header timestamp.
  - A witness that contains only the parent header.
- Expected: `validate_block(...)` returns `StatelessValidationError::ConsensusValidationFailed`.

**Step 2: Run test to verify it fails**

Run: `cargo test -p raiko2-stateless --lib`

Expected: FAIL (currently consensus does not validate header against parent timestamp).

**Step 3: Minimal implementation**

- Add Alethia-Reth consensus dependency and use `TaikoBeaconConsensus` in the pre-execution consensus phase.
- In addition to `validate_header`, call `validate_header_against_parent` using the parent header from witness.
- Add post-execution anchor validation using `validate_anchor_transaction_in_block`.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p raiko2-stateless --lib`

Expected: PASS.

## Task 2: Guest per-crate patches

**Files:**
- Modify: `crates/guest-risc0/Cargo.toml`
- Modify: `crates/guest-sp1/Cargo.toml`
- Create: `crates/guest-risc0/.cargo/config.toml`
- Create: `crates/guest-sp1/.cargo/config.toml`
- Modify: `script/build-guest.sh`

**Step 1: Reproduce current warning**

Run: `cargo check -p raiko2-guest-risc0` and `cargo check -p raiko2-guest-sp1`

Expected: Warning about non-root `[patch]` being ignored.

**Step 2: Apply per-crate patch config**

- Move guest-specific `[patch.crates-io]` from the guest `Cargo.toml` files into per-crate `.cargo/config.toml`.
- Adjust guest build script to use per-backend lockfiles to avoid patch conflicts.

**Step 3: Verify warning is gone**

Run: `script/build-guest.sh risc0` and `script/build-guest.sh sp1` (or equivalent).

Expected: No "patch for non root package" warning.

## Final verification

- Run: `cargo fmt --all`.
- Run: `cargo clippy -p raiko2-stateless --all-targets -D warnings`.
