# Shasta Zk Path Does Not Rebuild Block From Derived Txlist

## Summary

The current Shasta zk proposal path in `raiko2` does include blob sidecars in `GuestInput`, and the
guest does execute a complete `blob -> manifest -> txlist` derivation.

However, the proof path stops short of the older `raiko` semantics.

Today the guest:

- verifies blob usage and blob proofs
- derives the expected transaction list from blob-backed data sources
- checks that the derived transactions exactly match the canonical remote block body
- replays the canonical remote block against the witness-backed EVM database

The important boundary is that the derived txlist is **not** what gets fed into the executor.
Only the canonical remote block body is executed.

It does **not** rebuild the executed block from the derived txlist and then prove that invalid
transactions are filtered during execution to yield the canonical block/state transition.

That means the zk proof currently establishes:

- `blob -> txlist` is correct
- the canonical block body matches that txlist
- the canonical block replays against the supplied witness DB

It does **not** establish the stronger statement:

- `blob -> txlist -> invalid-tx filtering -> final executed block body/state transition`

## Evidence

### Blob Data Is Present In GuestInput

`GuestInput` embeds `taiko: TaikoManifest`, and `TaikoManifest` embeds `data_sources`, each of
which can carry:

- `tx_data_from_blob`
- `blob_commitments`
- `blob_proofs`

Relevant code:

- `crates/primitives-shasta/src/input.rs:17`
- `crates/protocol/src/manifest.rs:38`
- `crates/protocol/src/manifest.rs:75`

### Preflight Hydrates Blob Sidecars For Zk Paths

When Shasta proposal sources exist and the manifest has no `data_sources`, preflight fetches beacon
blob sidecars and stores raw blob bytes plus commitments/proofs into the manifest.

Relevant code:

- `crates/pipeline/src/forks/shasta/spec.rs:147`
- `crates/provider/src/network/blobs.rs:125`

For `sp1` and `risc0`, the manifest resolves `blob_proof_type` to
`BlobProofType::ProofOfEquivalence`.

Relevant code:

- `crates/pipeline/src/forks/shasta/manifest.rs:105`

### Guest Executes The Full Blob -> Txlist Derivation

The proposal guest calls `prove_shasta_proposal`, which first verifies proposal-mode blob usage and
then derives expected Shasta blocks from the manifest data sources.

Relevant code:

- `guests/sp1/src/shasta_proposal.rs:13`
- `guests/common/src/lib.rs:517`
- `crates/primitives-shasta/src/blob.rs:32`
- `guests/common/src/lib.rs:303`

For blob-backed sources, derivation decodes `tx_data_from_blob` through `BlobCoder::decode_blobs`,
then decompresses and decodes the manifest into `Vec<TxEnvelope>`.

Relevant code:

- `crates/protocol-shasta/src/shasta/derivation.rs:294`
- `crates/protocol-shasta/src/shasta/manifest.rs:55`

### The Guest Then Requires Exact Equality With The Remote Block Body

After derivation, the guest validates that the canonical block body has exactly:

- one anchor tx at index 0
- then the same transaction count as the derived manifest
- then byte-for-byte equal RLP for every derived tx vs canonical tx

Relevant code:

- `guests/common/src/lib.rs:441`
- `guests/common/src/lib.rs:491`

This is a direct equality check, not a reconstruction path that starts from the derived txlist and
lets execution decide which non-anchor transactions survive.

As a consequence, invalid-transaction conditions on the **derived txlist itself** are not evaluated
here via signer recovery or transaction/state validation. The guest only proves that the derived
transactions serialize to the same bytes as the canonical remote block transactions.

### Stateless Replay Still Executes The Canonical Remote Block

The zk path eventually calls `validate_block_with_witness_resources`, passing the existing
`stateless_input.block.clone()` into stateless execution.

Relevant code:

- `guests/common/src/lib.rs:523`
- `crates/stateless/src/validation.rs:87`
- `crates/stateless/src/validation.rs:149`

The witness DB serves account/storage/code/blockhash reads, but it does not replace the block body
with the derived txlist.

Relevant code:

- `crates/stateless/src/witness_db.rs:30`
- `crates/stateless/src/witness_db.rs:88`

