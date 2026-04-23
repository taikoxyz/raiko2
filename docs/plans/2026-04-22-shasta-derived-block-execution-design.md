# Shasta Derived Block Execution Design

## Status

Proposed.

This design supersedes [2026-04-22-shasta-filtered-block-helper-design.md](./2026-04-22-shasta-filtered-block-helper-design.md).
That earlier design moved the existing provider-based one-pass reconstruction loop out of
`raiko2`, but it kept the wrong execution shape:

- builder-oriented reconstruction
- `WitnessStateProvider`
- `StateProviderDatabase`
- `builder.finish(state_provider, None)`

That path is now treated as an implementation mistake, even though parts of it were merged.

## Goal

Replace the current provider-based Shasta one-pass reconstruction path with an
`execute(derived_block)`-style path whose core execution logic lives in `alethia-reth`.

The target steady state is:

- `raiko2` derives a candidate `derived_block`
- `alethia-reth` executes that block in prover mode
- invalid non-anchor transactions are skipped during block execution
- `alethia-reth` exposes the committed transaction set and block-assembly helpers
- `raiko2` stays thin and does not own low-level replay/filter loops

## Non-Goals

- Replace the normal stateless canonical block validation path in `raiko2`
- Remove `WitnessDatabase`, which is still the correct primitive for normal witness-backed
  block replay
- Preserve the current provider-based one-pass helper shape

## Rejected Baseline

This work should **not** treat `0cc3a67 fix: rebuild shasta proposal blocks from derived txlist`
as the semantic base.

That merged path already bakes in the wrong abstraction boundary:

- proposal reconstruction is expressed as `anchor + txlist` builder replay
- `raiko2` must adapt witness state into a `StateProvider`
- the assembled filtered block comes from `builder.finish(...)`

For design purposes, the intended semantic baseline is the repo state before that merge:

- `raiko2`: treat `81baa9a` as the last clean mainline state before the provider-based one-pass
  path landed
- `alethia-reth`: use the latest rebased `main`, but do not preserve the provider-based one-pass
  API shape

In practice the implementation may still be developed on top of rebased latest branches, but it
should conceptually **replace** the `0cc3a67` direction rather than evolve it.

## Problem

The current merged one-pass path proves the right high-level statement:

- derive txs from blob-backed proposal data
- execute candidate transactions against witness-backed pre-state
- skip invalid non-anchor transactions
- compare the resulting block against the canonical L2 block

But it proves it through the wrong local abstraction:

- `raiko2` owns a proposal-only witness-to-provider adapter
- `raiko2` owns reconstruct-specific ancestor-hash glue
- `alethia-reth` only exposes a builder-level helper, not a block-level execution API

That means:

- `raiko2` remains coupled to low-level reconstruct plumbing
- the API does not resemble the normal block execution shape
- future `alethia-reth` upgrades will not naturally simplify `raiko2`

## Design Options

### Option 1: Keep The Provider-Based Helper And Thin It Further

This would keep the current `execute_and_filter_block_transactions(...)` helper and continue
passing a witness-backed `StateProvider` from `raiko2`.

Rejected because:

- it preserves the wrong abstraction boundary
- it keeps `WitnessStateProvider` alive in `raiko2`
- it does not converge toward the normal `execute(block)` model

### Option 2: Keep Reconstruction Local To `raiko2`

This would revert the upstream extraction and keep the entire reconstruct/filter/assemble path in
`raiko2`.

Rejected because:

- it repeats the original coupling problem
- it makes `raiko2` own more `reth`-level execution details than necessary
- it does not align with the goal of letting `alethia-reth` own Taiko execution semantics

### Option 3: Add A Block-Based Prover API In `alethia-reth`

This option treats proposal reconstruction as block execution, not provider-driven builder replay.

Recommended because:

- it matches the mental model the team wants: `execute(derived_block)`
- it lets invalid-tx skipping stay inside Taiko block execution semantics
- it removes proposal-only provider glue from `raiko2`
- it leaves `raiko2` with witness materialization and final comparison only

## Chosen Design

Add a new prover-only block-level API in `alethia-reth-block` that executes a candidate
`derived_block` and returns enough information for `raiko2` to assemble and compare the filtered
canonical block **without executing the EVM twice**.

The design is intentionally two-stage:

1. `alethia-reth` executes the candidate block and records which transactions were actually
   committed
2. `raiko2` computes the post-state root from its witness-backed trie and asks `alethia-reth`
   to assemble the filtered final block using the committed transactions plus execution output

