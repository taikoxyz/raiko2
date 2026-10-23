# Proposal GuestInput Fixture

The `shasta` in this fixture's filename is a frozen identifier, not a fork selector. See the
`Frozen identifier` entry in [../../CONTEXT.md](../../CONTEXT.md).

The fixture contains real post-Unzen mainnet blocks `12556056`-`12556247`. Proposal timestamp
`1791755639` is after mainnet's Unzen activation at `1786021200`. Raiko2 proposal validation,
proving, and zkGas quoting require both the proposal and every witness block to be Unzen or later.

`shasta_guest_input_taiko_mainnet_proposal_41455_l2_12556056_12556247.json` is a checked-in
`GuestInput` fixture used by:

- `bin/raiko2/src/server/fixture.rs`
- `crates/primitives-shasta/tests/guest_input_bincode_roundtrip.rs`
- `docs/development.md`

It replaces the old untracked repo-root `test.json` workflow.

`tests/fixtures/remote_prover/` contains remote prover protocol fixtures owned by `raiko2`:

- `shasta_aggregate_request_v1_single_fixture_proof.json`

The aggregate fixture defines the strict `raiko2`-owned request contract for:

- `raiko2-shasta-aggregate-request-v1`

Current gaiko2 proposal requests are generated from the shared `GuestInput` fixture as
`raiko2-shasta-request-v1` packets with `payload.guest_input`. Provider-specific repositories may
keep their own copies, but `raiko2` owns the adapter behavior and validates it in
`crates/prover/tests/remote_prover_fixture.rs`.

`risc0-zkgas/<version>/` contains compact, generated calibration inputs for the production RISC0
zkGas estimator. Its config, fit rows, and validation rows are consumed by
`scripts/modeling/risc0_zkgas_model.py` and the prover regression tests. Regenerate this subtree with
the repo-local calibration skill; do not hand-edit fixture observations or audit hashes.

## Provenance

This fixture was generated from a real post-Unzen Taiko mainnet preflight using:

- `proposal_id = 41455`
- `l2_start = 12556056`
- `l2_end = 12556247`
- `l1_inclusion_block_number = 26172029`
- `last_anchor_block_number = 26171960`
- `proposal_timestamp = 1791755639`
- `l2_chain_id = 167000`

The input includes the blob commitments and proofs required to verify blob-backed proposal data.
The checked-in JSON is copied byte-for-byte from the retained preflight input, with SHA-256
`20c76ab78f7bde77612f28d82f734912247ee905fd682de0568d9013053cc575`.

## Regenerate

```bash
target/debug/preflight \
  --rpc-url http://l2-rpc.example.com:8545 \
  --l1-rpc-url https://ethereum-rpc.publicnode.com \
  --l2-chain-id 167000 \
  --l1-chain-id 1 \
  --proposal-id 41455 \
  --l1-inclusion-block-number 26172029 \
  --last-anchor-block-number 26171960 \
  --l2-start 12556056 \
  --l2-end 12556247 \
  --proof-type native \
  --validate \
  --output tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_41455_l2_12556056_12556247.json
```