This means signer recovery and EVM transaction/state validation do occur, but only for the
canonical block transactions that are already present in `stateless_input.block`.
They do not independently run over the blob-derived txlist as a candidate block body.

## Why This Matters

This is weaker than the old `raiko` proving semantics for txlist-driven execution.

The current proof depends on the canonical remote block body as an explicit proof input and proves
that it matches the derived txlist. It does not prove that the canonical executed transaction set is
the result of running the derived txlist through the block-construction / invalid-transaction
filtering rules.

In particular, this leaves the following question outside the zk statement:

- if the blob-derived txlist contains transactions that should be skipped during execution
  (for example nonce-too-low or other invalid non-anchor cases), do the builder/filtering rules
  deterministically produce the canonical block body?

That relationship is not currently proven by the zk guest path.

## Related Observation

`NativeProver` does not execute the guest path at all. It deserializes `GuestInput`, reads
`proof_carry_data`, and returns a signed mock proof envelope.

Relevant code:

- `crates/prover/src/native.rs:47`

This is expected for the native lane, but it should be documented separately as a non-verifying
path when reasoning about txlist semantics.

## Impact

- the zk proof currently proves txlist consistency with the canonical block, not txlist-driven block
  reconstruction
- invalid-tx handling semantics are not independently proven in the zk path
- the proof statement is weaker than the old `raiko` model the team expects for txlist-backed
  execution

## Most Direct Repair Path

The most direct way to recover the old semantics in `raiko2` is to insert a txlist-driven block
reconstruction step inside the Shasta proposal guest:

- after `derive_expected_shasta_blocks`
- before `validate_shasta_manifest_block` / `validate_block_with_witness_resources`

At that point the guest already has:

- the parent header
- the derived `Vec<TxEnvelope>` for the candidate block body
- the witness-backed pre-state and chain config needed for EVM execution

The reusable dependency surface that best matches this job is the Reth `BlockBuilder` path:

- `ConfigureEvm::builder_for_next_block` creates a builder from parent header plus next-block
  attributes
- `BasicBlockBuilder` records transactions only when they are actually committed
- `finish()` assembles a complete block from the committed transaction set and resulting execution
  output

Relevant code:

- `~/.cargo/git/checkouts/reth-e231042ee7db3fb7/d6324d6/crates/evm/evm/src/lib.rs:352`
- `~/.cargo/git/checkouts/reth-e231042ee7db3fb7/d6324d6/crates/evm/evm/src/execute.rs:315`
- `vendor/alethia-reth/crates/block/src/assembler.rs:53`

In practice the flow would look like:

1. recover each derived `TxEnvelope`
2. skip unrecoverable transactions up front, exactly like the old payload-builder path already
   does for invalid signatures
3. build a candidate block using the witness-backed DB and the same Taiko EVM config
4. feed the recovered transactions into the builder in order
5. when execution returns invalid non-anchor transaction errors, skip that transaction instead of
   aborting the whole candidate block
6. finish the builder and compare the generated block body against the canonical/expected block
   body
7. if desired, additionally compare header fields and final state root

There is already precedent for both halves of this behavior in vendored dependencies:

- tx decoding + signer recovery + skip-invalid-signature:
  `vendor/alethia-reth/crates/primitives/src/payload/builder.rs:98`
- skip-invalid execution errors while iterating candidate transactions:
  `vendor/alethia-reth/crates/block/src/tx_selection/mod.rs:233`

This is important because it preserves the right semantic boundary:

- invalid signature can be filtered before EVM
- nonce/balance/gas-availability checks happen when the tx is actually tested against the evolving
  candidate block state

That second category should not be modeled as a simple stateless prefilter if the goal is to match
the real block-construction semantics.

## Possible Thinner Repair Path

There is also a potentially thinner alternative worth keeping in mind.

`alethia-reth`'s Taiko executor already changes `execute_block(...)` under the `prover` feature so
that invalid non-anchor transactions are skipped instead of aborting block replay. That means a
future repair may be able to reuse `execute(block)` more directly, rather than wrapping
`execute_transaction(...)` in a guest-local `BlockBuilder` loop.

