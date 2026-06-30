# Raiko2 API

## Overview

Raiko2 exposes a Shasta-first `/v3` API aligned with the current upstream `raiko` proof surface,
plus `raiko2` task-inspection extensions under `/v3/tasks/*`.

Proof submission is asynchronous. The legacy `POST /proof/*` responses intentionally match old
`raiko` v3 response shapes and do not expose raiko2 task IDs. Operators can:

- query `GET /v3/proof/report` for all root task IDs and full root-task views
- poll `GET /v3/tasks/{id}` for one full root-task view
- query `GET /v3/proof/list` for completed root tasks with final proof material

The proof routes are available both under `/v3/proof/*` and `/proof/*`.

The public API surface is:

- `POST /v3/proof/batch/shasta`
- `POST /v3/proof/aggregate`
- `GET /v3/proof/report`
- `GET /v3/proof/list`
- `GET /v3/prover/status`
- `GET /v3/tasks/{id}` (`raiko2` extension)
- `GET /health`
- `GET /metrics`
- `GET /ready`

ACL-protected API surface requires an `x-api-key` whose ACL allows the listed feature:

- A key with `admin` is accepted for any ACL-protected endpoint.
- `POST /v3/proof/prune` requires `admin`
- `POST /proof/prune` requires `admin`
- `POST /v3/tasks/{id}/cancel` requires `admin`
- `POST /v3/prover/clear` requires `prover.clear`
- `GET /admin/ballot` requires `admin.ballot.read`
- `POST /admin/ballot` requires `admin.ballot.write`

`/v1/...` routes are removed.

## Health

```http
GET /health
```

## Ready

```http
GET /ready
```

Readiness checks every configured `(network, l1_network)` pair in `rpc.pairs`, the configured
queue backend, and the hosted proving capabilities exposed by the endpoint.

## Metrics

```http
GET /metrics
```

Returns Prometheus text-format metrics for the hosted API.

The canonical minimal metric families are:

- `raiko2_request_registrations_total`
- `raiko2_stage_tasks_inflight`
- `raiko2_stage_task_started_total`
- `raiko2_stage_task_terminal_total`
- `raiko2_stage_task_duration_seconds`
- `raiko2_external_submission_total`

Stage metrics are labeled by `route`, `proof_type`, `pair`, `aggregate`, and `stage`.
Terminal counters and duration histograms also include `status`.

## Admin Ballot

```http
GET /admin/ballot
POST /admin/ballot
x-api-key: <server.acl.keys[].key with allow=["admin.ballot.read" or "admin.ballot.write"]>
```

These endpoints mirror old `raiko` dynamic ballot control for `proof_type=zk_any`. `GET` requires
`admin.ballot.read`; `POST` requires `admin.ballot.write`. The payload is a JSON object whose
keys are `Sp1` and `Risc0`, and whose values are `[probability, per_day]` tuples. Only those two
proof types are accepted.

## V4 Prover API Spec

V4 success envelope:

```json
{
  "status": "ok",
  "data": {}
}
```

V4 error envelope:

```json
{
  "status": "error",
  "error": "invalid_proof_type",
  "message": "proof_type must be a concrete proof type"
}
```

Common request fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `network` | string | yes | L2 network key configured in `rpc.pairs`. |
| `l1_network` | string | yes | L1 network key configured in `rpc.pairs`. |
| `proof_type` | string | yes | Concrete proof backend. `zk_any` is rejected. |

Common `proof_type` values:

| Value | Proposal | Aggregation | Status/Clear |
| --- | --- | --- | --- |
| `risc0` | yes | yes | yes |
| `sp1` | yes | yes | yes |
| `sgx` | yes | yes | no |
| `sgxgeth` | yes | yes | no |
| `native` | yes | no | no |
| `zk_any` | no | no | no |

### Submit Proposal Proof

```http
POST /v4/proof/proposal
Content-Type: application/json
```

Request:

```json
{
  "network": "taiko_mainnet",
  "l1_network": "ethereum",
  "proposal_id": 12345,
  "l1_inclusion_block_number": 100,
  "l2_block_number_start": 200,
  "l2_block_number_end": 201,
  "last_anchor_block_number": 199,
  "checkpoint": null,
  "proof_type": "risc0",
  "prover": "0x0000000000000000000000000000000000000000",
  "graffiti": "0x0000000000000000000000000000000000000000000000000000000000000000",
  "blob_proof_type": "proof_of_equivalence",
  "prover_args": {}
}
```

