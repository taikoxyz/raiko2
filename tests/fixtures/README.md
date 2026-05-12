# Shasta Fixture

`shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json` is a checked-in Shasta
`GuestInput` fixture used by:

- `bin/raiko2/src/server/fixture.rs`
- `crates/primitives-shasta/tests/guest_input_bincode_roundtrip.rs`
- `docs/development.md`

It replaces the old untracked repo-root `test.json` workflow.

`shasta_remote_request_fixture_chain_167013_block_42.json` is a checked-in minimal remote prover
request fixture used by:

- `crates/sgx-runtime/tests/dump_valid_request.rs`
- manual `curl` smoke tests against `raiko2-sgx-prover`
- devops startup/link testing for the `sgx/remote` lane

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

Generate the SGX remote request fixture from the integration test:

```bash
cargo test -p raiko2-sgx-runtime --test dump_valid_request dump_valid_request_json -- --nocapture
```
