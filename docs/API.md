# Raiko2 API

## Overview

Raiko2 exposes a hoodi-compatible v3 API for Shasta batch proof requests and aggregation.
The public API surface is:

- `POST /v3/proof/batch/shasta`
- `POST /v3/proof/aggregate`
- `GET /v3/tasks/{id}`
- `POST /v3/tasks/{id}/cancel`
- `GET /health`
- `GET /ready`

`/v1/...` routes are removed.

## Health

```http
GET /health
```

## Ready

```http
GET /ready
```

Readiness checks every configured `(network, l1_network)` pair in `rpc.pairs`.

## Submit Shasta Batch Proof

```http
POST /v3/proof/batch/shasta
Content-Type: application/json
```

### Request

```json
{
  "proposals": [
    {
      "proposal_id": 42,
      "l1_inclusion_block_number": 100,
      "l2_block_numbers": [42, 43, 44],
      "last_anchor_block_number": 41
    }
  ],
  "aggregate": true,
  "proof_type": "zk_any",
  "network": "taiko_hoodi",
  "l1_network": "hoodi",
  "sp1": {
    "mode": "prove",
    "prover": "network",
    "recursion": "plonk",
    "verify": true,
    "network_mode": "reserved",
    "fulfillment_strategy": "reserved",
    "skip_simulation": true,
    "cycle_limit": 1000000000000,
    "timeout_secs": 7200
  },
  "graffiti": "0x0000000000000000000000000000000000000000000000000000000000000000",
  "prover": "0x0000000000000000000000000000000000000000",
  "blob_proof_type": "kzg_versioned_hash"
}
```

### Rules

- `proposals` must not be empty.
- `proposal.l2_block_numbers` must be non-empty, strictly increasing, and contiguous.
- `proposal.checkpoint` is not supported in this version.
- `proposal.l1_inclusion_block_number` is required. The server derives the canonical Shasta
  `proposal_event` and `originBlockHash` from L1 RPC; clients should not send
  `shasta_proposal_event` overrides.
- `proposal.last_anchor_block_number` participates in Shasta anchor monotonicity validation. At
  least one anchor in the batch must advance beyond it.
- `proof_type` mapping:
  - `native -> native/local`
  - `sp1 -> sp1/local`
  - `risc0 -> risc0/<server default runner>`
  - `zk_any -> risc0/<server default runner>`
  - `sgx -> 400`
- Optional request-scoped prover config may be passed as flattened keys. For `sp1`, the canonical
  shape is a nested `sp1` object with:
  - `mode`: `prove` or `execute`
  - `prover`: `local`, `mock`, or `network`
  - `recursion`: `core`, `compressed`, or `plonk`
  - `verify`: `true` or `false`
  - `network_mode`: `reserved` or `mainnet` (network prover only)
  - `fulfillment_strategy`: `reserved`, `hosted`, or `auction` (network prover only)
  - `skip_simulation`: `true` or `false` (network prover only)
  - `cycle_limit`: positive integer (network prover only)
  - `timeout_secs`: positive integer (network prover only)
- `sp1.mode=execute` is only valid when `proof_type=sp1`.
- `sp1.mode=execute` requires `aggregate=false`.
- `sp1.mode=execute` does not support `sp1.prover=network`.
- `sp1` network-only settings require `sp1.prover=network`.
- `sp1.network_mode=mainnet` requires `sp1.fulfillment_strategy=auction`.
- `sp1.network_mode=reserved` requires `sp1.fulfillment_strategy=reserved` or `hosted`.
- `network` and `l1_network` must match an explicitly configured allowed pair.
- flattened `prover_args` are accepted, but they must not override route/spec selection keys such as
  `proof_type`, `network`, `l1_network`, `guest_system`, or `runner`.
- `NETWORK_PRIVATE_KEY` must be present in the server environment when `sp1.prover=network` is
  used. `sp1.rpc_url` is an operator config file setting only; it is not accepted in request
  overrides.

### Response

```json
{
  "status": "ok",
  "proof_type": "zk_any",
  "data": {
    "status": "registered",
    "task_id": "task_..."
  }
}
```

## Submit Aggregate Proof

```http
POST /v3/proof/aggregate
Content-Type: application/json
```

### Request