Request fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `network` | string | yes | L2 network key configured in `rpc.pairs`. |
| `l1_network` | string | yes | L1 network key configured in `rpc.pairs`. |
| `proposal_id` | number | yes | Taiko proposal ID to prove. |
| `l1_inclusion_block_number` | number | yes | L1 block where the proposal was included. |
| `l2_block_number_start` | number | yes | First L2 block number covered by the proposal. |
| `l2_block_number_end` | number | yes | Last L2 block number covered by the proposal. |
| `last_anchor_block_number` | number | yes | Last anchor block number before the proposal range. |
| `checkpoint` | object/null | no | Fork-specific checkpoint data when required. |
| `proof_type` | string | yes | One of `risc0`, `sp1`, `sgx`, `sgxgeth`, `native`. |
| `prover` | address | no | Designated prover address. |
| `graffiti` | bytes32 | no | Graffiti passed to proposal proof construction. |
| `blob_proof_type` | string | no | Blob proof mode, when required by the selected fork. |
| `prover_args` | object | no | Backend-specific prover options, such as `sp1.cycle_limit`. |

Response:

```json
{
  "status": "ok",
  "proof_type": "risc0",
  "data": {
    "task_id": "task_0x1234",
    "status": "registered",
    "proof": null
  }
}
```

Response fields:

| Field | Type | Description |
| --- | --- | --- |
| `proof_type` | string | Concrete proof backend selected by the caller. |
| `task_id` | string | Root task ID. |
| `status` | string | `registered`, `work_in_progress`, `completed`, `failed`, or `cancelled`. |
| `proof` | object/null | Final proof payload when `data.status=completed`. |

Validation:

- `proof_type=zk_any` returns `400 invalid_proof_type`.
- Unknown `proof_type` returns `400 invalid_proof_type`.
- Unsupported concrete `proof_type` for the configured network returns `400 unsupported_proof_type`.
- Unknown `(network, l1_network)` returns `400 invalid_network_pair`.
- `l2_block_number_start` and `l2_block_number_end` define an inclusive range and must satisfy
  `l2_block_number_start <= l2_block_number_end`.
- `l2_block_numbers` is not accepted by the v4 proposal request.
- Invalid or unavailable proposal context returns `400 invalid_request`.
- Unsupported proposal fork returns `400 unsupported_fork`.

### Submit Aggregation Proof

```http
POST /v4/proof/aggregation
Content-Type: application/json
```

Request:

```json
{
  "network": "taiko_mainnet",
  "l1_network": "ethereum",
  "aggregation_ids": [12345, 12346],
  "proof_type": "sp1",
  "proofs": [
    {
      "proposal_id": 12345,
      "proof": "0x",
      "quote": "0x",
      "input": {},
      "uuid": "017c1f17-8f1a-4f62-8f71-7e2a5b5d7800",
      "extra_data": {}
    }
  ]
}
```

Request fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `network` | string | yes | L2 network key configured in `rpc.pairs`. |
| `l1_network` | string | yes | L1 network key configured in `rpc.pairs`. |
| `aggregation_ids` | number[] | yes | Proposal IDs covered by the aggregation. |
| `proof_type` | string | yes | One of `risc0`, `sp1`, `sgx`, `sgxgeth`. |
| `proofs` | object[] | yes | Proposal proof payloads to aggregate. |
| `proofs[].proposal_id` | number | yes | Proposal ID for this proof payload. |
| `proofs[].proof` | string/object | backend-specific | Proof bytes or structured proof payload. |
| `proofs[].quote` | string/object | backend-specific | Quote or receipt payload. |
| `proofs[].input` | object | backend-specific | Guest input or public input payload. |
| `proofs[].uuid` | string | backend-specific | External prover order or proof UUID. |
| `proofs[].extra_data` | object | no | Backend-specific metadata. |

Response:

```json
{
  "status": "ok",
  "proof_type": "sp1",
  "data": {
    "task_id": "task_0x5678",
    "status": "registered",
    "proof": null
  }
}
```

Response fields:

| Field | Type | Description |
| --- | --- | --- |
| `proof_type` | string | Concrete proof backend selected by the caller. |
| `task_id` | string | Root task ID. |
| `status` | string | `registered`, `work_in_progress`, `completed`, `failed`, or `cancelled`. |
| `proof` | object/null | Final aggregation proof payload when `data.status=completed`. |

Validation:

- `proof_type=zk_any` returns `400 invalid_proof_type`.
- Empty `aggregation_ids` returns `400 invalid_request`.
- Empty `proofs` returns `400 invalid_request`.
- A `proofs[].proposal_id` not present in `aggregation_ids` returns `400 invalid_request`.
- Unknown `(network, l1_network)` returns `400 invalid_network_pair`.
- Unsupported aggregation fork returns `400 unsupported_fork`.

### Query V4 Task

```http
GET /v4/tasks/{id}
```