This keeps the execution semantics in `alethia-reth` while keeping witness-specific state-root
computation in `raiko2`, where the sparse witness trie already lives.

## Target API Shape

The exact names can change, but the intended shape is:

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

Important properties of this shape:

- the caller passes a full candidate block, not `anchor + txlist + provider`
- execution is DB-based, like the normal stateless replay path
- assembly is still owned by `alethia-reth`
- `raiko2` does not own tx filtering or header/body assembly

## Alethia-Reth Changes

### 1. Extend Prover Execution To Record Committed Transactions

`TaikoBlockExecutor::execute_block(...)` already skips invalid non-anchor transactions under the
`prover` feature.

The new work is to also record which transactions were actually committed.

This can be implemented by tracking committed transaction indices or cloned signed transactions
during the existing execute loop and exposing them through the new `execute_derived_block(...)`
helper.

### 2. Reuse Normal Block Execution, Not Builder Replay

The new helper should execute the candidate block through the normal block executor path, not
through `BlockBuilder + finish(state_provider, None)`.

That means:

- no `StateProviderDatabase`
- no `WitnessStateProvider`
- no proposal-only builder loop

### 3. Reuse `TaikoBlockAssembler` For Final Block Assembly

`TaikoBlockAssembler` already knows how to build the final Taiko block header/body from:

- parent header / execution context
- committed transactions
- execution output
- final state root

The new assembly helper should reuse that code directly.

That keeps header construction in `alethia-reth`, where Taiko-specific block semantics already
live.

### 4. Remove The Current Provider-Based Helper

The current `filtered_block` helper should be removed or replaced.

The new prover API should not expose:

- `StateProvider`
- `StateProviderDatabase`
- `builder.finish(...)`
- reconstruct-specific provider plumbing

## Raiko2 Changes

### 1. Build A Candidate `derived_block`

`raiko2` should derive the expected Shasta tx sequence from manifest/blob data exactly as it does
today, then construct a candidate `derived_block`:

- canonical anchor transaction at index 0
- derived non-anchor txs after it
- parent linkage and next-block header fields derived from existing proposal inputs

### 2. Execute The Candidate Block With `WitnessDatabase`

`raiko2` should execute that `derived_block` by calling the new `alethia-reth` block-level prover
API against the existing witness-backed `WitnessDatabase`.

This keeps the existing stateless DB primitive and removes the proposal-only `StateProvider`
adapter.

### 3. Compute Post-State Root Locally

After execution, `raiko2` should continue using its sparse witness trie to compute the post-state
root from `hashed_state`.

This part remains local because the witness trie and witness materialization logic live in
`raiko2`.

### 4. Ask `alethia-reth` To Assemble The Filtered Block

Once `raiko2` has the final state root, it should call the new `assemble_filtered_block(...)`
helper to obtain the final filtered block.

That block is then compared against the canonical/expected block exactly once.

### 5. Remove Proposal-Only Provider Glue

If the new block-based API lands, the following proposal-only pieces should disappear from the
reconstruct path:

- `WitnessStateProvider`
- `compute_next_block_ancestor_hashes(...)`
- provider-based reconstruct plumbing added for `0cc3a67`

`WitnessDatabase` remains, because normal stateless block validation still uses it.

## Data Flow

The intended end-to-end Shasta proposal path becomes:

1. derive `expected_block.transactions` from blob-backed proposal sources
2. construct `derived_block = anchor + expected transactions`
3. materialize witness-backed sparse pre-state in `raiko2`
4. execute `derived_block` through the new `alethia-reth` prover API using `WitnessDatabase`
5. let prover-mode block execution skip invalid non-anchor transactions internally
6. return committed transactions + execution output + hashed post-state
7. compute post-state root in `raiko2`
8. assemble final filtered block in `alethia-reth`
9. compare filtered block against canonical block supplied by the L2 chain

This yields the statement the team wants:

- `blob -> txlist -> derived_block -> block execution -> invalid-tx filtering -> canonical block`

## Why This Is Better

- It matches the intended abstraction: execute a candidate block, do not locally simulate a block
  builder protocol.
- It keeps Taiko execution and final block assembly inside `alethia-reth`.
- It removes proposal-only provider adaptation from `raiko2`.
- It avoids double EVM execution.
- It preserves the existing witness-backed stateless validation primitives that are still correct
  for canonical block replay.

## Testing

The critical regressions remain:

- a candidate block whose txlist contains an invalid non-anchor transaction must still yield the
  canonical filtered block
- invalid-signature transactions must be skipped
- nonce/balance/gas-invalid transactions must be skipped by execution, not by a guest-local
  prefilter
