# Shasta Filtered Block Helper Extraction Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Preserve the current one-pass txlist proving semantics while shrinking `raiko2`'s public/stateless surface toward a thin `alethia-reth-block` helper.

**Architecture:** Stage the work in two layers. First, keep the existing one-pass behavior but replace raw `reth` builder-result exposure in `raiko2-stateless` with a helper-owned filtered-block outcome type. Second, once `alethia-reth-block` exposes the thin replay/filter/assemble helper, swap the local implementation over and remove direct `BlockBuilder` coupling from `raiko2`.

**Tech Stack:** Rust, `raiko2-stateless`, `raiko2-guest-common`, witness-backed sparse trie DB, `alethia-reth-block`, existing Shasta proposal/stateless regressions.

---

### Task 1: Add the failing compatibility-surface regression

**Files:**
- Modify: `crates/stateless/src/validation.rs`
- Modify: `guests/common/tests/proposal_validation.rs`
- Test: `crates/stateless/src/validation.rs`
- Test: `guests/common/tests/proposal_validation.rs`

**Step 1: Write the failing test**

Change the existing reconstruction/proposal tests to depend on a helper-owned outcome field such as
`filtered_block`, instead of reaching through raw `BlockBuilderOutcome::block`.

**Step 2: Run test to verify it fails**

Run: `cargo test -p raiko2-stateless reconstruct_block_skips_invalid_nonce_transaction`
Run: `cargo test --manifest-path guests/common/Cargo.toml --test proposal_validation top_level_proposal_proof_reconstructs_block_and_skips_invalid_tx`

Expected: FAIL to compile or fail assertions because the new compatibility surface does not exist yet.

**Step 3: Commit**

Do not commit yet. This task establishes the red test only.

### Task 2: Introduce a helper-owned filtered-block outcome type

**Files:**
- Modify: `crates/stateless/src/lib.rs`
- Modify: `crates/stateless/src/validation.rs`

**Step 1: Add the minimal compatibility type**

Add an exported result type such as `FilteredBlockExecutionOutcome` that carries:

- `filtered_block`
- `execution_result`
- `hashed_state`
- `trie_updates`

Change `reconstruct_block_from_transactions_with_witness_resources(...)` to return this helper-owned
type instead of raw `BlockBuilderOutcome`.

**Step 2: Keep the existing reconstruction behavior**

Internally keep the same one-pass builder logic and skip-invalid semantics. This task should be a
surface refactor, not a semantic change.

**Step 3: Run targeted tests**

Run: `cargo test -p raiko2-stateless reconstruct_block_skips_invalid_nonce_transaction`

Expected: PASS

### Task 3: Switch guest/proposal callers to the compatibility surface

**Files:**
- Modify: `guests/common/src/lib.rs`
- Test: `guests/common/tests/proposal_validation.rs`

**Step 1: Update `prove_shasta_proposal` and tests**

Use `outcome.filtered_block` everywhere the guest/tests currently reach through `outcome.block`.

**Step 2: Run the focused guest regression**

Run: `cargo test --manifest-path guests/common/Cargo.toml --test proposal_validation top_level_proposal_proof_reconstructs_block_and_skips_invalid_tx`

Expected: PASS

**Step 3: Run the full guest proposal test file**

Run: `cargo test --manifest-path guests/common/Cargo.toml --test proposal_validation`

Expected: PASS

### Task 4: Document and prepare the dependency-side extraction

**Files:**
- Modify: `docs/issues/2026-04-21-shasta-zk-path-does-not-rebuild-block-from-derived-txlist.md`
- Modify: `docs/plans/2026-04-22-shasta-filtered-block-helper-design.md`
- Modify: `docs/plans/2026-04-22-main-minimal-one-pass-txlist-proving-implementation-plan.md`

**Step 1: Record the exact upstream helper shape**

Document the intended `alethia-reth-block` helper signature and rollout order so the later
dependency extraction is mechanical rather than design work.

**Step 2: Format and verify**

Run: `cargo fmt --all`
Run: `cargo test -p raiko2-stateless reconstruct_block_skips_invalid_nonce_transaction`
Run: `cargo test --manifest-path guests/common/Cargo.toml --test proposal_validation`

Expected: PASS

**Step 3: Inspect diff**

Run: `git diff --stat`

Expected: current diff narrows the local API and leaves the future `alethia-reth-block` extraction
as a small implementation swap rather than another guest-path rewrite.