Path fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `id` | string | yes | Root task ID returned by a v4 submission route. |

Response:

```json
{
  "status": "ok",
  "data": {
    "task_id": "task_0x1234",
    "kind": "proposal",
    "network": "taiko_mainnet",
    "l1_network": "ethereum",
    "proposal_id": 12345,
    "aggregation_ids": null,
    "proof_type": "risc0",
    "status": "completed",
    "proof": {},
    "error": null
  }
}
```

Response fields:

| Field | Type | Description |
| --- | --- | --- |
| `task_id` | string | Root task ID. |
| `kind` | string | `proposal` or `aggregation`. |
| `network` | string | L2 network key. |
| `l1_network` | string | L1 network key. |
| `proposal_id` | number/null | Proposal ID for proposal tasks. |
| `aggregation_ids` | number[]/null | Aggregation IDs for aggregation tasks. |
| `proof_type` | string | Concrete proof backend selected by the caller. |
| `status` | string | `registered`, `work_in_progress`, `completed`, `failed`, or `cancelled`. |
| `proof` | object/null | Final proof payload when available. |
| `error` | string/null | Failure reason when `status=failed`. |

Validation:

- Unknown `id` returns `404 task_not_found`.

### Query V4 Prover Status

```http
GET /v4/prover/status?proof_type=risc0&network=taiko_mainnet&l1_network=ethereum
```

Query fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `proof_type` | string | yes | One of `risc0`, `sp1`. |
| `network` | string | no | Optional L2 network filter. |
| `l1_network` | string | no | Optional L1 network filter. |

Response:

```json
{
  "status": "ok",
  "data": {
    "proof_type": "risc0",
    "network": "taiko_mainnet",
    "l1_network": "ethereum",
    "clean": false,
    "tasks": {
      "pending": 1,
      "ready": 0,
      "retrying": 0,
      "running": 1,
      "orphaned": 0
    },
    "remote": {
      "inflight_orders": 0
    },
    "skipped": {
      "invalid_metadata": 0,
      "unavailable_pipeline": 0
    }
  }
}
```

Response fields:

| Field | Type | Description |
| --- | --- | --- |
| `proof_type` | string | Concrete proof backend being reported. |
| `network` | string/null | L2 network filter, or null when unfiltered. |
| `l1_network` | string/null | L1 network filter, or null when unfiltered. |
| `clean` | boolean | True when the selected proof type has no non-terminal backlog. |
| `tasks.pending` | number | Matching queue tasks waiting to become ready. |
| `tasks.ready` | number | Matching queue tasks ready to run. |
| `tasks.retrying` | number | Matching queue tasks waiting for retry. |
| `tasks.running` | number | Matching queue tasks currently running. |
| `tasks.orphaned` | number | Matching non-terminal tasks whose runtime record is missing or stale. |
| `remote.inflight_orders` | number | Matching external prover orders with resumable progress. |
| `skipped.invalid_metadata` | number | Non-terminal roots skipped because metadata is unreadable. |
| `skipped.unavailable_pipeline` | number | Non-terminal roots skipped because their pipeline is unavailable. |

Validation:

- Missing `proof_type` returns `400 missing_proof_type`.
- `proof_type=zk_any` returns `400 invalid_proof_type`.
- Any `proof_type` other than `risc0` or `sp1` returns `400 unsupported_proof_type`.
- Supplying only one of `network` or `l1_network` returns `400 invalid_network_pair`.

### Clear V4 Prover Backlog

```http
POST /v4/prover/clear
Content-Type: application/json
x-api-key: <server.acl.keys[].key with allow=["prover.clear"] or allow=["admin"]>
```

Request:

```json
{
  "proof_type": "risc0",
  "network": "taiko_mainnet",
  "l1_network": "ethereum"
}
```

Request fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `proof_type` | string | yes | One of `risc0`, `sp1`. |
| `network` | string | no | Optional L2 network filter. |
| `l1_network` | string | no | Optional L1 network filter. |

Response:

```json
{
  "status": "ok",
  "data": {
    "proof_type": "risc0",
    "network": "taiko_mainnet",
    "l1_network": "ethereum",
    "cancelled": 2,
    "skipped": {
      "invalid_metadata": 0,
      "unavailable_pipeline": 0,
      "remote_progress": 1,
      "failed": 0
    }
  }
}
```

Response fields:

| Field | Type | Description |
| --- | --- | --- |
| `proof_type` | string | Concrete proof backend targeted by the clear operation. |
| `network` | string/null | L2 network filter, or null when unfiltered. |
| `l1_network` | string/null | L1 network filter, or null when unfiltered. |
| `cancelled` | number | Non-terminal root tasks cancelled. |
| `skipped.invalid_metadata` | number | Roots skipped because metadata is unreadable. |
| `skipped.unavailable_pipeline` | number | Roots skipped because their pipeline is unavailable. |
| `skipped.remote_progress` | number | Roots skipped because an external prover order has resumable progress. |
| `skipped.failed` | number | Roots that failed during cancellation. |