- the assembled filtered block header must match the canonical block header

Target test coverage:

- `alethia-reth-block`
  - execute candidate block with invalid nonce tx and verify committed tx set
  - assemble filtered block from execution outcome and verify header/body fields
- `raiko2-stateless`
  - reconstruct candidate block through the new block-based API and verify invalid nonce skip
- `guests/common`
  - proposal-level regression: `top_level_proposal_proof_reconstructs_block_and_skips_invalid_tx`

## Rollout Plan

1. land the new block-based prover API in `alethia-reth`
2. switch `raiko2` proposal reconstruction to the new API
3. delete the provider-based reconstruct path introduced by `0cc3a67`
4. keep the normal `WitnessDatabase`-based canonical replay path intact
5. only then drop temporary compatibility glue and update dependency pins

## Open Constraint

The one part intentionally left in `raiko2` is witness-specific state-root computation.

That is acceptable because:

- it already exists
- it is not proposal-only logic
- it does not force `raiko2` to own Taiko execution semantics

If the witness trie ever moves into a shared dependency, that final local responsibility can also
be revisited later.

## Append Log

### 2026-04-22 17:00 CST

Initial design version written.

Locked decisions:

- treat `0cc3a67 fix: rebuild shasta proposal blocks from derived txlist` as the wrong semantic
  baseline
- do not continue evolving the provider-based one-pass helper
- converge on an `execute(derived_block)`-style API in `alethia-reth`
- keep witness-specific state-root computation in `raiko2` for now

Process rule for follow-up iterations:

- update this document in append-only mode
- record design corrections, implementation discoveries, and scope changes as new dated entries
- do not silently rewrite earlier rationale once implementation starts

### 2026-04-22 17:20 CST

Design review after re-inspecting the current merged code paths.

Concrete findings:

- `raiko2` currently still carries the wrong one-pass boundary introduced by `0cc3a67`
  (`fix: rebuild shasta proposal blocks from derived txlist`)
- the current helper path is still builder/provider based:
  - `WitnessStateProvider`
  - `StateProviderDatabase`
  - `execute_and_filter_block_transactions(...)`
  - `builder.finish(state_provider, None)`
- this confirms the current merged path is not a good semantic base for the next repair

Additional inspection results from `alethia-reth`:

- prover-mode `TaikoBlockExecutor::execute_block(...)` already skips invalid non-anchor
  transactions during normal block execution
- `TaikoBlockAssembler` already knows how to assemble a final Taiko block header/body from:
  - execution context
  - committed transactions
  - execution result
  - final state root

This strongly supports the `execute(derived_block)` direction:

- execution should be expressed as candidate block execution, not provider-backed builder replay
- final block assembly should remain in `alethia-reth`
- `raiko2` should not own reconstruct-specific `reth` execution plumbing

Boundary decision tightened in this review:

- move all proposal reconstruct execution semantics into `alethia-reth`
- keep only witness-specific materialization and state-root calculation in `raiko2`
- treat `WitnessStateProvider` as temporary wrong-path glue to be deleted
- keep `WitnessDatabase` for now, because it is still the right DB primitive for witness-backed
  block execution

Refined API intent:

- `alethia-reth` should expose a block-based prover API, not a txlist/provider API
- `raiko2` should pass:
  - `parent_header`
  - `derived_block`
  - witness-backed execution DB
- `alethia-reth` should return:
  - committed transactions or committed transaction indices
  - execution result
  - hashed post-state
- `alethia-reth` should also assemble the final filtered block from those artifacts once
  `raiko2` provides the computed state root

Refined minimization goal for `raiko2`:

- acceptable short-term local responsibilities:
  - derive `derived_block`
  - materialize sparse witness state
  - provide witness-backed `WitnessDatabase`
  - compute state root from hashed post-state
  - compare final filtered block with expected canonical block
- unacceptable long-term local responsibilities:
  - provider adaptation for proposal reconstruct
  - builder replay loops
  - committed-tx filtering semantics
  - final block/header assembly logic

Important nuance:

- this change will not immediately remove every direct `reth_*` type from `raiko2`
- current `raiko2` still directly uses `reth` block/tx types in pipeline/stateless/tests
- the concrete objective of this design is narrower and more realistic:
  remove `reth` execution-primitive ownership from the proposal reconstruct path

If further cleanup is desired later, a second phase can reduce direct `reth` type imports by:

- re-exporting canonical block/tx types through `alethia-reth`
- moving more stateless validation helpers behind `alethia-reth` facades

### 2026-04-22 18:05 CST

