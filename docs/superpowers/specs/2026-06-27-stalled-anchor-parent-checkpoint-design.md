# Stalled Anchor Parent Checkpoint Design

## Context

Raiko2 currently accepts a Shasta stalled-anchor compatibility path where
`taiko.l1_ancestor_headers` may be empty when every block repeats the parent
anchor number and that anchor is older than the configured anchor offset. This
preserves compatibility with historical stalled derivation, but it leaves the
repeated `anchorV4` calldata tuple under-authenticated: the guest proves the
anchor number is the parent Anchor storage number, but it does not authenticate
the repeated `blockHash` and `stateRoot`.

The Go `taiko-client` prover request already sends `last_anchor_block_number`.
For this fix, the external request schema remains unchanged. Raiko2 will derive
and verify the complete parent anchor checkpoint from parent L2 state.

## Goals

- Keep the `POST /v3/proof/batch/shasta` request body unchanged.
- Avoid fetching an unbounded L1 header range for long stalled periods.
- Authenticate the complete repeated anchor tuple in the guest:
  `(number, blockHash, stateRoot)`.
- Preserve the existing L1-header-chain validation for advancing anchors.
- Fail closed when the parent checkpoint cannot be proven from parent L2 state.

## Non-Goals

- Do not modify L1 or L2 protocol contracts.
- Do not add `last_anchor_checkpoint` or `last_anchor_header` to the Go
  `taiko-client` request body.
- Do not trust host-supplied hash/root values unless the guest can authenticate
  them against the parent state root.
- Do not replace normal advancing-anchor validation with parent checkpoint
  lookup.

## Selected Approach

Use the parent L2 state as the source of truth for the stalled parent anchor
checkpoint.

The guest already reads `Anchor._blockState.anchorBlockNumber` from the parent
state to verify `prover_data.last_anchor_block_number`. The fix extends this
same parent-state authentication boundary: when the proposal is in the stalled
anchor compatibility path, raiko2 also provides storage proof material for the
L2 `SignalService` checkpoint record at the parent anchor number.

The guest then reads:

- `Anchor._blockState.anchorBlockNumber`, proving the parent anchor number.
- `SignalService._checkpoints[parentAnchorNumber].blockHash`.
- `SignalService._checkpoints[parentAnchorNumber].stateRoot`.

For stalled anchors, every decoded `anchorV4` calldata checkpoint must equal
that complete parent checkpoint tuple exactly. Number-only equality is not
sufficient.

## Data Flow

1. Go `taiko-client` sends the existing request with `last_anchor_block_number`.
2. Raiko2 preflight builds the Shasta manifest and detects whether all anchor
   transactions are in the stalled compatibility shape.
3. When stalled, the provider supplements the first block witness with parent
   L2 storage proof nodes for the SignalService checkpoint slots matching
   `last_anchor_block_number`.
4. Guest validation reads the parent Anchor storage slot and confirms the
   request hint equals the real parent anchor number.
5. Guest validation reads the SignalService checkpoint slots from the same
   parent state root.
6. Guest validation requires every repeated `anchorV4` checkpoint calldata to
   match the authenticated parent tuple.
7. Advancing anchors continue to require the normal L1 ancestor header chain.

## Components

### Chain Metadata

Raiko2 needs an authenticated source for the L2 SignalService address. The
address must not be supplied as an unauthenticated request parameter.

The implementation should expose the L2 SignalService address through chain
metadata, for example as `l2_signal_service`. Built-in Taiko chain specs should
populate this from the known predeploy convention, while custom specs must set
it explicitly. If the address is absent, stalled-anchor proofs fail closed.

### Provider Witness Supplement

The provider should add targeted parent-state proof material only when needed.
For a stalled proposal, compute the SignalService checkpoint storage slots:

- Mapping base slot: `254` for `SignalService._checkpoints`.
- Mapping entry slot:
  `keccak256(abi.encode(uint48(parentAnchorNumber), uint256(254)))`.
- `blockHash` slot: entry slot plus `0`.
- `stateRoot` slot: entry slot plus `1`.

Only these slots are needed for the first witness parent state. This keeps input
growth bounded to a small storage proof instead of a header range proportional
to `origin - last_anchor`.

### Guest Validation

Guest validation should return an authenticated parent anchor checkpoint:

```text
ParentAnchorCheckpoint {
  number,
  block_hash,
  state_root,
}
```

The existing parent Anchor number check remains mandatory. Stalled-anchor
validation uses the full checkpoint:

- If `l1_ancestor_headers` is empty and the stalled predicate is true, require
  all decoded anchor checkpoints to equal the authenticated parent checkpoint.
- If the predicate is false, empty `l1_ancestor_headers` remains invalid.
- If an anchor advances, keep the current header-chain path and require
  checkpoint hash/root equality against L1 headers.

### API And Request Handling

No public request field changes are required. `last_anchor_block_number` remains
a request hint and is validated against parent Anchor storage.

Docs should clarify that `last_anchor_block_number` is not trusted as complete
anchor data. Raiko2 authenticates hash/root internally when stalled anchor
compatibility is used.

## Failure Behavior

The fix is fail-closed:

- Missing SignalService address for a stalled proof fails.
- Missing SignalService account proof or storage proof fails.
- Missing checkpoint record fails.
- Zero `blockHash` or zero `stateRoot` fails.
- Request hint mismatch against parent Anchor storage fails.
- Any repeated `anchorV4` tuple mismatch fails.
- Advancing anchors with empty L1 ancestor headers fail.

## Tests

Add focused tests for the new behavior:

- Guest accepts a stalled repeated anchor whose calldata tuple matches the
  authenticated parent SignalService checkpoint.
- Guest rejects a stalled repeated anchor with the same number but altered
  `blockHash`.
- Guest rejects a stalled repeated anchor with the same number but altered
  `stateRoot`.
- Guest rejects stalled validation when the SignalService checkpoint proof is
  missing.
- Guest rejects an empty or zero-valued SignalService checkpoint.
- Guest still rejects advancing anchors with empty `l1_ancestor_headers`.
- Pipeline/provider supplements only the targeted SignalService checkpoint
  storage proof for stalled proposals.
- Existing non-stalled L1 header-chain validation tests continue to pass.

## Rollout Notes

This is a guest soundness change. After implementation, rebuild the affected
guest programs and run the focused Shasta guest-common and pipeline tests first.
Then run the repository's relevant Rust checks for shared type and pipeline
changes. Public API compatibility is preserved because the Go request body is
unchanged.