Validation:

- Missing `proof_type` returns `400 missing_proof_type`.
- `proof_type=zk_any` returns `400 invalid_proof_type`.
- Any `proof_type` other than `risc0` or `sp1` returns `400 unsupported_proof_type`.
- Supplying only one of `network` or `l1_network` returns `400 invalid_network_pair`.

## Submit Shasta Batch Proof

```http
POST /v3/proof/batch/shasta
Content-Type: application/json
```

Registers a Shasta batch root task. The server expands it into proposal prove tasks and, when
`aggregate=true`, an aggregation task. For remote SGX lanes, configure `[prover.remote_sgx]`:
`base_url` backs `proof_type=sgx`, while `sgxgeth_base_url` backs `proof_type=sgxgeth`.

### Request

```json
{
  "proposals": [
    {
      "proposal_id": 42,
      "checkpoint": {
        "block_number": 44,
        "block_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "state_root": "0x0000000000000000000000000000000000000000000000000000000000000000"
      },
      "l1_inclusion_block_number": 100,
      "l2_block_numbers": [42, 43, 44],
      "last_anchor_block_number": 41
    }
  ],
  "aggregate": true,
  "proof_type": "sp1",
  "network": "taiko_hoodi",
  "l1_network": "hoodi",
  "sp1": {
    "mode": "prove",
    "prover": "network",
    "verify": true,
    "network_mode": "reserved",
    "fulfillment_strategy": "reserved",
    "skip_simulation": true,
    "cycle_limit": 1000000000000,
    "timeout_secs": 7200
  },
  "graffiti": "0x0000000000000000000000000000000000000000000000000000000000000000",
  "prover": "0x0000000000000000000000000000000000000000",
  "blob_proof_type": "proof_of_equivalence"
}
```

### Rules

- `proposals` must not be empty.
- `aggregate=true` is allowed with a single proposal for backward compatibility with `raiko`.
- `proposal.l2_block_numbers` must be non-empty, strictly increasing, and contiguous.
- `proposal.checkpoint` is optional and is validated against the canonical final witness
  checkpoint when the proof is built.
- `proposal.l1_inclusion_block_number` is required. The server derives canonical Shasta proposal
  data from RPC; request-time internal manifest overrides are not accepted.
- `proposal.proposal_id` must fit Shasta's `uint48` protocol field.
- `proposal.last_anchor_block_number` participates in Shasta anchor monotonicity validation.
- `proof_type` mapping:
  - `native -> native/local` only when the server route is `native/local`; otherwise rejected
  - `sp1 -> sp1/local | sp1/network` from the effective SP1 prover mode
  - `risc0 -> risc0/<server default runner>` with `prover_type = mock | local | network`
  - `zk_any -> admission-time draw to sp1 or risc0`
  - `sgx -> sgx/remote` backed by `raiko2-sgx-prover`
  - `sgxgeth -> sgx/remote` backed by the external geth-backed remote SGX server
  - `boundless -> unsupported legacy error response`
- When the server prover route is `sgx/remote`, the hosted public API only accepts
  `proof_type=sgx` and `proof_type=sgxgeth`.
- `proof_type=zk_any` is only supported on `POST /v3/proof/batch/shasta`.
- `proof_type=zk_any` is only valid when `aggregate=false`. It is an admission-time draw for
  proposal proving. When drawn, the selected concrete proof type (`sp1` or `risc0`) is the
  canonical route, task key, and proof artifact key.
- `aggregate=true` requires a concrete `proof_type` such as `sp1`, `risc0`, `sgx`, or `sgxgeth`.
  Aggregate requests may reuse existing proposal proof artifacts for that concrete proof type.
- Boundless is the current network provider for `proof_type=risc0`; the canonical HTTP task
  field is `prover_type = "network"`.
- Hosted `proof_type=sp1` batch proposal proving always emits Compressed proofs.
- Hosted `proof_type=sp1` aggregation always emits a Plonk proof.
- When a `zk_any` request is not drawn, the server returns HTTP 200 with:
  - `proof_type = "zk_any"`
  - `batch_id = first proposal_id`
  - `data.status = "zk_any_not_drawn"`
- Request-scoped prover overrides are strict and typed. The public API accepts flattened
  top-level prover namespaces:
  - `sp1` is supported
  - `native`, `risc0`, `sgx`, and `sgxgeth` request-scoped prover args are rejected
