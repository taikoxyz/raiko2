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
  "graffiti": "0x0000000000000000000000000000000000000000000000000000000000000000",
  "prover": "0x0000000000000000000000000000000000000000",
  "blob_proof_type": "kzg_versioned_hash"
}
```

### Rules

- `proposals` must not be empty.
- `proposal.l2_block_numbers` must be non-empty, strictly increasing, and contiguous.
- `proposal.checkpoint` is not supported in this version.
- `proof_type` mapping:
  - `native -> native/local`
  - `sp1 -> sp1/local`
  - `risc0 -> risc0/<server default runner>`
  - `zk_any -> risc0/<server default runner>`
  - `sgx -> 400`
- `network` and `l1_network` must match an explicitly configured allowed pair.
- flattened `prover_args` are accepted, but they must not override route/spec selection keys such as
  `proof_type`, `network`, `l1_network`, `guest_system`, or `runner`.

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
          "offchain": false
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
- `data.runtime` is the root task runtime view stored in `runtime.sqlite`.
- `proposals[].runtime` and `aggregate.runtime` expose runner-specific runtime metadata when it
  exists. For `risc0/boundless`, that includes `provider_request_id`, `remote_tx_hash`,
  `image_ref`, `deployment`, and `offchain`.
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
- `rpc.client.witness_mode` controls how L2 witnesses are fetched:
  - `auto`: prefer remote `debug_executionWitness`
  - `remote`: require remote `debug_executionWitness`
  - `local`: force local witness generation against the configured L2 RPC
- `rpc.client.local_witness_concurrency_limit` controls how many blocks can run on-the-spot
  witness generation concurrently when `witness_mode=local`.
- Built-in `SupportedChainSpecs::default()` is the only spec source in this version.
- Legacy single-pair `rpc.l1_rpc` / `rpc.l2_rpc` / `rpc.l1_chain_id` / `rpc.l2_chain_id` remains
  as a fallback only when `rpc.pairs` is empty.