Implementation-planning pass after re-checking the live code on both sides.

Additional concrete findings:

- `raiko2` still imports the old provider-based helper directly in
  `crates/stateless/src/validation.rs`:
  - `alethia_reth_block::filtered_block::{FilteredBlockExecutionOutcome,
    execute_and_filter_block_transactions}`
  - `WitnessStateProvider::new(...)`
- `raiko2` still re-exports the provider-era outcome surface from
  `crates/stateless/src/lib.rs`
- `derive_expected_shasta_blocks(...)` in `guests/common/src/lib.rs` still returns
  `Result<Option<Vec<BlockManifest>>>`
  even though proposal proving now requires the expected block list to exist

Dependency-side findings:

- `alethia-reth` already exposes the primitives needed for the replacement design:
  - prover-mode `TaikoBlockExecutor::execute_block(...)`
  - `TaikoEvmConfig::{evm_env, context_for_block}`
  - `TaikoBlockAssembler`
- the current helper module `crates/block/src/filtered_block.rs` is still entirely
  `StateProvider`/builder based and should be replaced, not evolved

Rollout constraint recorded for implementation:

- during development, `raiko2` should temporarily use a workspace `[patch]` pointing to the local
  `../alethia-reth` checkout
- once the upstream helper merges, remove the local patch and repin the git `rev`

API-tightening note:

- proposal mode should fail before execution if the expected derived block list is absent
- the expected block sequence is part of the proving statement and should not be optional in
  `prove_shasta_proposal(...)`

### 2026-04-23 00:35 CST

Implementation update after wiring the local alethia helper.

The final helper boundary is slightly more explicit than the initial sketch:

- `execute_derived_block(...)` owns EVM execution and invalid non-anchor filtering
- `raiko2` owns witness state-root calculation because the sparse witness trie remains local
- `assemble_filtered_block(...)` owns final block/header assembly after `raiko2` supplies the
  state root

The helper returns committed transactions as recovered transactions rather than bare signed
transactions. This avoids losing signer information between execution and final block assembly and
keeps `raiko2` from re-running signer recovery for already accepted transactions.

The helper also returns finalized block zk gas. Assembly must install this value into the next-block
execution context before invoking `TaikoBlockAssembler`; otherwise the assembled header can diverge
from the execution that selected the committed transactions.

Context selection requirement:

- execute and assemble with `next_evm_env(...)` plus `context_for_next_block(...)`
- do not use `context_for_block(...)` for the derived candidate
- the derived candidate is not yet canonical, and using finalized-block context can make Uzen
  difficulty validation depend on candidate header fields before reconstruction has proven them

Repository-topology note:

- root `Cargo.toml` and `guests/common/Cargo.toml` both need the temporary alethia patch while
  testing locally
- `guests/common` is an excluded independent workspace, so the root patch does not apply to its
  focused guest tests
- after alethia merges, both dependency locations need to be repinned to the merged revision and
  the local patches removed

Current local verification:

- alethia helper focused test passes
- `raiko2-stateless` invalid nonce reconstruction test passes
- full `raiko2-stateless` crate test suite passes
- proposal guest invalid tx reconstruction test passes
- full `guests/common` proposal validation integration test passes

### 2026-04-23 01:40 CST

Rebase update after latest `raiko2` main.

Dependency decision:

- keep the alethia dependency pinned by commit, not branch
- current temporary rev is `63c2d001bc6c0485449c253ad35423cb5d1f0d2e`
- this preserves reproducibility while alethia PR review is in progress
- after alethia merges, replace this rev with the merge commit on alethia `main`

Latest main changed the top-level guest test strategy:

- proposal mode rejects inline payloads before execution
- therefore the old inline one-pass top-level acceptance fixture is no longer valid
- derived txlist invalid-tx filtering is covered at the stateless reconstruction layer
- guest-level tests now verify that inline one-pass payloads are rejected by the trust boundary

Post-rebase verification:

- root dependency lock resolves alethia packages from the pinned git rev
- `guests/common` dependency lock resolves alethia packages from the same pinned git rev
- `raiko2-stateless` tests pass
- full `guests/common` tests pass

### 2026-04-23 10:20 CST

Cleanup after the latest main trust-boundary changes:

- removed the deprecated inline one-pass proposal fixture from `proposal_validation.rs`
- removed the duplicated top-level inline one-pass rejection test
- kept the smaller inline-payload rejection tests that directly exercise the current trust
  boundary
- kept invalid transaction filtering coverage in `raiko2-stateless`, where derived txlist
  reconstruction is still exercised directly