- `proof_type=zk_any` does not accept request-scoped prover args.
- `network` and `l1_network` are optional for backward compatibility with old `raiko` clients.
  When omitted, the server uses the first configured entry in `rpc.pairs` as the default pair.
  If either field is provided, both fields must be provided together.
- `sp1.mode=execute` is only valid when `proof_type=sp1`.
- `sp1.mode=execute` requires `aggregate=false`.
- `sp1.mode=execute` does not support `sp1.prover=network`.
- `sp1.mode=prove` requires `sp1.verify=true` on the hosted API.
- `sp1.prover=network` with `sp1.verify=true` requires the selected `(network, l1_network)` pair
  to declare `sp1_verifier_rpc_url` and `sp1_verifier_address` in server config.
  `sp1_verifier_address` is the Succinct SP1 verifier gateway used for `ISP1Verifier.verifyProof`,
  not the Taiko Shasta verifier registered in the chain spec.
- `sp1` network-only settings require `sp1.prover=network`.
- `sp1.network_mode=mainnet` requires `sp1.fulfillment_strategy=auction`.
- `sp1.network_mode=reserved` requires `sp1.fulfillment_strategy=reserved` or `hosted`.
- When provided, `network` and `l1_network` must match an explicitly configured allowed pair.
- `NETWORK_PRIVATE_KEY` must be present in the server environment when `sp1.prover=network` is
  used. `sp1.rpc_url` is operator configuration only and is not accepted as a request override.

### Response

`POST /v3/proof/batch/shasta` is idempotent for canonically identical requests.

- The first accepted request returns old-raiko `registered`.
- A repeated identical request may return:
  - `data.status = "registered"`
  - `data.status = "work_in_progress"`
  - `data.proof` when the proof is ready
  - `data.status = {"anyhow_error": "..."}` for a terminal stored task failure
- A failed repeated task is re-enqueued when it failed before remote submission, or when stored
  remote-submission metadata is sufficient to resume it. This decision is independent of proof type.
- Request validation errors return old-raiko top-level `status = "error"` with HTTP 200.
- `GET /v3/proof/report` and `GET /v3/tasks/{id}` remain the raiko2 task-inspection APIs.

```json
{
  "status": "ok",
  "proof_type": "sp1",
  "data": {
    "status": "registered"
  }
}
```

Example repeated-request success after completion:

```json
{
  "status": "ok",
  "proof_type": "sp1",
  "data": {
    "proof": {
      "proof": "0x...",
      "input": null,
      "quote": null,
      "uuid": null,
      "kzg_proof": null,
      "extra_data": null
    }
  }
}
```

Example stored task failure:

```json
{
  "status": "ok",
  "proof_type": "sp1",
  "data": {
    "status": {
      "anyhow_error": "..."
    }
  }
}
```

Example not-drawn response:

```json
{
  "status": "ok",
  "proof_type": "zk_any",
  "batch_id": 42,
  "data": {
    "status": "zk_any_not_drawn"
  }
}
```

## Submit Aggregate Proof

```http
POST /v3/proof/aggregate
Content-Type: application/json
```

Registers an aggregation root task from externally supplied proposal proofs.

### Request

```json
{
  "aggregation_ids": [42, 43],
  "proofs": [
    {
      "proof": "0x...",
      "input": "0x...",
      "uuid": "...",
      "extra_data": {}
    },
    {
      "proof": "0x...",
      "input": "0x...",
      "uuid": "...",
      "extra_data": {}
    }
  ],
  "proof_type": "sp1",
  "network": "taiko_hoodi",
  "l1_network": "hoodi"
}
```

### Rules

- `proofs` must not be empty.
- Single-proof aggregation is allowed for backward compatibility with `raiko`.
- `aggregation_ids` is optional for backward compatibility with old `raiko` clients.
- `proof_type` must be a concrete proof type: `risc0`, `sp1`, `sgx`, or `sgxgeth`.
- `proof_type=zk_any` is not supported for aggregate requests.
- `network` and `l1_network` are optional for backward compatibility with old `raiko` clients.
  When omitted, the server uses the first configured entry in `rpc.pairs` as the default pair.
  If either field is provided, both fields must be provided together.
- `proof_type=sp1` requires each proof to include `proof`, `input`, `uuid`, and `extra_data`.
- Hosted `proof_type=sp1` aggregate requests expect Compressed proposal proofs and emit a Plonk
  aggregation proof.
- `proof_type=risc0` on the hosted network route requires each proof to include `quote`.
- `sp1.mode=prove` requires `sp1.verify=true` on the hosted API.
- `sp1.prover=network` with `sp1.verify=true` requires pair-level SP1 verifier config on the
  selected `(network, l1_network)`.
