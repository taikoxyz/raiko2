# Shasta Filtered Block Helper Design

## Goal

Preserve the current one-pass proving semantics for Shasta proposal blocks, but move the heavy
Taiko-specific replay/filter/assemble loop out of `raiko2` and behind a thin helper in
`alethia-reth-block`.

## Problem

The current fix in `raiko2` proves the right statement:

- derive `txlist` from blob-backed proposal data
- replay `anchor + derived txlist`
- skip invalid non-anchor transactions
- assemble the filtered block
- prove the filtered block equals the canonical L2 block

But the current implementation owns the `BlockBuilder` loop directly inside
`raiko2-stateless::validation`, which keeps `raiko2` coupled to low-level `reth` execution and
block-building primitives.

## Chosen Approach

Keep witness materialization in `raiko2`, but move the block replay/filter/assemble logic into a
Taiko-specific helper in `alethia-reth-block`.

`raiko2` should keep doing:

- witness trie materialization
- `WitnessStateProvider` construction
- canonical block / expected block loading
- final `filtered_block == expected_block` assertion

`alethia-reth-block` should do:

- create the next-block builder from parent header + `TaikoNextBlockEnvAttributes`
- execute the anchor transaction first
- iterate candidate non-anchor transactions in order
- skip unrecoverable or invalid non-anchor transactions
- finish block assembly and return the filtered block plus execution artifacts

## Target Interface

The intended helper surface is:

```rust
pub struct FilteredBlockExecutionOutcome {
    pub filtered_block: RecoveredBlock<Block>,
    pub execution_result: BlockExecutionResult<Receipt>,
    pub hashed_state: HashedPostState,
    pub trie_updates: TrieUpdates,
}

pub fn execute_and_filter_block_transactions<P>(
    evm_config: &TaikoEvmConfig,
    parent_header: &SealedHeader,
    block_env: TaikoNextBlockEnvAttributes,
    anchor_tx: Recovered<TransactionSigned>,
    transactions: Vec<TransactionSigned>,
    state_provider: P,
) -> Result<FilteredBlockExecutionOutcome, BlockExecutionError>
where
    P: StateProvider + Clone;
```

This is intentionally narrower than exposing `BlockBuilder` or raw `BlockBuilderOutcome` to
`raiko2`.

## Data Flow

1. The guest derives `expected_block.transactions` from the Shasta manifest.
2. `raiko2-stateless` materializes the sparse witness-backed pre-state.
3. `raiko2-stateless` creates a `WitnessStateProvider`.
4. `alethia-reth-block` executes `anchor + derived txlist` against that provider.
5. Invalid non-anchor transactions are skipped, committed transactions are assembled into
   `filtered_block`.
6. The guest compares `filtered_block` against the canonical L2 block supplied in
   `stateless_input.block`.
7. The returned execution/state artifacts remain available for post-state validation.

## Why This Is Better

- It keeps the proving statement unchanged.
- It keeps witness materialization in the repo that already owns it.
- It removes direct `BlockBuilder` coupling from the guest path.
- It matches the old `raiko` proof shape more closely: candidate txlist goes in, canonical block
  comes out only if invalid tx filtering behaves correctly.

## Rollout Plan

Because `raiko2` currently depends on `alethia-reth-block` via a pinned git dependency, the
extraction should be staged:

1. narrow the local `raiko2-stateless` API to a helper-owned outcome type that mirrors the target
   `alethia-reth` helper result
2. keep existing regression coverage green while preserving one-pass semantics
3. extract the internal builder loop into `alethia-reth-block`
4. switch `raiko2` to the upstream helper and remove the local low-level builder coupling

## Testing

The critical regression remains the same:

- a derived txlist containing an invalid non-anchor transaction must still reconstruct the canonical
  block by skipping that transaction

The existing proposal-level and stateless invalid-nonce tests should continue to guard this
behavior across both the local seam refactor and the later upstream extraction.
