# Proposal GuestInput Fixture

The `shasta` in this fixture's filename is a frozen identifier, not a fork selector. It is the
current proposal fixture and exercises Unzen proving. See the `Frozen identifier` entry in
[../../CONTEXT.md](../../CONTEXT.md).

`shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json` is a checked-in
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

## Provenance

This fixture was generated on 2026-07-22 from a real Taiko mainnet preflight using:

- `proposal_id = 23077`
- `l2_start = 9051439`
- `l2_end = 9051630`
- `l1_inclusion_block_number = 25585003`
- `last_anchor_block_number = 25584933`
- `l2_chain_id = 167000`

The generated input passed current native guest-input validation and includes the blob commitments
and proofs required to verify blob-backed proposal data.

## Regenerate

```bash
target/debug/preflight \
  --rpc-url http://l2-rpc.example.com:8545 \
  --l1-rpc-url https://ethereum-rpc.publicnode.com \
  --l2-chain-id 167000 \
  --l1-chain-id 1 \
  --proposal-id 23077 \
  --l1-inclusion-block-number 25585003 \
  --last-anchor-block-number 25584933 \
  --l2-start 9051439 \
  --l2-end 9051630 \
  --proof-type native \
  --validate \
  --pretty \
  --output tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json
```