- Required metadata depends on the selected route:
  - `native`: `input` + `extra_data`
  - `sgx` / `sgxgeth`: `input` + `extra_data`
  - `sp1`: `proof` + `input` + `uuid` + `extra_data`
  - `risc0/local`: `input` + `uuid` + `quote` + `extra_data`
  - `risc0/network`: `quote`

### Response

```json
{
  "status": "ok",
  "proof_type": "sp1",
  "data": {
    "status": "registered"
  }
}
```

Repeated `POST /v3/proof/aggregate` with the same logical request is idempotent. It reuses the
existing root task and returns the same legacy-compatible `registered` / `work_in_progress` /
`proof` / stored failure status response semantics as `POST /v3/proof/batch/shasta`.

## Report All Root Tasks

```http
GET /v3/proof/report
GET /proof/report
```

Returns an array of root-task views in the same shape as `GET /v3/tasks/{id}` `data`, one entry
per registered root task.

## List Completed Root Proofs

```http
GET /v3/proof/list
GET /proof/list
```

Returns only root-task views whose root status is `completed` and whose final `proof` field is
present.

## Prune All Root Tasks

```http
POST /v3/proof/prune
POST /proof/prune
x-api-key: <server.acl.keys[].key with allow=["admin"]>
```

Requires an ACL key that allows `admin`.

Removes all registered root tasks, their child engine tasks, their runtime rows, and their task
directories. Reusable proof artifacts under `cache/proofs/...` are retained.

### Response

```json
{
  "status": "ok"
}
```

## Query Prover Status

```http
GET /v3/prover/status
```

Returns queue and external-network activity for non-terminal roots originally submitted with
`proof_type=zk_any`. Concrete `sp1` or `risc0` tasks created by the admission draw are still grouped
under the original `zk_any` request for this operator view.

### Response

```json
{
  "status": "ok",
  "data": {
    "clean": false,
    "tasks": {
      "pending": 1,
      "ready": 0,
      "retrying": 0,
      "running": 2,
      "orphaned": 0
    },
    "network": {
      "sp1": {
        "inflight_orders": 1
      },
      "risc0": {
        "inflight_orders": 1
      }
    },
    "skipped": {
      "invalid_metadata": 0,
      "unavailable_pipeline": 1,
      "remote_progress": 0
    }
  }
}
```

`orphaned` counts non-terminal runtime records without active local queue execution and without
remote submission progress. The runtime cleanup pass cancels stale orphaned records after
`runtime.inactive_ttl_secs`.
`clean=true` means there are no matching non-terminal queue tasks in `pending`, `ready`,
`retrying`, `running`, or `orphaned` state, no resumable SP1 or RISC0 network submissions, and
no skipped non-terminal roots with invalid metadata or unavailable pipelines.

## Clear Prover

```http
POST /v3/prover/clear
x-api-key: <server.acl.keys[].key with allow=["prover.clear"] or allow=["admin"]>
```

Requires an ACL key that allows `prover.clear` or `admin`. Marks every non-terminal root originally
submitted with `proof_type=zk_any` as `cancelled` first, then best-effort cancels owned proposal and
aggregation queue tasks.

Shared child tasks still referenced by another live root are left running.
Already submitted upstream SP1 or RISC0/Boundless orders are protected and skipped.

### Response

```json
{
  "status": "ok",
  "cancelled": 2,
  "skipped": {
    "invalid_metadata": 0,
    "unavailable_pipeline": 1,
    "remote_progress": 0
  },
  "failed": 0
}
```

## Query Root Task

```http
GET /v3/tasks/{id}
```

Returns the root-task view derived from the original batch request.

### Response

```json
{
  "status": "ok",
  "proof_type": "risc0",
  "data": {
    "task_id": "task_...",
    "route": "risc0/network",
    "prover_type": "network",
    "execution_mode": "prove",
    "status": "completed",
    "network": "taiko_hoodi",
    "l1_network": "hoodi",
    "runtime": {
      "runner_status": "completed",
      "active_stage": "aggregate",
      "last_event": "completed",
      "updated_at": 1742836800,
      "engine_state_present": true
    },
    "current_index": null,
    "proposals": [
      {
        "index": 0,
        "proposal_id": 42,
        "checkpoint": null,
        "task_id": "...",
        "status": "completed",
        "l1_inclusion_block_number": 100,
        "l2_block_numbers": [42, 43, 44],
        "last_anchor_block_number": 41,
        "proof": "0x...",
        "proof_ref": "proposal:...",
        "proof_path": "cache/proofs/taiko_hoodi_hoodi/proposal_....json"
      }
    ],
    "aggregate": {
      "task_id": "...",
      "status": "completed",
      "proof": "0x...",
      "proof_ref": "aggregate:...",
      "proof_path": "cache/proofs/taiko_hoodi_hoodi/aggregate_....json"
    },
    "proof": "0x...",
    "proof_ref": "aggregate:...",
    "proof_path": "cache/proofs/taiko_hoodi_hoodi/aggregate_....json"
  }
}
```

