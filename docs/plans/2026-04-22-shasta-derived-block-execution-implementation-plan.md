# Shasta Derived Block Execution Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the current provider-based Shasta proposal reconstruction path with a block-based `execute(derived_block)` flow that lives primarily in `alethia-reth`, while keeping witness materialization and post-state root calculation in `raiko2`.

**Architecture:** First add a prover-only block-level API in `alethia-reth-block` that executes a candidate derived block, skips invalid non-anchor transactions, and exposes the committed transaction set plus execution artifacts. Then switch `raiko2-stateless` and the proposal guest to construct a candidate block, execute it against `WitnessDatabase`, assemble the filtered canonical block through `alethia-reth`, and compare it with the expected block supplied by the L2 chain.

**Tech Stack:** Rust, `alethia-reth-block`, `raiko2-stateless`, `raiko2-guest-common`, witness-backed sparse trie DB, `reth` v2.1.0 execution traits, Shasta proposal regressions.

---

### Task 1: Add the failing `alethia-reth` block-level regression

**Files:**
- Modify: `../alethia-reth/crates/block/src/lib.rs`
- Modify: `../alethia-reth/crates/block/src/executor.rs`
- Create: `../alethia-reth/crates/block/src/derived_block.rs`
- Test: `../alethia-reth/crates/block/src/derived_block.rs`

**Step 1: Write the failing test**

Add a prover-only regression that constructs a candidate block with:

- anchor transaction at index `0`
- one valid non-anchor transaction
- one invalid non-anchor transaction with nonce too high or balance too low

Assert that:

- block execution succeeds
- only the valid non-anchor transaction appears in `committed_transactions`
- `hashed_state` is returned
- the assembled filtered block body excludes the invalid transaction

Test skeleton:

```rust
#[cfg(feature = "prover")]
#[test]
fn execute_derived_block_skips_invalid_nonce_transaction_and_records_committed_txs() {
    let outcome = execute_derived_block(&config, &parent, derived_block, db).unwrap();
    assert_eq!(outcome.committed_transactions.len(), 2);

    let filtered = assemble_filtered_block(
        &config,
        &parent,
        &derived_block,
        outcome.committed_transactions.clone(),
        &outcome.execution_result,
        state_root,
    )
    .unwrap();

    assert_eq!(filtered.body().transactions().count(), 2);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p alethia-reth-block execute_derived_block_skips_invalid_nonce_transaction_and_records_committed_txs --features prover`

Expected: FAIL because the block-level prover helper does not exist yet.

**Step 3: Commit**

Do not commit yet. This task establishes the failing upstream regression first.

### Task 2: Implement the `alethia-reth` derived-block prover API

**Files:**
- Modify: `../alethia-reth/crates/block/src/lib.rs`
- Create: `../alethia-reth/crates/block/src/derived_block.rs`
- Modify: `../alethia-reth/crates/block/src/executor.rs`
- Modify: `../alethia-reth/crates/block/src/config.rs`
- Modify: `../alethia-reth/crates/block/src/factory.rs`
- Delete or replace: `../alethia-reth/crates/block/src/filtered_block.rs`

**Step 1: Add the new outcome type and public API**

Introduce a prover-only surface along these lines:

```rust
#[cfg(feature = "prover")]
pub struct DerivedBlockExecutionOutcome {
    pub committed_transactions: Vec<TransactionSigned>,
    pub execution_result: BlockExecutionResult<Receipt>,
    pub hashed_state: HashedPostState,
}

#[cfg(feature = "prover")]
pub fn execute_derived_block<DB>(
    evm_config: &TaikoEvmConfig,
    parent_header: &SealedHeader,
    derived_block: RecoveredBlock<Block>,
    db: DB,
) -> Result<DerivedBlockExecutionOutcome, BlockExecutionError>
where
    DB: StateDB;

#[cfg(feature = "prover")]
pub fn assemble_filtered_block(
    evm_config: &TaikoEvmConfig,
    parent_header: &SealedHeader,
    derived_block: &RecoveredBlock<Block>,
    committed_transactions: Vec<TransactionSigned>,
    execution_result: &BlockExecutionResult<Receipt>,
    state_root: B256,
) -> Result<RecoveredBlock<Block>, BlockExecutionError>;
```

**Step 2: Reuse normal block execution instead of builder replay**

Implement `execute_derived_block(...)` by:

- creating the execution env from the candidate block header
- creating the Taiko execution context from the candidate block
- executing the full candidate block through the prover-mode block executor
- recording committed signed transactions as the executor accepts them
- returning `execution_result` plus `hashed_state`

Do not use:

- `StateProvider`
- `StateProviderDatabase`
- `builder_for_next_block(...)`
- `builder.finish(...)`

**Step 3: Reuse `TaikoBlockAssembler` for final assembly**

Implement `assemble_filtered_block(...)` by reusing the existing Taiko block assembler with the
filtered committed transaction list and caller-provided `state_root`.

If the assembler API requires state/provider placeholders, pass inert values internally from the
helper rather than exposing those requirements to `raiko2`.