```json
{
  "proofs": [
    {
      "proof": "0x...",
      "input": "0x...",
      "quote": "...",
      "uuid": "0x...",
      "extra_data": {
        "shasta": {
          "proof_carry_data": {}
        }
      }
    }
  ],
  "proof_type": "risc0",
  "network": "taiko_hoodi",
  "l1_network": "hoodi"
}
```

### Rules

- `proofs` must contain at least two entries.
- Only canonical `Proof` objects are accepted.
- Required metadata depends on the selected route:
  - `native`: `input` + `extra_data`
  - `sp1`: `input` + `uuid` + `extra_data`
  - `risc0/local`: `input` + `uuid` + `quote` + `extra_data`
  - `risc0/boundless`: `quote`

### Response

```json
{
  "status": "ok",
  "proof_type": "risc0",
  "data": {
    "status": "registered",
    "task_id": "task_..."
  }
}
```

## Query Task

```http
GET /v3/tasks/{id}
```

### Response

```json
{
  "status": "ok",
  "proof_type": "zk_any",
  "data": {
    "task_id": "task_...",
    "route": "risc0/boundless",
    "execution_mode": "prove",
    "status": "proving",
    "network": "taiko_hoodi",
    "l1_network": "hoodi",
    "runtime": {
      "runner_status": "running",
      "active_stage": "prove",
      "last_event": "submission_registered",
      "updated_at": 1742836800,
      "engine_state_present": true
    },
    "current_index": 1,
    "proposals": [
      {
        "index": 0,
        "proposal_id": 42,
        "task_id": "...",
        "status": "completed",
        "l1_inclusion_block_number": 100,
        "l2_block_numbers": [42, 43, 44],
        "last_anchor_block_number": 41,
        "proof": "0x...",
        "runtime": {
          "updated_at": 1742836800,
          "engine_state_present": true,
          "provider_request_id": "0x1234",
          "remote_tx_hash": "0xabcd",
          "image_ref": "0ximage",
          "deployment": "base",
          "offchain": false,
          "quoted_mcycles_count": 6000,
          "evaluated_mcycles_count": 12345
        }
      }
    ],
    "aggregate": {
      "task_id": "...",
      "status": "pending"
    }
  }
}
```

`current_index` points at the first unfinished proposal. When proposal proving is done and an
aggregate task exists, it becomes `proposals.len()`.

### Runtime Semantics

- `data.route` is the canonical resolved route that accepted the request, such as
  `native/local`, `sp1/local`, `risc0/local`, or `risc0/boundless`.
- `data.execution_mode` is present for SP1 tasks and distinguishes `prove` from `execute`.
- `data.runtime` is the root task runtime view stored in `runtime.sqlite`.
- `proposals[].runtime` and `aggregate.runtime` expose runner-specific runtime metadata when it
  exists. For `risc0/boundless`, that includes `provider_request_id`, `remote_tx_hash`,
  `image_ref`, `deployment`, `offchain`, `quoted_mcycles_count`, and
  `evaluated_mcycles_count`.
- When `data.execution_mode=execute`, proposal completion returns `proof = null` and places the
  execute report under `proposals[].extra_data.sp1`.
- `engine_state_present=false` means the HTTP response is serving the last runtime snapshot even
  though the in-memory engine no longer has a live status object for that stage. This preserves
  observability after container restarts, but it does not imply task recovery.

## Cancel Task

```http
POST /v3/tasks/{id}/cancel
```

Cancelling a batch root cascades to every proposal stage task and to the optional aggregate task.

## Error Envelope

All API errors use the hoodi-style envelope:

```json
{
  "status": "error",
  "error": "bad_request",
  "message": "..."
}
```

## Configuration Notes

- `rpc.pairs` is the canonical configuration for allowed `(network, l1_network)` combinations.
- `rpc.l2_rpc` must point to a witness-capable endpoint that supports `debug_executionWitness`.
- If the upstream L2 does not expose `debug_executionWitness`, deploy `zeth-rpc-proxy` as a
  compatibility layer and point `rpc.l2_rpc` at that proxy.
- Built-in `SupportedChainSpecs::default()` is the only spec source in this version.
- Legacy single-pair `rpc.l1_rpc` / `rpc.l2_rpc` / `rpc.l1_chain_id` / `rpc.l2_chain_id` remains
  as a fallback only when `rpc.pairs` is empty.