### Runtime Semantics

- `data.route` is the canonical resolved route that accepted the request, such as
  `native/local`, `sp1/local`, `sp1/network`, `risc0/local`, or `risc0/network`.
- `data.prover_type` is present for zkVM proof types and reports the effective prover mode:
  `mock`, `local`, or `network`. For RISC0, the network mode is currently backed by Boundless.
- `data.execution_mode` is present for SP1 tasks and distinguishes `prove` from `execute`.
- `data.runtime.runner_status` is the persisted root runtime lifecycle stored in `runtime.sqlite`:
  `allocated`, `running`, `completed`, `failed`, or `cancelled`.
- `data.status` is the proof-oriented root status shown to API clients:
  `pending`, `proving`, `completed`, `failed`, or `cancelled`.
- `proof_ref` and `proof_path`, when present, point at the persisted proof artifact for the
  resolved concrete route. For `zk_any` requests these fields use the selected `sp1` or `risc0`
  artifact key, never `zk_any`.
- `proposals[].runtime` and `aggregate.runtime` expose runner-specific runtime metadata when it
  exists. For `risc0/network`, that includes `provider_request_id`, `remote_tx_hash`,
  `expires_at`, `image_ref`, `deployment`, `offchain`, `quoted_mcycles_count`, and
  `evaluated_mcycles_count`.
- Terminal root tasks may be automatically removed from `runtime.sqlite` and `tasks/...` after
  `runtime.inactive_ttl_secs` of inactivity. Active root tasks are never removed by TTL cleanup.
  Completed proof artifacts are stored independently under `cache/proofs/...` and are indexed by
  stable proof refs, so aggregation can reuse them after engine task cleanup or process restart.
- When `data.execution_mode=execute`, proposal completion returns `proof = null` and places the
  execute report under `proposals[].extra_data.sp1`. When present,
  `proposals[].extra_data.sp1.gas` is SP1 prover gas from `ExecutionReport::gas()`, not EVM gas.
- `engine_state_present=false` means the API is serving the last runtime snapshot even though the
  in-memory engine no longer has a live task state object for that stage.

## Cancel Root Task

```http
POST /v3/tasks/{id}/cancel
x-api-key: <server.acl.keys[].key with allow=["admin"]>
```

Requires an ACL key that allows `admin`.

Cancelling a root task cascades to its proposal stage tasks and optional aggregate task. The root
runtime is only marked `cancelled` after child-task cancellation succeeds; shared child tasks are
left running for the other live root that still references them.

## Error Envelope

All API errors use the Hoodi-style envelope:

```json
{
  "status": "error",
  "error": "bad_request",
  "message": "..."
}
```

## Configuration Notes

- `rpc.pairs` is the canonical configuration for allowed `(network, l1_network)` combinations.
- `rpc.pairs[*].beacon_rpc` is optional. When set, Shasta blob sidecar fetches use that L1
  beacon endpoint instead of the built-in endpoint from the resolved L1 chain spec.
- `rpc.pairs[*].l2_rpc` is the canonical read/state RPC used for blocks and account/state proofs.
- `rpc.pairs[*].l2_provider` selects the L2 execution-client family. It defaults to `reth`;
  set it to `geth` when `l2_rpc`/`l2_witness_rpc` points at a geth endpoint with native
  `debug_executionWitness`, or `geth_local_witness` when blocks and accounts come from a regular
  geth endpoint and witnesses must always be assembled locally.
- `rpc.pairs[*].l2_witness_rpc` is optional. When set, witness/debug traffic uses that endpoint
  while the rest of the provider keeps using `l2_rpc`.
- `prover.guest_input_abi` defaults to `current`. Set `v0_1_0` only when intentionally
  running v0.1.0 release guest ELFs with a newer host.
- `prover.boundless.batch_quoted_mcycles` controls proposal quote cycles for `risc0/network`
  when set; `prover.boundless.aggregation_quoted_mcycles` controls aggregation quote cycles.
  `rpc.pairs[*].boundless` can override either value for one `(network, l1_network)` pair.