**Step 4: Run focused upstream tests**

Run: `cargo test -p alethia-reth-block execute_derived_block_skips_invalid_nonce_transaction_and_records_committed_txs --features prover`

Expected: PASS

Run: `cargo test -p alethia-reth-block --features prover`

Expected: PASS for the touched block crate tests.

**Step 5: Commit**

```bash
git -C ../alethia-reth add crates/block/src/lib.rs crates/block/src/derived_block.rs crates/block/src/executor.rs crates/block/src/config.rs crates/block/src/factory.rs
git -C ../alethia-reth commit -m "feat(block): add derived block prover execution helper"
```

### Task 3: Patch `raiko2` to consume the local `alethia-reth` helper

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

**Step 1: Add a temporary local patch for development**

Add a workspace-level patch block that points the touched `alethia-reth` crates to the local
checkout while the upstream PR is still open.

Patch shape:

```toml
[patch."https://github.com/taikoxyz/alethia-reth"]
alethia-reth-block = { path = "../alethia-reth/crates/block" }
alethia-reth-chainspec = { path = "../alethia-reth/crates/chainspec" }
alethia-reth-consensus = { path = "../alethia-reth/crates/consensus" }
alethia-reth-primitives = { path = "../alethia-reth/crates/primitives" }
```

**Step 2: Update the lockfile**

Run: `cargo metadata --format-version 1 >/dev/null`

Expected: lockfile refreshes to use the local patch source.

**Step 3: Commit**

Do not commit yet. Keep this patch local until the code compiles against the new API.

### Task 4: Replace the `raiko2` proposal reconstruction path

**Files:**
- Modify: `crates/stateless/src/lib.rs`
- Modify: `crates/stateless/src/validation.rs`
- Modify: `crates/stateless/src/witness_db.rs`
- Modify: `guests/common/src/lib.rs`
- Test: `crates/stateless/src/validation.rs`
- Test: `guests/common/tests/proposal_validation.rs`

**Step 1: Write or update the failing local regressions**

Keep these regressions as the acceptance criteria for the refactor:

- `cargo test -p raiko2-stateless reconstruct_block_skips_invalid_nonce_transaction`
- `cargo test --manifest-path guests/common/Cargo.toml --test proposal_validation top_level_proposal_proof_reconstructs_block_and_skips_invalid_tx`

If necessary, add one more guest regression that fails when the expected derived block list is
missing in proposal mode:

```rust
#[test]
fn proposal_proof_requires_expected_block_list() {
    let mut guest_input = canonical_inline_source_guest_input();
    guest_input.taiko.proposal_event.proposal.sources.clear();
    let err = prove_shasta_proposal(&guest_input).unwrap_err();
    assert!(err.to_string().contains("missing expected Shasta block"));
}
```

**Step 2: Build a candidate `derived_block` locally**

Change `reconstruct_block_from_transactions_with_witness_resources(...)` so it constructs a full
candidate `RecoveredBlock<Block>` from:

- the canonical anchor transaction
- the derived non-anchor txlist
- `TaikoNextBlockEnvAttributes`
- the parent header

The candidate header should preserve the next-block metadata already validated from the expected
manifest and canonical block.

**Step 3: Execute with `WitnessDatabase`, not `WitnessStateProvider`**

Replace the current provider-based flow:

```rust
let provider = WitnessStateProvider::new(trie, bytecode, ancestor_hashes);
let outcome = execute_and_filter_block_transactions(..., provider)?;
```

with the DB-based flow:

```rust
let db = WitnessDatabase::new(&trie, bytecode, ancestor_hashes);
let outcome = execute_derived_block(evm_config, &parent_header, derived_block.clone(), db)?;
let state_root = trie.calculate_state_root(outcome.hashed_state.clone())?;
let filtered_block = assemble_filtered_block(
    evm_config,
    &parent_header,
    &derived_block,
    outcome.committed_transactions,
    &outcome.execution_result,
    state_root,
)?;
```

**Step 4: Remove proposal-only provider glue**

Delete `WitnessStateProvider` and any proposal-only helper paths that only existed to support
`StateProviderDatabase` reconstruction.

Keep `WitnessDatabase` intact because the normal stateless replay path still depends on it.

**Step 5: Tighten proposal-mode invariants**

Make proposal proving fail fast if the expected block list is absent. `prove_shasta_proposal(...)`
must not silently accept `None` for the expected derived block sequence, because that expected
sequence is part of the statement provided by the L2 chain.

**Step 6: Run focused local tests**

Run: `cargo test -p raiko2-stateless reconstruct_block_skips_invalid_nonce_transaction`

Expected: PASS

Run: `cargo test --manifest-path guests/common/Cargo.toml --test proposal_validation top_level_proposal_proof_reconstructs_block_and_skips_invalid_tx`

Expected: PASS

Run: `cargo test --manifest-path guests/common/Cargo.toml --test proposal_validation`

Expected: PASS

**Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/stateless/src/lib.rs crates/stateless/src/validation.rs crates/stateless/src/witness_db.rs guests/common/src/lib.rs guests/common/tests/proposal_validation.rs
git commit -m "refactor(stateless): execute derived proposal blocks through alethia"
```

### Task 5: Remove the temporary patch and repin to the upstream revision

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `docs/plans/2026-04-22-shasta-derived-block-execution-design.md`
- Modify: `docs/issues/2026-04-21-shasta-zk-path-does-not-rebuild-block-from-derived-txlist.md`

**Step 1: Switch back from local patch to git revision**

After the `alethia-reth` PR merges, remove the local `[patch]` block and update the workspace
dependency `rev` to the merged commit.

**Step 2: Append the final rollout notes**

Record:

- the exact merged `alethia-reth` commit
- that `WitnessStateProvider` was deleted
- that proposal proving now uses block-level execution semantics
- any remaining follow-up items for canonical stateless validation or API cleanup

**Step 3: Run end-to-end verification**

Run: `cargo fmt --all`

Expected: PASS

Run: `cargo clippy --workspace -- -D warnings`

Expected: PASS

Run: `cargo nextest run --workspace`

Expected: PASS

If the touched helper changes guest-facing semantics, also run:

Run: `just build-guest sp1`

Expected: PASS

**Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock docs/plans/2026-04-22-shasta-derived-block-execution-design.md docs/issues/2026-04-21-shasta-zk-path-does-not-rebuild-block-from-derived-txlist.md
git commit -m "chore(deps): repin alethia after derived block helper merge"
```

## Append Log

### 2026-04-23 00:35 CST

Current local implementation status:

- alethia helper has been implemented in the local patched checkout
- `raiko2-stateless` now calls block-based `execute_derived_block(...)` and
  `assemble_filtered_block(...)`
- `WitnessStateProvider` has been removed from the local raiko2 patch
- `WitnessDatabase` remains the only witness-backed execution DB in the reconstruction path
- `guests/common` also has a temporary alethia `[patch]` because it is an independent workspace

Adjustment to Step 3:

- `assemble_filtered_block(...)` now requires `finalized_block_zk_gas`
- the call sequence is:

```rust
let db = WitnessDatabase::new(&trie, bytecode, ancestor_hashes);
let outcome = execute_derived_block(evm_config, &parent_header, &derived_block, db)?;
let state_root = trie.calculate_state_root(outcome.hashed_state.clone())?;
let filtered_block = assemble_filtered_block(
    evm_config,
    &parent_header,
    &derived_block,
    outcome.committed_transactions,
    &outcome.execution_result,
    outcome.finalized_block_zk_gas,
    state_root,
)?;
```

Focused tests run and passed in the local patch state:

```bash
cargo test -p alethia-reth-block execute_derived_block_skips_invalid_nonce_transaction_and_records_committed_txs --features prover
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo test -p raiko2-stateless reconstruct_block_skips_invalid_nonce_transaction
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo test -p raiko2-stateless
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo test --manifest-path guests/common/Cargo.toml --test proposal_validation top_level_proposal_proof_reconstructs_block_and_skips_invalid_tx
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo test --manifest-path guests/common/Cargo.toml --test proposal_validation
```

Remaining before PR-ready commits:

- commit/publish the alethia helper branch
- replace local `/tmp` patches with the alethia branch URL or merged revision
- rerun broader checks once dependency pins are stable

### 2026-04-23 01:40 CST

Rebase execution status:

- `raiko2` branch rebased onto latest `origin/main`
- local derived-block patch reapplied after the rebase
- root and `guests/common` alethia dependencies now pin
  `63c2d001bc6c0485449c253ad35423cb5d1f0d2e`
- temporary `[patch."https://github.com/taikoxyz/alethia-reth"]` blocks removed

Testing adjustment:

- do not use the old
  `top_level_proposal_proof_reconstructs_block_and_skips_invalid_tx` filter anymore
- latest main changed the fixture into
  `top_level_proposal_proof_rejects_inline_one_pass_payload`
- top-level proposal proving now rejects inline payloads before reconstruction
- use `raiko2-stateless` for invalid-tx reconstruction acceptance and full `guests/common` tests
  for guest trust-boundary coverage

Commands run after rebase:

```bash
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo test -p raiko2-stateless reconstruct_block_skips_invalid_nonce_transaction
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo test -p raiko2-stateless
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo test --manifest-path guests/common/Cargo.toml --test proposal_validation
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo test --manifest-path guests/common/Cargo.toml
```

### 2026-04-23 10:20 CST

Deprecated test cleanup:

- deleted `one_pass_guest_input_with_skipped_invalid_tx`
- deleted `top_level_proposal_proof_rejects_inline_one_pass_payload`
- removed the guest-test-only direct call to
  `reconstruct_block_from_transactions_with_witness_resources`
- reran:

```bash
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo test -p raiko2-stateless
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo test --manifest-path guests/common/Cargo.toml --test proposal_validation
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo test --manifest-path guests/common/Cargo.toml
```
