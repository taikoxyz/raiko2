# Shasta Fixture

`shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json` is a checked-in Shasta
`GuestInput` fixture used by:

- `bin/raiko2/src/server/fixture.rs`
- `crates/primitives-shasta/tests/guest_input_bincode_roundtrip.rs`
- `docs/development.md`

It replaces the old untracked repo-root `test.json` workflow.

`tests/fixtures/remote_prover/` contains the canonical remote prover protocol goldens owned by
`raiko2`:

- `shasta_request_v1_taiko_mainnet_proposal_2222_l2_5412225_5412416.json`
- `shasta_aggregate_request_v1_single_fixture_proof.json`

These files define the strict `raiko2`-owned request contract for:

- `raiko2-shasta-request-v1`
- `raiko2-shasta-aggregate-request-v1`

Remote prover implementations are expected to accept byte-for-byte equivalent payloads for these
fixtures. Provider-specific repositories may keep their own copies, but `raiko2` owns the canonical
request goldens and validates them in `crates/prover/tests/remote_prover_fixture.rs`.
These replay-packet fixtures target `gaiko2`/`sgxgeth`-style providers. They are not valid
`raiko2-sgx-prover` proposal proof requests, because `raiko2-sgx-prover` requires the full
`GuestInput` envelope and reruns Shasta guest validation before signing.

## Provenance

This fixture was generated on 2026-04-13 from a real Taiko mainnet preflight using:

- `proposal_id = 2222`
- `l2_start = 5412225`
- `l2_end = 5412416`
- `l1_inclusion_block_number = 24862953`
- `last_anchor_block_number = 24862885`
- `l2_chain_id = 167000`

## Regenerate

```bash
target/debug/preflight \
  --rpc-url http://l2-rpc.example.com:8545 \
  --l1-rpc-url https://ethereum-rpc.publicnode.com \
  --l2-chain-id 167000 \
  --l1-chain-id 1 \
  --proposal-id 2222 \
  --l1-inclusion-block-number 24862953 \
  --last-anchor-block-number 24862885 \
  --l2-start 5412225 \
  --l2-end 5412416 \
  --proof-type native \
  --output tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json
```