- `prover.boundless.offer_params.{batch,aggregation}.pricing_mode` defaults to `manual`.
  `manual` requires `max_price_per_mcycle` and optionally accepts `min_price_per_mcycle`;
  `market` delegates price selection to the Boundless SDK price provider, may set
  `dynamic_pricing_timeout_modifier >= 1.0` to multiply `lockTimeout` and `timeout` after dynamic
  pricing, and optionally accepts `max_price_per_mcycle` as a per-mcycle safety cap. The cap value
  is multiplied by the quoted mcycle count; SDK autoprice offers whose resulting `maxPrice` exceeds
  that total cap fail before submission. `market` must omit `min_price_per_mcycle`.
- Expired Boundless requests are resubmitted automatically. `manual` pricing doubles the
  offer's max price on each resubmission, capped at `4x` configured `max_price_per_mcycle`;
  min price is unchanged. `market` resubmissions are re-priced by the SDK price provider.
- `prover.sp1.cycle_limit` is the default SP1 network request cycle limit. Optional
  `prover.sp1.proposal_cycle_limit` and `prover.sp1.aggregation_cycle_limit` override it per
  stage; request-scoped `prover_args.sp1.cycle_limit` still takes precedence for compatibility.
- `rpc.client.timeout_ms` defaults to `600000` to tolerate slow preflight witness and
  `eth_getProof` RPC calls. It controls provider RPC calls, not remote prover request deadlines.
- `preflight.verify_checkpoint_l2_rpcs` is an optional map from `rpc.pairs[*].network` to a
  second L2 RPC endpoint used to cross-check the proposal boundary parent/checkpoint blocks after
  preflight. Omit a network from the map to skip that verification for the pair. The verification
  RPC uses the same `rpc.client` timeout, concurrency, and retry settings as the main preflight
  provider RPCs.
- Shasta preflight splits proposals into chunks of `8` blocks by default and runs at most `6`
  chunks concurrently. Operators may override those values with `PREFLIGHT_CHUNK_SIZE`
  (`PREFETCH_CHUNK_SIZE` is accepted for old-raiko compatibility) and
  `PREFLIGHT_CHUNK_CONCURRENCY`.
- On-the-spot local witness construction uses `WITNESS_BATCH_SIZE`, defaulting to `2` block
  witnesses at a time per preflight chunk, to keep regular geth RPC endpoints from being flooded.
- `queue.workers` defaults to `6`, aligned with the old raiko hosted proving concurrency. Each
  worker runs one queue task at a time; preflight chunk concurrency is controlled separately.
- Shasta preflight retries retryable provider/RPC/IO failures inside the preflight stage with
  exponential backoff, while invalid request/configuration and deterministic validation failures
  still fail fast. Queue tasks no longer have a global wall-clock deadline.
- `rpc.pairs[*].sp1_verifier_rpc_url` and `rpc.pairs[*].sp1_verifier_address` are optional
  pair-level settings that enable hosted `sp1.prover=network` verification through a remote
  Succinct verifier contract. Leaving them unset keeps that pair closed for hosted SP1 network
  proving. This verifier is separate from the Taiko Shasta verifier address in the chain spec.
- Queue tasks use a renewable lease for worker ownership but no global wall-clock timeout. RISC0
  network routes own retry/rebid behavior in the Boundless prover. SP1 network routes retry failed
  root tasks up to twenty times with a fixed five-minute delay.
- Boundless storage upload is environment-driven. Set `BOUNDLESS_STORAGE_UPLOADER=gcs`,
  `GCS_BUCKET=<your-gcs-bucket>`, and
  `GCS_PUBLIC_URL=false` to use a private GCS bucket. The GCP
  project is selected by gcloud/ADC, for example `<your-gcp-project>`, and is
  not a raiko2 config value. `GCS_PUBLIC_URL=false` returns private `gs://` URLs,
  so downstream downloaders need GCS credentials. Set `GCS_PUBLIC_URL=true` only
  for publicly readable buckets. `STORAGE_UPLOADER` remains accepted for
  compatibility. Optional `GCS_URL` supports custom endpoints, and
  `GCS_CREDENTIALS_JSON` can provide service account JSON when ADC is not used.
- `rpc.pairs[*].l2_witness_rpc` should ideally point to a witness-capable endpoint that supports
  `debug_executionWitness`.
- `l2_provider = "reth"` expects `debug_executionWitness` headers as RLP-encoded bytes.
  `l2_provider = "geth"` expects geth's native witness response with JSON header objects.
  `l2_provider = "geth_local_witness"` does not call `debug_executionWitness`; it executes the
  block locally against RPC-backed state to build the witness.
- Geth witness endpoints should run geth v1.17.2 or newer to include the upstream
  `debug_executionWitness` corruption fix.
- If the upstream L2 does not expose `debug_executionWitness` and predictable latency matters,
  deploy `zeth-rpc-proxy` as a compatibility layer and point `rpc.pairs[*].l2_witness_rpc` at
  that proxy.
