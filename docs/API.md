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
- `POST /v4/proof/proposal` requires `prover.submit` when a `prover.submit` or `admin`
  ACL key is configured
- `POST /v4/proof/aggregation` requires `prover.submit` when a `prover.submit` or
  `admin` ACL key is configured
- `GET /v4/tasks/{id}` requires `prover.submit` when a `prover.submit` or `admin` ACL
  key is configured
- `POST /v4/prover/clear` requires `prover.clear`
- `GET /admin/ballot` requires `admin.ballot.read`
- `POST /admin/ballot` requires `admin.ballot.write`

ACL key `id` and `key` values must be unique. ACL keys may set `rate_limit_per_minute`,
which defaults to `200` requests per minute when omitted. ACL-protected endpoints return
`429 rate_limited` after the key exceeds its 60-second request window.
When no ACL key allows `prover.submit`, the v4 submit and task lookup endpoints are public and
not rate-limited. Configure a `prover.submit` or `admin` key to protect that surface.

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

V4 is the complete explicit-proof-type interface for proposal proving, aggregation, task lookup,
status, and clear operations. A raiko2 instance serves one configured chain environment, so v4
requests do not carry chain-selection fields. V4 does not accept `zk_any`; callers must choose a
concrete proof type for each request.

V4 routes:

- `POST /v4/proof/proposal`
- `POST /v4/proof/aggregation`
- `GET /v4/tasks/{id}`
- `GET /v4/prover/status`
- `POST /v4/prover/clear`

Endpoint responsibilities:

- `POST /v4/proof/proposal` registers or polls one proposal proof task only.
- `POST /v4/proof/aggregation` registers or polls one aggregation proof task only.
- `GET /v4/tasks/{id}` is an inspection/debugging endpoint, not the taiko-client polling path.

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
  "message": "proof_type must be one of: risc0, sp1, sgx, sgxgeth"
}
```

All v4 errors return this JSON envelope with the matching HTTP status code. Clients should match the
stable snake_case `error`; `message` is diagnostic and not stable.

V4 error codes:

| Error | HTTP | Description |
| --- | --- | --- |
| `missing_proof_type` | 400 | `proof_type` is required. |
| `invalid_proof_type` | 400 | `proof_type` is syntactically invalid or a policy/fallback type such as `zk_any`. |
| `unsupported_proof_type` | 400 | The requested concrete proof type is valid v4 input but unavailable in this server configuration. |
| `invalid_request` | 400 | The request body is malformed or contains endpoint-incompatible fields. |
| `not_found` | 404 | The route exists, but the requested server feature is not enabled. |
| `task_not_found` | 404 | The task ID does not exist. |
| `unauthorized` | 401 | The required API key is missing or invalid. |
| `forbidden` | 403 | The API key is valid but is not allowed to use the required ACL feature. |
| `request_conflict` | 409 | A repeated submission conflicts with the existing root task. |
| `dependency_not_ready` | 409 | Aggregation dependencies are missing or not completed. |
| `rate_limited` | 429 | The API key exceeded its per-minute request limit. |

Common proof submission fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `proof_type` | string | yes | Concrete proof backend. One of `risc0`, `sp1`, `sgx`, `sgxgeth`. |

Common proof-type validation:

- Missing `proof_type` returns `400 missing_proof_type`.
- `proof_type=zk_any` returns `400 invalid_proof_type`.
- Unknown proof type strings return `400 invalid_proof_type`.
- Any valid concrete `proof_type` that the configured server route cannot serve returns
  `400 unsupported_proof_type`.

Proof submission validation:

- Proof submission request bodies are endpoint-specific. Legacy v3 batch fields or fields owned by
  another v4 proof endpoint return `400 invalid_request`.
- Unknown request body fields return `400 invalid_request`.

Proof submission response shape:

- `proof_type` echoes the requested concrete proof type.
- `proposal_id_start` and `proposal_id_end` echo the proposal range used as the client request
  key.
- Submission responses return one root task status. They do not embed the other proof endpoint's
  result; use `GET /v4/tasks/{id}` for the detailed task view.

### Submit Proposal Proof

```http
POST /v4/proof/proposal
Content-Type: application/json
x-api-key: <required only when server.acl has allow=["prover.submit"] or allow=["admin"]>
```

Request:

```json
{
  "proposal_id_start": 12345,
  "proposal_id_end": 12345,
  "l1_inclusion_block_number": 100,
  "l2_block_number_start": 200,
  "l2_block_number_end": 201,
  "last_anchor_block_number": 199,
  "checkpoint": null,
  "proof_type": "risc0",
  "prover": "0x0000000000000000000000000000000000000000"
}
```

Request fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `proposal_id_start` | number | yes | Taiko proposal ID to prove. Must equal `proposal_id_end`. |
| `proposal_id_end` | number | yes | Taiko proposal ID to prove. Must equal `proposal_id_start`. |
| `l1_inclusion_block_number` | number | yes | L1 block where the proposal was included. |
| `l2_block_number_start` | number | yes | First L2 block number covered by the proposal. |
| `l2_block_number_end` | number | yes | Last L2 block number covered by the proposal. |
| `last_anchor_block_number` | number | yes | Last anchor block number before the proposal range. |
| `checkpoint` | object/null | no | Fork-specific checkpoint data when required. |
| `proof_type` | string | yes | One of `risc0`, `sp1`, `sgx`, `sgxgeth`. |
| `prover` | address | no | Designated prover address. |

Response:

```json
{
  "status": "ok",
  "proof_type": "risc0",
  "proposal_id_start": 12345,
  "proposal_id_end": 12345,
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
| `proposal_id_start` | number | First proposal ID in the unique request key. |
| `proposal_id_end` | number | Last proposal ID in the unique request key. |
| `data.task_id` | string | Opaque root task ID. |
| `data.status` | string | `registered`, `work_in_progress`, `completed`, `failed`, or `cancelled`. |
| `data.proof` | string/null | Final proposal proof hex string when `data.status=completed`; null only before completion. |

Polling and idempotency:

- Clients may repeat the same `POST /v4/proof/proposal` request to poll progress. The idempotency
  key is derived only from client-supplied request data, so a server-side prover-config change (for
  example mock vs. real, or local vs. network routing) does not turn a repeated identical request
  into a conflict.
- Repeated requests for the same `(proposal_id_start, proposal_id_end, proof_type)` return the
  existing root task and current status instead of registering duplicate work.
- If the existing task for that key has terminally failed or was cancelled, a request with corrected
  proof-input fields replaces it (for example after an L1 reorg changes `l1_inclusion_block_number`).
- Otherwise, a repeated request whose proof-input fields conflict with a still-active or completed
  root task returns `409 request_conflict`.

Validation:

- `proposal_id_start` must equal `proposal_id_end` for proposal proof requests.
- `l2_block_number_start` and `l2_block_number_end` define an inclusive range and must satisfy
  `l2_block_number_start <= l2_block_number_end`.
- `proposal_id` is not accepted by the v4 proposal request.
- `proposals` is not accepted by the v4 proposal request.
- `l2_block_numbers` is not accepted by the v4 proposal request.
- `network`, `l1_network`, `blob_proof_type`, `aggregate`, `aggregation_ids`, and `proofs` are not
  accepted by the v4 proposal request.
- Invalid or unavailable proposal context returns `400 invalid_request`.

### Submit Aggregation Proof

```http
POST /v4/proof/aggregation
Content-Type: application/json
x-api-key: <required only when server.acl has allow=["prover.submit"] or allow=["admin"]>
```

Request:

```json
{
  "proposal_id_start": 12345,
  "proposal_id_end": 12346,
  "proof_type": "sp1"
}
```

Request fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `proposal_id_start` | number | yes | First proposal ID covered by the aggregation. |
| `proposal_id_end` | number | yes | Last proposal ID covered by the aggregation. |
| `proof_type` | string | yes | One of `risc0`, `sp1`, `sgx`, `sgxgeth`. |

Response:

```json
{
  "status": "ok",
  "proof_type": "sp1",
  "proposal_id_start": 12345,
  "proposal_id_end": 12346,
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
| `proposal_id_start` | number | First proposal ID in the unique request key. |
| `proposal_id_end` | number | Last proposal ID in the unique request key. |
| `data.task_id` | string | Opaque root task ID. |
| `data.status` | string | `registered`, `work_in_progress`, `completed`, `failed`, or `cancelled`. |
| `data.proof` | string/null | Final aggregation proof hex string when `data.status=completed`; null only before completion. Proposal proofs are not returned here. |

Polling and idempotency:

- Clients may repeat the same `POST /v4/proof/aggregation` request to poll progress.
- Repeated requests for the same `(proposal_id_start, proposal_id_end, proof_type)` return the
  existing root task and current status instead of registering duplicate work.

Validation:

- `proposal_id_start` and `proposal_id_end` define an inclusive contiguous range and must satisfy
  `proposal_id_start <= proposal_id_end`.
- Proposal proofs for every proposal in the range must already exist as local v4 proposal tasks for
  the selected `proof_type`.
- Each dependency is looked up by `(proposal_id, proof_type)`.
- All proposal proofs consumed by one aggregation request must use the requested `proof_type`.
- Mixed-proof-type aggregation is not supported.
- Missing or incomplete proposal proofs in the range return `409 dependency_not_ready`; the
  aggregation endpoint does not register proposal proof work.
- `aggregation_ids` is not accepted by the v4 aggregation request.
- `proofs` is not accepted by the v4 aggregation request.
- `network`, `l1_network`, `aggregate`, `proposals`, `proposal_id`, `l1_inclusion_block_number`,
  `l2_block_number_start`, `l2_block_number_end`, `last_anchor_block_number`, `checkpoint`,
  `prover`, and `blob_proof_type` are not accepted by the v4 aggregation request.

### Query V4 Task

Task lookup is an inspection/debugging endpoint. Taiko-client is expected to poll proposal and
aggregation progress by repeating proof submission requests, not by calling this endpoint.

```http
GET /v4/tasks/{id}
x-api-key: <required only when server.acl has allow=["prover.submit"] or allow=["admin"]>
```

Path fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `id` | string | yes | Opaque root task ID. |

Response:

```json
{
  "status": "ok",
  "data": {
    "task_id": "task_0x1234",
    "route": "risc0/network",
    "prover_type": "network",
    "execution_mode": "prove",
    "status": "completed",
    "network": "taiko_mainnet",
    "l1_network": "ethereum",
    "runtime": {
      "runner_status": "completed",
      "active_stage": "proposal",
      "last_event": "completed",
      "updated_at": 1742836800,
      "engine_state_present": true
    },
    "current_index": null,
    "proposals": [
      {
        "index": 0,
        "proposal_id": 12345,
        "checkpoint": null,
        "task_id": "task_...",
        "status": "completed",
        "l1_inclusion_block_number": 100,
        "l2_block_numbers": [200, 201],
        "last_anchor_block_number": 199,
        "proof": "0x...",
        "proof_ref": "proposal:...",
        "proof_path": "cache/proofs/taiko_mainnet_ethereum/proposal_....json"
      }
    ],
    "aggregate": null,
    "proof": "0x...",
    "proof_ref": "proposal:...",
    "proof_path": "cache/proofs/taiko_mainnet_ethereum/proposal_....json",
    "error": null
  }
}
```

Response fields:

| Field | Type | Description |
| --- | --- | --- |
| `data.task_id` | string | Opaque root task ID. |
| `data.route` | string | Resolved route, such as `sp1/network` or `risc0/network`. |
| `data.prover_type` | string/null | Effective prover mode: `mock`, `local`, or `network`. |
| `data.execution_mode` | string/null | SP1 execution mode: `prove` or `execute`. |
| `data.status` | string | `pending`, `proving`, `completed`, `failed`, or `cancelled`. |
| `data.network` | string | Server-configured L2 network. |
| `data.l1_network` | string | Server-configured L1 network. |
| `data.runtime` | object/null | Persisted runtime snapshot. |
| `data.current_index` | number/null | Current proposal index for multi-proposal roots. |
| `data.proposals` | array | Proposal task views. |
| `data.proposals[].l2_block_numbers` | array | L2 block numbers covered by the proposal. |
| `data.aggregate` | object/null | Aggregation task view, when the root has aggregation. |
| `data.proof` | string/null | Final root proof hex string when completed. |
| `data.proof_ref` | string/null | Stable persisted proof reference. |
| `data.proof_path` | string/null | Persisted proof artifact path. |
| `data.error` | string/null | Terminal error detail when failed. |

Validation:

- Unknown task ID returns `404 task_not_found`.
- The v4 task response does not include a top-level `proof_type`; inspect `data.route` for the
  resolved route.
- `aggregation_ids` is not returned by the v4 task response.

### Query V4 Prover Status

```http
GET /v4/prover/status?proof_type=risc0
```

Query fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `proof_type` | string | yes | One of `risc0`, `sp1`, `sgx`, `sgxgeth`. |

Response:

```json
{
  "status": "ok",
  "proof_type": "risc0",
  "data": {
    "clean": false,
    "tasks": {
      "pending": 1,
      "ready": 0,
      "retrying": 0,
      "running": 1,
      "orphaned": 0
    },
    "network": {
      "risc0": {
        "inflight_orders": 0
      },
      "sp1": {
        "inflight_orders": 0
      }
    },
    "skipped": {
      "invalid_metadata": 0,
      "unavailable_pipeline": 0,
      "remote_progress": 0
    }
  }
}
```

Response fields:

| Field | Type | Description |
| --- | --- | --- |
| `proof_type` | string | Concrete proof backend being reported. |
| `data.clean` | boolean | True when the selected proof type has no non-terminal backlog. |
| `data.tasks` | object | Local runtime and queue backlog counts for the selected proof type. |
| `data.network` | object | Remote prover-network inflight-order counts by backend. |
| `data.skipped` | object | Records skipped while collecting status because they are invalid, unavailable, or already have remote progress. |

Validation:

- Unknown query parameters return `400 invalid_request`.

### Clear V4 Prover Backlog

```http
POST /v4/prover/clear
Content-Type: application/json
x-api-key: <server.acl.keys[].key with allow=["prover.clear"] or allow=["admin"]>
```

Request:

```json
{
  "proof_type": "risc0"
}
```

Request fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `proof_type` | string | yes | One of `risc0`, `sp1`, `sgx`, `sgxgeth`. |

Response:

```json
{
  "status": "ok",
  "proof_type": "risc0",
  "data": {
    "cancelled": 2,
    "skipped": {
      "invalid_metadata": 0,
      "unavailable_pipeline": 0,
      "remote_progress": 0
    },
    "failed": 0
  }
}
```

Response fields:

| Field | Type | Description |
| --- | --- | --- |
| `proof_type` | string | Concrete proof backend targeted by the clear operation. |
| `data.cancelled` | number | Non-terminal root tasks cancelled. |
| `data.skipped` | object | Records skipped because they are invalid, unavailable, or already have remote progress. |
| `data.failed` | number | Runtime tasks marked cancelled but not fully cancelled in the engine queue. |

Idempotency:

- If the selected proof type has no non-terminal backlog, the response is `200 ok` with
  `data.cancelled=0`.

Validation:

- Unknown request body fields return `400 invalid_request`.

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
- `prover.boundless.rebid_timeout_ms` defaults to `300000` and controls how long an unlocked
  Boundless market request may remain unclaimed before `raiko2` resubmits at a higher max price.
  It must be at least `1000` ms and is separate from the overall
  `prover.boundless.timeout_ms` fulfillment deadline.
- `prover.boundless.rebid_price_multiplier` defaults to `2` and controls the manual max-price
  multiplier applied on each no-lock rebid.
- `prover.boundless.rebid_max_attempts` defaults to `4` and caps no-lock rebids. It must be no
  greater than `31`.
  `rpc.pairs[*].boundless` can override `poll_interval_ms`, `timeout_ms`, `rebid_timeout_ms`,
  `rebid_price_multiplier`, and `rebid_max_attempts` per `(network, l1_network)` pair.
- `prover.boundless.offer_params.{batch,aggregation}.pricing_mode` defaults to `manual`.
  `manual` requires `max_price_per_mcycle` and optionally accepts `min_price_per_mcycle`;
  `market` delegates price selection to the Boundless SDK price provider, may set
  `dynamic_pricing_timeout_modifier >= 1.0` to multiply `lockTimeout` and `timeout` after dynamic
  pricing, and optionally accepts `max_price_per_mcycle` as a per-mcycle safety cap. The cap value
  is multiplied by the quoted mcycle count; SDK autoprice offers whose resulting `maxPrice` exceeds
  that total cap fail before submission. `market` must omit `min_price_per_mcycle`.
- Expired Boundless requests are resubmitted automatically. `manual` pricing multiplies the
  offer's max price by `prover.boundless.rebid_price_multiplier` on each resubmission up to
  `prover.boundless.rebid_max_attempts`; min price is unchanged. `market` resubmissions are
  re-priced by the SDK price provider.
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