In principle that thinner path would look like:

1. construct a candidate block from `anchor + derived txlist`
2. feed that block into `execute(block)`
3. let the prover-mode executor skip invalid non-anchor transactions during replay
4. compare the resulting committed/final transaction set and header against the canonical block

However, the current API surface does not make this directly usable today:

- the existing `raiko2` stateless validation path validates the input block header/body before
  execution, so a naive "swap in the derived txlist and keep the old path" is not enough
- `Executor::execute(...)` returns execution results and state, but not an assembled post-filter
  block
- the filtered transaction set, final body, and final header are not exposed back to the guest

So this issue should be understood as:

- the executor semantics may already be close enough to support the old proving model
- but `raiko2` still needs either:
  - a local reconstruction path that assembles the filtered final block, or
  - a new/revised `alethia-reth` API that exposes the committed transaction set or final block

If `execute(block)` eventually returns committed transaction indices or an assembled filtered block,
that may be a cleaner long-term replacement for the guest-local builder wrapper.

## Chosen Direction

The preferred repair is now to keep the current proving semantics, but move the heavy
`BlockBuilder`-driven replay/filtering loop behind a small Taiko-specific helper in
`alethia-reth-block`.

The key observation is that `raiko2` does **not** need `alethia-reth` to expose every low-level
`reth` trait or builder primitive. It only needs a helper that:

- accepts the parent header / next-block env
- accepts the canonical anchor tx plus the derived non-anchor txlist
- accepts a caller-provided state provider for the witness-backed pre-state
- returns the filtered/rebuilt block together with the execution artifacts needed for post-state
  validation

In other words, `raiko2` should keep owning witness materialization, while `alethia-reth` should
own the Taiko-specific "execute candidate transactions, skip invalid non-anchor txs, and assemble
the filtered block" logic.

An intentionally thin target surface looks like:

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
    anchor_tx: Option<Recovered<TransactionSigned>>,
    transactions: Vec<TransactionSigned>,
    state_provider: P,
) -> Result<FilteredBlockExecutionOutcome, BlockExecutionError>
where
    P: StateProvider + Clone;
```

This is "thin" in the right place:

- `raiko2` still constructs `WitnessStateProvider` from witness materialization
- `alethia-reth` hides `BlockBuilder`, `builder_for_next_block`, and skip-invalid replay details
- the proposal guest continues to compare the resulting filtered block against the canonical block
  supplied by the L2 chain

## Complexity Assessment

This extraction should be moderate, not large.

The tricky proving semantics are already implemented today in `raiko2`:

- execute anchor first
- skip bad non-anchor signatures before EVM
- skip nonce/balance/gas invalid non-anchor transactions during replay
- assemble a filtered block from committed transactions

So the main work is not inventing new semantics. The main work is:

1. carving out the replay/filter/assemble loop into `alethia-reth-block`
2. defining a helper return type that hides `reth`'s raw `BlockBuilderOutcome`
3. updating `raiko2` to call the new helper instead of owning the builder loop directly
4. bumping or patching the `alethia-reth-block` dependency once the helper exists upstream

The only real integration caveat is repository topology: this repo currently consumes
`alethia-reth-block` via a pinned git dependency, so the final extraction is a coordinated
dependency-side change, not a `raiko2`-only patch.

## Staged Rollout

To keep reviewable diffs small, this should be landed in two layers:

1. inside `raiko2`, narrow the local stateless API so callers no longer depend on raw
   `BlockBuilderOutcome`
2. once the local API shape is stable, extract the internal builder loop into
   `alethia-reth-block` and switch the implementation over

That staging preserves the current one-pass fix while making the final upstream extraction mostly a
mechanical swap.

## Next Investigation

- narrow `raiko2-stateless`'s exported reconstruction result to a helper-owned outcome type
- keep proposal proving dependent on `filtered_block == expected_block`
- extract the replay/filter/assemble loop into `alethia-reth-block`
- wire `raiko2` to the new helper and drop direct `BlockBuilder` coupling from the guest path
- file a follow-up issue documenting that `native` is not a semantic verifier for blob/txlist/EVM
  behavior
