# Hoodi Tx-List Witness Rollout

This runbook covers the Hoodi rollout path for Shasta proposal proving with the
`debug_executionWitnessForTxList` witness API.

The goal is to make `raiko2` prove the derived Shasta transaction list instead of depending on
canonical block witnesses when the derived tx list contains recoverable invalid or non-canonical
suffix transactions.

## Components

- `alethia-reth` L2 node with `debug_executionWitnessForTxList`.
- Proof-history storage enabled and covering the target proposal block range.
- `raiko2` image built from #70 or a later commit that requests tx-list witnesses for Shasta source
  manifests.
- Remote SGX providers:
  - `raiko2-sgx` for `proof_type = "sgx"`.
  - `gaiko2-sgxgeth` for `proof_type = "sgxgeth"`.

## Required Runtime Config

Use a Reth L2 endpoint for Hoodi preflight and witness generation. If normal L2 RPC and witness
RPC are split, the witness RPC must point to the `alethia-reth` node with the new debug API.

```toml
[[rpc.pairs]]
network = "taiko_hoodi"
l1_network = "hoodi"
l1_rpc = "<hoodi-l1-rpc>"
l2_rpc = "<hoodi-l2-reth-rpc>"
l2_witness_rpc = "<hoodi-l2-reth-witness-rpc>"
l2_provider = "reth"

[prover.sgx]
enabled = true
base_url = "<raiko2-sgx-http>"
timeout_ms = 300000

[prover.sgxgeth]
enabled = true
base_url = "<gaiko2-sgxgeth-http>"
timeout_ms = 300000
```

Each lane is enabled independently. Its endpoint and timeout live in the same table, so enabling
one lane never selects or configures the other.

Do not route `l2_witness_rpc` to a node that only supports canonical `debug_executionWitness`.
The Shasta source-manifest path must use `debug_executionWitnessForTxList`.

## Alethia-Reth Readiness

Before deploying `raiko2`, confirm the witness node is synced and its proof-history window covers
the proposal range to be proved.

Check the proof-history metrics:

```bash
curl -fsS <reth-metrics-url>/metrics \
  | grep -E 'reth_optimism_trie_block_(earliest|latest)_number|reth_blockchain_tree_in_mem_state_latest_block'
```

The required invariant is:

- `reth_optimism_trie_block_earliest_number <= first_l2_block_number`
- `reth_optimism_trie_block_latest_number >= last_l2_block_number`

For a quick RPC smoke test, call the tx-list witness API with an empty RLP tx list:

```bash
cast rpc debug_executionWitnessForTxList <block-number> 0xc0 \
  --rpc-url <hoodi-l2-reth-witness-rpc>
```

This should return an execution witness object with `state`, `headers`, `codes`, and `keys`.

## Raiko2 Smoke Test

After deploying `raiko2`, check server and remote-provider health:

```bash
curl -fsS <raiko2-url>/ready
curl -fsS <raiko2-sgx-url>/health
curl -fsS <gaiko2-sgxgeth-url>/healthz
```

Submit one recent Hoodi proposal with each proof type:

- `proof_type = "sgx"`
- `proof_type = "sgxgeth"`

The `raiko2` logs should include:

- `shasta tx-list witnesses ready`
- `fetched witnesses via debug_executionWitnessForTxList`

The witness count must match the derived manifest block count.

## Regression Gate

Before production rollout, run a recent-proposal regression over both SGX lanes.

Recommended gate:

- range: latest 500 Hoodi proposals, or the requested production canary range
- lanes: `sgx` and `sgxgeth`
- stop on the first terminal failure
- no `witness count ... does not match derived manifest block count`
- no `debug_executionWitnessForTxList failed`
- no remote-provider timeout or proof verification failure

The regression is only valid if the full L2 block range is inside the proof-history window.

## Rollout Order

1. Deploy or update the Hoodi `alethia-reth` witness node.
2. Wait until proof-history metrics cover the intended proposal range.
3. Smoke-test `debug_executionWitnessForTxList` directly on the witness node.
4. Deploy `raiko2-sgx` and `gaiko2-sgxgeth` remote providers, recording image digests and
   attestation metadata for the verifier allowlist process.
5. Deploy `raiko2` with `l2_provider = "reth"` and `l2_witness_rpc` pointing at the new witness
   node.
6. Run one-proposal smoke tests for `sgx` and `sgxgeth`.
7. Run the recent-proposal regression gate.
8. Roll out traffic gradually and monitor witness-generation and remote-prover latency.

## Rollback

The `alethia-reth` API is additive, so a `raiko2` rollback can usually leave the witness node in
place. If the witness node itself is unhealthy, route traffic back to the previously known-good L2
RPC/witness topology.

Rollback triggers:

- repeated `debug_executionWitnessForTxList` RPC errors
- proof-history window falls behind the proposal range
- witness count mismatch
- SGX or SGXGETH remote-provider health failures
- verifier rejection caused by signer or `mr_enclave` mismatch

## Notes

- `debug_executionWitnessForTxList` replays the explicit transaction list on top of the requested
  block's parent state.
- Debug experiments may use the optional `skipZkGasDifficultyCheck` flag, but production `raiko2`
  proving should not depend on that override.
- If a remote provider image changes, re-check the signer and measurement values expected by the
  verifier before sending production traffic.
