# Preflight Performance And Witness Timeouts

## Summary

Shasta `preflight` is currently a material bottleneck for SGX regression runs.

Two distinct symptoms were reproduced:

- older Taiko mainnet ranges can fail during witness collection with
  `debug_executionWitness ... deadline has elapsed`
- newer ranges may succeed, but still take multiple minutes before proving can start

Because both `proof_type=sgx` and `proof_type=sgxgeth` remain stuck in `active_stage=preflight`,
this is upstream of the remote prover selection and blocks both SGX lanes equally.

## Evidence

### Fast Failure On Older Range

Tuple:

- `proposal_id=2222`
- `l2=5412225..5412416`
- `l1_inclusion_block_number=24862953`
- `last_anchor_block_number=24862885`

Observed via `raiko2` task status:

```text
There was an error with the RPC provider: debug_executionWitness failed for block 5412228: deadline has elapsed
```

### Slow But Successful Standalone Preflight

Tuple:

- `proposal_id=2660`
- `l2=5496213..5496404`
- `l1_inclusion_block_number=24876929`
- `last_anchor_block_number=24876859`

Command:

```bash
/usr/bin/time -p cargo run -q -p preflight -- \
  --rpc-url http://l2-rpc.example.com:8545 \
  --l1-rpc-url https://ethereum-rpc.publicnode.com \
  --l2-chain-id 167000 \
  --l1-chain-id 1 \
  --proposal-id 2660 \
  --l1-inclusion-block-number 24876929 \
  --last-anchor-block-number 24876859 \
  --l2-start 5496213 \
  --l2-end 5496404 \
  --output /tmp/raiko2-preflight-2660.json
```

Observed result:

- `elapsed_ms=172012`
- `/usr/bin/time real 172.69`

Output artifact:

- `/tmp/raiko2-preflight-2660.json`

### Long-Running Main-Service Preflight

Tuple:

- `proposal_id=2379`
- `l2=5442330..5442521`
- `l1_inclusion_block_number=24867961`
- `last_anchor_block_number=24867891`

Observed via `raiko2` task status:

- `proof_type=sgx` task `task_18a62ea8e2b02eb9_3`
- `proof_type=sgxgeth` task `task_18a62ea911b7ae1d_4`

Both remained in:

- `status=proving`
- `runtime.active_stage=preflight`

for multiple minutes, with `updated_at` continuing to advance.

Standalone `preflight` for the same tuple eventually failed after a much longer run:

```bash
/usr/bin/time -p cargo run -q -p preflight -- \
  --rpc-url http://l2-rpc.example.com:8545 \
  --l1-rpc-url https://ethereum-rpc.publicnode.com \
  --l2-chain-id 167000 \
  --l1-chain-id 1 \
  --proposal-id 2379 \
  --l1-inclusion-block-number 24867961 \
  --last-anchor-block-number 24867891 \
  --l2-start 5442330 \
  --l2-end 5442521 \
  --output /tmp/raiko2-preflight-2379.json
```

Observed result:

- `/usr/bin/time real 1305.70`
- failed with:

```text
There was an error with the RPC provider: error sending eth_getProof batch for parent of block 5442464: error sending request for url (http://l2-rpc.example.com:8545/)
```

### Historical-Block Bias

The current regression tuples are all historical ranges rather than blocks near the head.

That matters because `preflight` has to replay the entire 192-block span through the slow witness
path:

- `eth_getBlockByNumber` batches for the full range
- `debug_executionWitness` over historical blocks
- `eth_getProof` lookups for parent/state linkage

This biases local performance measurements toward the worst path and likely overstates the cost of
fresh or near-head proposals.

## Impact

- end-to-end SGX regression runs are dominated by witness collection and preflight latency
- `sgx` and `sgxgeth` appear equally slow because both wait behind the same preflight stage
- older ranges may fail entirely before the request reaches either remote prover
- historical regression ranges amplify the problem because every block in the batch takes the
  expensive historical witness path

## Next Investigation

- instrument `preflight` / provider witness calls to measure per-block `debug_executionWitness`
  latency
- confirm whether the slowdown is uniform across the full range or concentrated on a few blocks
- consider whether current RPC timeout and retry defaults are too aggressive for older ranges
- compare standalone `preflight` timings with the in-engine `preflight` path to rule out queue or
  orchestration overhead
