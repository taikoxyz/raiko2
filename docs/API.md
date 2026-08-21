# Raiko2 API

## Overview

Raiko2 exposes a Shasta-first `/v4` API for explicit proof-type proposal proving,
aggregation, task lookup, status, and clear operations.

Proof submission is asynchronous. Legacy `/v3/*` and `/proof/*` compatibility routes are kept in
the codebase for short-term reference, but are not mounted by the default server router. The legacy
`POST /proof/*` responses intentionally match old `raiko` v3 response shapes and do not expose
raiko2 task IDs. When those routes are mounted in tests or a temporary compatibility build,
operators can:

- query `GET /v3/proof/report` for all root task IDs and full root-task views
- poll `GET /v3/tasks/{id}` for one full root-task view
- query `GET /v3/proof/list` for completed root tasks with final proof material

The public API surface is:

- `POST /v4/proof/proposal`
- `GET /v4/tasks/{id}`
- `GET /v4/prover/status`
- `POST /v4/prover/clear`
- `POST /v4/prover/invalidate-artifacts`
- `GET /health`
- `GET /metrics`
- `GET /ready`

ACL-protected API surface requires an `x-api-key` whose ACL allows the listed feature:

- A key with `admin` is accepted for any ACL-protected endpoint.
- `POST /v4/proof/proposal` requires `prover.submit` when a `prover.submit` or `admin`
  ACL key is configured
- `GET /v4/tasks/{id}` requires `prover.submit` when a `prover.submit` or `admin` ACL
  key is configured
- `POST /v4/prover/clear` requires `prover.clear`
- `POST /v4/prover/invalidate-artifacts` requires `prover.clear`
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

Readiness checks every configured `(network, l1_network)` pair in `rpc.pairs`, the global runtime
lifecycle and authoritative-store coherence, the in-memory queue, and the hosted proving
capabilities exposed by the endpoint. Draining or an inaccessible runtime store returns HTTP `503`.

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
- `raiko2_stage_task_failures_total`
- `raiko2_stage_task_duration_seconds`
- `raiko2_duplicate_requests_total`
- `raiko2_external_submission_total`
- `raiko2_runtime_state_serialized_bytes`
- `raiko2_runtime_state_records`
- `raiko2_runtime_retention_total`
- `raiko2_runtime_retention_retry_queue`
- `raiko2_runtime_retention_blocked`
- `raiko2_runtime_retention_attempts_total`
- `raiko2_runtime_retention_outcomes_total`

Stage metrics are labeled by `route`, `proof_type`, `pair`, `aggregate`, and `stage`.
Terminal counters and duration histograms also include `status`.
Failure counters also include a bounded `error_kind` label, such as `rpc_error`,
`witness_error`, `instance_id_mismatch`, `proof_persistence`, `stale_artifact`, or
`invalid_request`. Duplicate-request counters include `runner_status` so cache hits and stale
failed tasks can be alerted separately; completed tasks whose proof artifact is missing are
reported as `runner_status="completed_artifact_missing"`.

Runtime state metrics expose the serialized authoritative-state size and bounded record counts for
`tasks`, `artifacts`, and `pending_publications`. Runtime retention counters use only fixed outcome
labels such as `selected_tasks`, `removed_tasks`, `retained_task_failures`,
`invalidated_artifacts`, `retained_artifact_failures`, `removed_pending_publications`, and
`retained_pending_publication_failures`. Scheduler metrics use only fixed `lane`, `source`, and
`outcome` labels for the root, artifact, and pending lanes; task IDs and proof references are not
metric labels. `raiko2_runtime_retention_blocked{lane="orphan"}` reports whether the most recent
orphan pass returned an error before later retention lanes could run: it is `1` after a blocked pass
and returns to `0` after a successful pass. Alert on a sustained value; one transient pass can set it.

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
- `GET /v4/tasks/{id}`
- `GET /v4/prover/status`
- `POST /v4/prover/clear`
- `POST /v4/prover/invalidate-artifacts`

Endpoint responsibilities:

- `POST /v4/proof/proposal` registers or polls one proof task. `aggregate=false` accepts exactly
  one proposal. `aggregate=true` accepts one or more contiguous proposals and registers the
  aggregation stage for that same proposal batch.
- `GET /v4/tasks/{id}` is an inspection/debugging endpoint, not the taiko-client polling path.
- `POST /v4/prover/invalidate-artifacts` removes terminal local runtime tasks and matching proof
  artifacts for a concrete proof type.

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
  "message": "proof_type must be one of: native, risc0, sp1, sgx, sgxgeth"
}
```

The v4 proof request endpoint (`POST /v4/proof/proposal`) returns request errors with HTTP 200 and
this JSON envelope, matching the v3 polling style. Only source HTTP 400 and 409 errors are rewritten
to HTTP 200 on this proof request endpoint.
Authentication, authorization, missing-feature/not-found, rate-limit, and server-side failures keep
their HTTP status codes. Other v4 inspection/admin endpoints return this envelope with the matching
source HTTP status code. Clients should match the stable snake_case `error`; `message` is diagnostic
and not stable.

V4 error codes:

| Error | Source HTTP | Proof Request HTTP | Description |
| --- | --- | --- | --- |
| `missing_proof_type` | 400 | 200 | `proof_type` is required. |
| `invalid_proof_type` | 400 | 200 | `proof_type` is syntactically invalid or a policy/fallback type such as `zk_any`. |
| `unsupported_proof_type` | 400 or 503 | 200 or 503 | The requested concrete proof type is valid v4 input but unavailable in this server configuration, or the configured backend is unavailable. |
| `invalid_request` | 400 | 200 | The request body is malformed or contains endpoint-incompatible fields. |
| `not_found` | 404 | 404 | The route exists, but the requested server feature is not enabled. |
| `task_not_found` | 404 | 404 | The task ID does not exist. |
| `unauthorized` | 401 | 401 | The required API key is missing or invalid. |
| `forbidden` | 403 | 403 | The API key is valid but is not allowed to use the required ACL feature. |
| `request_conflict` | 409 | 200 | A repeated submission conflicts with the existing root task. |
| `rate_limited` | 429 | 429 | The API key exceeded its per-minute request limit. |
| `internal_error` | 5xx | 5xx | The server failed to process the request. |

Common proof submission fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `proof_type` | string | yes | Concrete proof backend. One of `native`, `risc0`, `sp1`, `sgx`, `sgxgeth`. |

Common proof-type validation:

- Missing `proof_type` returns `missing_proof_type`.
- `proof_type=zk_any` returns `invalid_proof_type`.
- Unknown proof type strings return `invalid_proof_type`.
- Any valid concrete `proof_type` whose self-contained prover table is disabled returns
  `unsupported_proof_type`.
- `proof_type=native` always uses `native/local`. It is intended for smoke tests and regression,
  and returns local execution output rather than a zk or TEE proof.

Proof submission validation:

- Legacy v3 fields outside the v4 request shape return `invalid_request`.
- Unknown request body fields return `invalid_request`.

Proof submission response shape:

- `proof_type` echoes the requested concrete proof type.
- `proposal_id_start` and `proposal_id_end` echo the proposal range used as the client request
  key.
- Submission responses return the root task ID, root task status, root proof, and root error. Use
  `GET /v4/tasks/{id}` for proposal and aggregate stage inspection.

### Submit Proof

```http
POST /v4/proof/proposal
Content-Type: application/json
x-api-key: <required only when server.acl has allow=["prover.submit"] or allow=["admin"]>
```

Request:

```json
{
  "proof_type": "risc0",
  "aggregate": true,
  "proposals": [
    {
      "proposal_id": 12345,
      "l1_inclusion_block_number": 100,
      "l2_block_number_start": 200,
      "l2_block_number_end": 201,
      "last_anchor_block_number": 199,
      "checkpoint": null
    },
    {
      "proposal_id": 12346,
      "l1_inclusion_block_number": 101,
      "l2_block_number_start": 202,
      "l2_block_number_end": 202,
      "last_anchor_block_number": 201
    }
  ],
  "prover": "0x0000000000000000000000000000000000000000"
}
```

Request fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `proof_type` | string | yes | One of `native`, `risc0`, `sp1`, `sgx`, `sgxgeth`. |
| `aggregate` | boolean | no | Defaults to `false`. When `false`, `proposals` must contain exactly one item. When `true`, the root task includes an aggregation stage for the submitted proposal batch. |
| `proposals` | array | yes | For `aggregate=false`, exactly one proposal. For `aggregate=true`, one or more contiguous proposals, up to 1,024 items. |
| `proposals[].proposal_id` | number | yes | Taiko proposal ID. Proposal IDs must be strictly increasing and contiguous. |
| `proposals[].l1_inclusion_block_number` | number | yes | L1 block where the proposal was included. |
| `proposals[].l2_block_number_start` | number | yes | First L2 block number covered by the proposal. |
| `proposals[].l2_block_number_end` | number | yes | Last L2 block number covered by the proposal. Must be greater than or equal to `l2_block_number_start`. |
| `proposals[].last_anchor_block_number` | number | yes | Last anchor block number before the proposal range. |
| `proposals[].checkpoint` | object/null | no | Fork-specific checkpoint data when required. |
| `prover` | address | no | Designated prover address. |

Response:

```json
{
  "status": "ok",
  "proof_type": "risc0",
  "proposal_id_start": 12345,
  "proposal_id_end": 12346,
  "data": {
    "task_id": "task_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    "status": "registered",
    "proof": null
  }
}
```

Response fields:

| Field | Type | Description |
| --- | --- | --- |
| `proof_type` | string | Concrete proof backend selected by the caller. |
| `proposal_id_start` | number | First proposal ID covered by this request. |
| `proposal_id_end` | number | Last proposal ID covered by this request. |
| `data.task_id` | string | Opaque root task ID derived from the normalized request fingerprint. Use it for task inspection and operational correlation. |
| `data.status` | string | `registered`, `work_in_progress`, `completed`, `failed`, or `cancelled`. |
| `data.proof` | string/null | Final root proof hex string when completed. For `aggregate=true`, this is the aggregation proof. A completed standalone SP1 proposal may return `null` because its canonical artifact is a typed Compressed proposal payload; use `GET /v4/tasks/{id}` to inspect its `proof_ref` and `proof_uri`. |
| `data.error` | string/null | Terminal root error detail when failed. Omitted when no error is present. |

Polling and idempotency:

- Clients may repeat the same `POST /v4/proof/proposal` request to poll progress. The idempotency
  key is derived only from client-supplied request data, so a server-side prover-config change (for
  example mock vs. real, or local vs. network routing) does not turn a repeated identical request
  into a conflict.
- Repeated requests with the same normalized request fingerprint return the existing root task and
  current status instead of registering duplicate work.
- Requests with corrected proof-input fields, for example after an L1 reorg changes
  `l1_inclusion_block_number` or `checkpoint`, get a different root task ID because the normalized
  request fingerprint changes.
- `aggregate=true` uses the same v3-style batch admission model: missing proposal proofs are
  registered as dependencies in the same request, and a recoverable repeated request may re-enqueue
  proposal or aggregation work according to the persisted root state.

Validation:

- `proposals` must not be empty.
- `proposals` may contain at most 1,024 items.
- `aggregate=false` requires exactly one proposal.
- `aggregate=true` accepts one or more contiguous proposals.
- `proposals[].proposal_id` must fit Shasta's `uint48` protocol field.
- `proposals[].proposal_id` values must be strictly increasing and contiguous.
- `proposals[].l2_block_number_end` must be greater than or equal to
  `proposals[].l2_block_number_start`.
- Each `proposals[].l2_block_number_start..=proposals[].l2_block_number_end` range may cover at
  most 100,000 L2 blocks.
- The total expanded L2 block count across all `proposals[]` entries may not exceed 100,000.
- Mixed-proof-type aggregation is not supported.
- `aggregate=true` requires a concrete proof type; `zk_any` is not accepted by v4.
- Top-level legacy fields such as `proposal_id_start`, `proposal_id_end`, `proposal_id`,
  `l2_block_number_start`, `l2_block_number_end`, `network`, `l1_network`, `blob_proof_type`,
  `aggregation_ids`, `proofs`, `graffiti`, and prover-specific argument objects are not accepted by
  the v4 proof request.
- Legacy `proposals[].l2_block_numbers` arrays are not accepted by the v4 proof request.
- Invalid or unavailable proposal context returns `invalid_request`.

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
        "proof_uri": "gs://raiko2-runtime/raiko2/runtime/v1/prod/raiko2-prod-a/proofs/shasta-sp1-local/sp1~2Fnetwork/taiko_mainnet~2Fethereum/proposal_.../content/<sha256>.proof.json"
      }
    ],
    "aggregate": null,
    "proof": "0x...",
    "proof_ref": "proposal:...",
    "proof_uri": "gs://raiko2-runtime/raiko2/runtime/v1/prod/raiko2-prod-a/proofs/shasta-sp1-local/sp1~2Fnetwork/taiko_mainnet~2Fethereum/proposal_.../content/<sha256>.proof.json",
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
| `data.execution_mode` | string/null | SP1 execution mode. Proof API tasks use `prove`; `execute` requests are rejected. |
| `data.status` | string | `pending`, `proving`, `completed`, `failed`, or `cancelled`. |
| `data.network` | string | Server-configured L2 network. |
| `data.l1_network` | string | Server-configured L1 network. |
| `data.runtime` | object/null | Persisted runtime snapshot. |
| `data.current_index` | number/null | Current proposal index for multi-proposal roots. |
| `data.proposals` | array | Proposal task views. |
| `data.proposals[].l2_block_numbers` | array | L2 block numbers covered by the proposal. |
| `data.aggregate` | object/null | Aggregation task view, when the root has aggregation. |
| `data.proof` | string/null | Final root proof hex string when completed. A completed standalone SP1 proposal may be `null` when the canonical artifact is a Compressed proposal payload. |
| `data.proof_ref` | string/null | Stable persisted proof reference. |
| `data.proof_uri` | string/null | Backend-neutral persisted proof URI (`memory://` or `gs://`). |
| `data.error` | string/null | Terminal error detail when failed. |

SP1 proposal artifacts and final aggregate artifacts have different payload contracts:

- A proposal artifact is readable when it contains a non-null `proof`, or, only for
  `PipelineKey::ShastaSp1`, when it contains the complete Compressed payload fields `quote`,
  `input`, `uuid`, and `extra_data`.
- An aggregate artifact is readable only when it contains a non-null final `proof`.
- Artifact validation is derived from the proposal or aggregate task identity. Whether the task is
  currently referenced by a standalone root, an aggregate root, or both does not change the
  accepted payload class.

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
| `proof_type` | string | yes | One of `native`, `risc0`, `sp1`, `sgx`, `sgxgeth`. |

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

Scope:

- `proof_type` selects tasks by their concrete requested backend. This includes tasks submitted through
  the v3 API (`POST /v3/proof/batch/shasta`, `POST /v3/proof/aggregate`) that resolved to the same
  concrete `proof_type`. It excludes tasks admitted as `zk_any` and drawn to this backend, which stay
  grouped under their original `zk_any` request.

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
| `proof_type` | string | yes | One of `native`, `risc0`, `sp1`, `sgx`, `sgxgeth`. |

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
| `status` | string | `ok` when every selected operation completed; `partial_failure` when `data.failed` is non-zero. |
| `proof_type` | string | Concrete proof backend targeted by the clear operation. |
| `data.cancelled` | number | Non-terminal root tasks cancelled. |
| `data.skipped` | object | Records skipped because they are invalid, unavailable, or already have remote progress. |
| `data.failed` | number | Runtime roots cancelled authoritatively whose owner-aware queue detach or exact artifact effect did not yet converge. |

Idempotency:

- If the selected proof type has no non-terminal backlog, the response is `200 ok` with
  `data.cancelled=0`.
- Cancellation commits the exact non-terminal root lifetime first, then atomically detaches that
  root owner from the in-process execution projection. Shared stages remain while another live root
  owns them; the last owner leaving cancels or removes the stage.
- The endpoint returns completed counts even when a later projection or artifact effect fails. In
  that case the envelope uses `status="partial_failure"`; clients may retry safely while background
  reconciliation converges the already-cancelled root.

Validation:

- Unknown request body fields return `400 invalid_request`.

Scope:

- `proof_type` cancels tasks by their concrete requested backend. This includes non-terminal tasks
  submitted through the v3 API (`POST /v3/proof/batch/shasta`, `POST /v3/proof/aggregate`) that
  resolved to the same concrete `proof_type`. It excludes tasks admitted as `zk_any` and drawn to this
  backend. If a temporary legacy v3 build admits `zk_any` tasks, they remain grouped under their
  original `zk_any` request and are cleared through the legacy `POST /v3/prover/clear` route in that
  legacy build.

### Invalidate V4 Proof Artifacts

```http
POST /v4/prover/invalidate-artifacts
Content-Type: application/json
x-api-key: <server.acl.keys[].key with allow=["prover.clear"] or allow=["admin"]>
```

Request:

```json
{
  "proof_type": "sgxgeth",
  "proof_prefix": "0x00000005",
  "proposal_id_start": 15950,
  "proposal_id_end": 15980,
  "dry_run": true
}
```

Request fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `proof_type` | string | yes | One of `native`, `risc0`, `sp1`, `sgx`, `sgxgeth`. |
| `proof_prefix` | string | no | Optional `0x`-prefixed hex prefix matched against cached proof payloads. Maximum length is 130 characters, including `0x`. This is useful for invalidating stale SGX instance-id prefixes after verifier rotation. |
| `proposal_id_start` | number | no | Inclusive proposal-id range start. Must be provided with `proposal_id_end`. |
| `proposal_id_end` | number | no | Inclusive proposal-id range end. Must be provided with `proposal_id_start`. |
| `dry_run` | boolean | no | Defaults to `false`. When `true`, reports matches without deleting runtime tasks, engine children, proof-artifact rows, or proof manifests. |

Response:

```json
{
  "status": "ok",
  "proof_type": "sgxgeth",
  "data": {
    "dry_run": true,
    "artifacts": {
      "matched": 10,
      "removed": 0,
      "manifests_removed": 0,
      "manifests_missing": 0,
      "failed": 0
    },
    "tasks": {
      "matched": 10,
      "removed": 0,
      "skipped_non_terminal": 0,
      "invalid_metadata": 0,
      "failed": 0
    }
  }
}
```

`manifests_removed` and `manifests_missing` report the exact result of deleting the canonical
manifest. Immutable content objects are retained and are not counted as removed files.
The response envelope uses `status="partial_failure"` when either `data.artifacts.failed` or
`data.tasks.failed` is non-zero; clients must not treat that response as a complete invalidation.

Scope:

- The endpoint invalidates completed, failed, or cancelled local runtime tasks and matching proof
  artifacts for the selected concrete proof type. It also removes completed child tasks from the local
  engine so a retried aggregate does not reuse stale child-proof state.
- It does not cancel or delete non-terminal tasks. Use `POST /v4/prover/clear` first if active work
  must be cancelled.
- If no proposal range is supplied, all terminal tasks and matching proof artifacts for `proof_type`
  are selected. If a proposal range is supplied, standalone proof artifacts are selected only when they
  are linked to a matched runtime task.
- If deleting a proof manifest fails, `failed` is incremented and the artifact remains
  tombstoned for a later invalidation retry. Tombstoned artifacts are not eligible for proof reuse.
- When `dry_run=false`, runtime state reserves invalidation for the exact artifact descriptor before
  any object-store effect. A matching live root returns a blocked result and retains its artifact.
  An accepted reservation creates a tombstone for `(logical key, manifest generation, content
  hash)`, removes only that manifest generation conditionally, and cleans up only the matching
  pending publication. Immutable content bytes are retained. A dry run performs selection and
  validation without reserving state or mutating objects.

Validation:

- Unknown request body fields return `400 invalid_request`.
- `proposal_id_start` and `proposal_id_end` must be supplied together, and start must be less than or
  equal to end.
- `proof_prefix`, when provided, must be a non-empty `0x`-prefixed hex string no longer than 130
  characters including `0x`. Prefix matching validates the complete manifest-selected content hash
  before examining the bounded prefix; corrupt or missing content fails the request instead of being
  treated as a non-match.

## Legacy V3 Submit Shasta Batch Proof

This legacy route is not mounted by the default server router.

```http
POST /v3/proof/batch/shasta
Content-Type: application/json
```

Registers a Shasta batch root task. The server expands it into proposal prove tasks and, when
`aggregate=true`, an aggregation task. Configure remote lanes independently through `[prover.sgx]`
and `[prover.sgxgeth]`; each table owns its `enabled`, `base_url`, and `timeout_ms` values.

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
  - `native -> native/local` when `prover.native.enabled = true`; otherwise rejected
  - `sp1 -> sp1/local | sp1/network` from `prover.sp1.enabled` and `prover.sp1.prover`
  - `risc0 -> risc0/local | risc0/network` from `prover.risc0.enabled` and
    `prover.risc0.runner`, with
    `prover_type = mock | local | network`
  - `zk_any -> admission-time draw to sp1 or risc0`
  - `sgx -> sgx/remote` backed by `raiko2-sgx-prover`
  - `sgxgeth -> sgxgeth/remote` backed by the external geth-backed remote SGX server
  - `boundless -> unsupported legacy error response`
- Each concrete proof type is accepted only when its matching route is enabled.
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
- `sp1.mode=execute` is rejected by the proof API because it does not produce the publishable
  Compressed proposal or final aggregate payload required by the proof lifecycle.
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

## Legacy V3 Submit Aggregate Proof

This legacy route is not mounted by the default server router.

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
- When provided, `aggregation_ids` must contain exactly one ID per entry in `proofs`.
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

## Legacy V3 Report All Root Tasks

These legacy routes are not mounted by the default server router.

```http
GET /v3/proof/report
GET /proof/report
```

Returns an array of root-task views in the same shape as `GET /v3/tasks/{id}` `data`, one entry
per registered root task.

## Legacy V3 List Completed Root Proofs

These legacy routes are not mounted by the default server router.

```http
GET /v3/proof/list
GET /proof/list
```

Returns only root-task views whose root status is `completed` and whose final `proof` field is
present.

## Legacy V3 Prune All Root Tasks

These legacy routes are not mounted by the default server router.

```http
POST /v3/proof/prune
POST /proof/prune
x-api-key: <server.acl.keys[].key with allow=["admin"]>
```

Requires an ACL key that allows `admin`.

Removes all registered root tasks and detaches their owner-aware in-process execution projections.
Reusable proof artifacts in the configured artifact store are retained.

### Response

```json
{
  "status": "ok"
}
```

## Legacy V3 Query Prover Status

This legacy route is not mounted by the default server router.

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
remote submission progress. The runtime cleanup pass cancels stale orphaned records after the
seven-day runtime retention window.
`clean=true` means there are no matching non-terminal queue tasks in `pending`, `ready`,
`retrying`, `running`, or `orphaned` state, no resumable SP1 or RISC0 network submissions, and
no skipped non-terminal roots with invalid metadata or unavailable pipelines.

## Legacy V3 Clear Prover

This legacy route is not mounted by the default server router.

```http
POST /v3/prover/clear
x-api-key: <server.acl.keys[].key with allow=["prover.clear"] or allow=["admin"]>
```

Requires an ACL key that allows `prover.clear` or `admin`. Marks every exact non-terminal root
originally submitted with `proof_type=zk_any` as `cancelled` first, then detaches its owner from the
proposal and aggregation execution graph.

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

`status` is `partial_failure` when `failed` is non-zero; callers must retry or alert on those
incomplete projection or artifact effects. The authoritative root remains cancelled while
reconciliation retries the effect.

## Legacy V3 Query Root Task

This legacy route is not mounted by the default server router.

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
        "proof_uri": "gs://raiko2-runtime/raiko2/runtime/v1/devnet/raiko2-devnet-a/proofs/shasta-sp1-local/sp1~2Fnetwork/taiko_hoodi~2Fhoodi/proposal_.../content/<sha256>.proof.json"
      }
    ],
    "aggregate": {
      "task_id": "...",
      "status": "completed",
      "proof": "0x...",
      "proof_ref": "aggregate:...",
      "proof_uri": "gs://raiko2-runtime/raiko2/runtime/v1/devnet/raiko2-devnet-a/proofs/shasta-sp1-local/sp1~2Fnetwork/taiko_hoodi~2Fhoodi/aggregate_.../content/<sha256>.proof.json"
    },
    "proof": "0x...",
    "proof_ref": "aggregate:...",
    "proof_uri": "gs://raiko2-runtime/raiko2/runtime/v1/devnet/raiko2-devnet-a/proofs/shasta-sp1-local/sp1~2Fnetwork/taiko_hoodi~2Fhoodi/aggregate_.../content/<sha256>.proof.json"
  }
}
```

### Runtime Semantics

- `data.route` is the canonical resolved route that accepted the request, such as
  `native/local`, `sp1/local`, `sp1/network`, `risc0/local`, or `risc0/network`.
- `data.prover_type` is present for zkVM proof types and reports the effective prover mode:
  `mock`, `local`, or `network`. For RISC0, the network mode is currently backed by Boundless.
- `data.execution_mode` is present for SP1 tasks and distinguishes `prove` from `execute`.
- `data.runtime.runner_status` is the root runtime lifecycle stored by the configured runtime store:
  `allocated`, `running`, `completed`, `failed`, or `cancelled`.
- `data.status` is the proof-oriented root status shown to API clients:
  `pending`, `proving`, `completed`, `failed`, or `cancelled`.
- `proof_ref` and `proof_uri`, when present, point at the persisted proof artifact for the
  resolved concrete route. For `zk_any` requests these fields use the selected `sp1` or `risc0`
  artifact key, never `zk_any`.
- A standalone SP1 proposal can be `completed` with `data.proof = null` when `proof_ref` and
  `proof_uri` identify a readable Compressed proposal artifact. Proposal entries in an aggregate
  task use the same contract. The aggregate root itself becomes `completed` only after a separate
  final artifact with a non-null `proof` is readable.
- `proposals[].runtime` and `aggregate.runtime` expose runner-specific runtime metadata when it
  exists. For `risc0/network`, that includes `provider_request_id`, `remote_tx_hash`,
  `expires_at`, `image_ref`, `deployment`, `offchain`, `quoted_mcycles_count`, and
  `evaluated_mcycles_count`.
- Terminal root task records use the configurable `runtime.terminal_task_ttl_secs` retention policy,
  which defaults to six hours. Artifact manifests and pending publication intents, including
  external aggregation inputs, are reclaimed independently when no runtime task record references
  them; object-store failure never retains a successfully detached root. Active manifests must not have a
  bucket age rule, and immutable proof or program content must remain available until every manifest
  that references it is gone. Retention admission preserves the terminal runner status, proof URI,
  error, and timestamp until exact task removal commits. Generation-scoped tombstones and unreferenced
  content use a minimum thirty-day retention window. Active root tasks are never removed by terminal
  cleanup. The terminal TTL also applies to failed or cancelled roots that carry remote submission
  progress. Once that checkpoint expires, a later resubmission may create and pay for a new provider
  request even if the old provider request eventually completes.
- `engine_state_present=false` means the API is serving the last runtime snapshot even though the
  in-memory engine no longer has a live task state object for that stage.

## Legacy V3 Cancel Root Task

This legacy route is not mounted by the default server router.

```http
POST /v3/tasks/{id}/cancel
x-api-key: <server.acl.keys[].key with allow=["admin"]>
```

Requires an ACL key that allows `admin`.

Cancelling a root task first marks the exact non-terminal runtime lifetime `cancelled`, then detaches
that root owner from its proposal and optional aggregation graph. Shared child tasks remain for any
other live owner. A failed detach does not roll the root back; the request returns an error and
reconciliation retries the incomplete effect.

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

Each proof type owns a self-contained table and an explicit `enabled` value. RISC0 selects
`runner = "local" | "network"`; SP1 derives local/network from `prover = "local" | "mock" |
"network"`; native is always local; SGX and SGXGETH are always remote and each owns its endpoint
and timeout. Native is enabled by default and can be disabled explicitly. One host may enable any
supported combination, with at least one proof type required.
Boundless is nested under `[prover.risc0.boundless]` because it is the RISC0 network backend.

`--prover-routes` and `RAIKO2_PROVER_ROUTES` accept a comma-separated list such as
`risc0/network,sp1/network,sgx/remote,sgxgeth/remote`. The operational override atomically disables
omitted proof types, enables the listed proof types, and updates the RISC0/SP1 execution selectors;
it never appends to the file configuration.

`--remote-sgx-timeout-ms` and `RAIKO2_REMOTE_SGX_TIMEOUT_MS` are shared operational overrides that
set both SGX lane timeouts. Use the independent `prover.sgx.timeout_ms` and
`prover.sgxgeth.timeout_ms` file values when the lanes need different timeouts.

- A string setting may use the explicit TOML reference `{ env = "NAME" }`. Raiko2 resolves only
  that singleton table before schema validation; missing, non-Unicode, or empty variables fail
  startup without printing their values. Shell expansion and partial-string interpolation are not
  supported. Schema error details are redacted when a file contains an environment reference.
- `runtime.environment` is the business/deployment boundary. `runtime.namespace` is the immutable
  single-instance persistence boundary. Namespaces do not share data; roots inside one namespace may
  reuse one canonical proof artifact. Both values scope request fingerprints, public task IDs,
  runtime records, provider checkpoints, and proof artifacts.
- `runtime.terminal_task_ttl_secs` controls how long terminal task metadata remains eligible for
  reuse before background root retirement. It defaults to `21600` (6 hours), must be greater than
  zero, and does not expire active tasks. Proof artifacts and pending publication intents use
  ownership-driven cleanup rather than this TTL.
- `runtime.cleanup_interval_secs` independently paces runtime retention passes and defaults to `30`.
  `runtime.cleanup_batch_size` bounds each root, artifact, pending-publication, and overdue-active
  retention lane to `64` records by default and accepts values from `1` through `1024`. The separate
  seven-day orphan-management pass processes at most `min(runtime.cleanup_batch_size, 64)` records
  because it performs per-task lifecycle mutations. A persistent orphan reconciliation or lifecycle
  error intentionally blocks later retention lanes until external repair or one-shot proof startup
  cleanup; ordinary task failures do not enter this path. Neither setting reuses queue maintenance
  timing. Overdue-active warnings retain only the first 4096 task incarnations observed by one
  process, so warning coverage resets on restart after that bounded set saturates.
- `runtime.preflight_cache` accepts `"shared"` (default) or `"off"`. Shared mode enables the
  persistent canonical preflight cache and process-local singleflight across proof lanes. Off mode
  bypasses both layers and rebuilds preflight independently for each request; use it only as an
  incident-response control while preserving proof storage and runtime state.
- `runtime.startup_cleanup` defaults to an empty list. `["proof"]` clears authoritative runtime task
  state first and then deletes active proposal and aggregate proof manifests. Use it for SGX/ZK guest,
  image, verifier, or proving-key changes. `["preflight"]` deletes only active canonical preflight
  manifests. Use `["proof", "preflight"]` when derivation, fork, or witness-generation rules changed.
  The scopes are exact: neither implies the other, and there is no `input` scope because materialized
  `GuestInput` values are not persisted. GCS pages through matching manifest objects and deletes their
  listed generations with bounded concurrency; immutable proof/preflight content and invalidation
  records remain unreachable until bucket lifecycle TTL removes them. Any listing or deletion failure
  aborts startup. Cleanup runs before recovery, workers, or HTTP admission and only after the previous
  process has stopped. Configured scopes run again on every restart, so remove `startup_cleanup`
  immediately after the cutover succeeds; leaving it configured can discard fresh task state and
  proof manifests during a routine restart. A missing list means no cleanup; duplicate scopes,
  unknown scopes, and the removed reset boolean fail schema validation. Sibling namespaces are never
  affected.
- `runtime.store.backend` selects the backend used by both the authoritative state repository and
  proof-object repository. Use `gcs` with a non-empty `bucket` for durable deployments. `memory`
  is process-local and disposable; it is accepted outside `development`, `local`, or `test` only
  when `runtime.store.allow_ephemeral = true`. Switching backends is a drain-and-cutover operator
  action and there is no automatic failover, merge, writeback, SQLite import, or compatibility
  migration.
- GCS object names start with `<prefix>/<environment>/<namespace>/`. The service intentionally has no
  distributed owner lease, owner epoch, or ownership heartbeat. Deployment must guarantee that old
  and replacement processes never overlap for one namespace. Configure prefixes so one deployment
  scope cannot be nested below another deployment's `(prefix, environment, namespace)` scope.
- Canonical Shasta preflight cores are keyed by chain/range/proposal/L1 inclusion and effective rule
  identity, then shared across all proof lanes in one process. Proof type, verifier addresses,
  request presentation fields, and RPC endpoints are not part of that cache identity. Cache hits are
  revalidated with guest-equivalent Shasta semantics before lane-specific `GuestInput` materialization.
- The namespace fence is global to the process but short-lived: draining closes mutation admission
  and readiness immediately, then waits only for repository commits already admitted and provider
  request-ID checkpoints covered by existing permits. It is not held across a full task,
  cancellation, or publication saga, and shutdown does not wait for all proof tasks to finish.
- Runtime state is authoritative. Submission attaches a complete owner-aware execution graph only
  after root registration; cancellation, terminal failure, and cleanup transition the exact
  `TaskLifetime` before detaching its root owner. Projection failures are reconciled from runtime
  state and do not roll an authoritative transition back.
- Proof bytes are immutable `*.proof.json` objects selected by a create-only `*.manifest.json`
  pointer. Runtime snapshots use `*.runtime.json`; the suffixes let operations distinguish
  authoritative state, active manifests, generation-scoped tombstones, and unreferenced content.
  Terminal roots use the configurable six-hour default in runtime state. Tombstones and unreferenced
  content use a minimum thirty-day object lifecycle, while active manifests must not be deleted by
  age and immutable content must remain available while any active manifest references it.
- A proof task reports `completed` only after its normalized `Proof` artifact is durably published,
  registered, readable, and satisfies its task-identity payload contract. Proposal tasks accept a
  non-null `proof`, plus the complete Compressed SP1 tuple (`quote`, `input`, `uuid`, `extra_data`)
  on `ShastaSp1`; aggregate tasks require a non-null final `proof`. Publication is create-only: an
  identical existing object is idempotent, while a different late object is discarded.
- Proof artifact identity includes the concrete execution route. In particular, `sp1/local` and
  `sp1/network` use different objects even though they share `PipelineKey::ShastaSp1`.
- Before the first canonical publication attempt, a completed proof payload is checkpointed under
  the queue lease. Authoritative runtime state first records a publication intent containing the
  typed artifact identity, content hash, and exact `TaskLifetime` owners; only then is the immutable
  pending blob materialized. State updates do not rewrite large proofs. A failed intent write creates
  no object, while a failed blob write leaves a durable retry intent. Recovery uses the same intent
  and pending blob after restart, and in-process publication retries do not run the prover again.
- Invalidation first reserves the exact artifact expectation in authoritative runtime state and
  verifies that no live matching owner remains. It then writes a tombstone for `(logical key,
  manifest generation, content hash)`, conditionally deletes only that manifest generation, and
  removes only the exact pending blob. Immutable proof content is retained. Recovery checks the
  exact marker during reconciliation and publication finalization, so a failed manifest deletion
  cannot make that descriptor reusable. A later lifecycle may publish identical or different
  content under a new manifest generation.
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
- `prover.risc0.boundless.batch_quote` and `prover.risc0.boundless.aggregation_quote` select how
  proposal and aggregation quote cycles are sized for `risc0/network`. Each is a table with
  `strategy = "raiko_agent"` (default; rounds the evaluated dry-run mcycle count up locally — batch
  to the next `1000` mcycles with a `2000` mcycle floor, aggregation to the next `100` mcycles with a
  `200` mcycle floor), `"evaluated"` (use the local dry-run mcycle count as-is), or `"fixed"` with a
  positive `mcycles` value.
  `rpc.pairs[*].boundless` can override either table for one `(network, l1_network)` pair.
- `prover.risc0.boundless.rebid_timeout_ms` defaults to `300000` and controls how long an unlocked
  Boundless market request may remain unclaimed before `raiko2` resubmits at a higher max price.
  It must be at least `1000` ms and is separate from the overall
  `prover.risc0.boundless.timeout_ms` fulfillment deadline.
- `prover.risc0.boundless.rebid_price_step_bps` defaults to `5000` (+50% per rung) and sets the
  per-rebid max-price escalation, in basis points, compounded over the offer's base max price
  (`1 → 1.5 → 2.25 → 3.375×`). `0` is a valid flat (no-escalation) ladder; any value in `1..100`
  is rejected as a likely basis-points/multiplier confusion (a `2` meant as "×2" is really
  +0.02%/rung). `manual` pricing escalates the configured max price; `market` pricing escalates
  the SDK autopriced max price, still subject to the optional cap.
- `prover.risc0.boundless.rebid_max_attempts` defaults to `4` and caps rebids across every retry
  path: no-lock, expired, and timed-out requests all draw from the same submission budget, and
  the proof task fails once it is exhausted. It must be no greater than `31`.
  `rpc.pairs[*].boundless` can override `poll_interval_ms`, `timeout_ms`, `rebid_timeout_ms`,
  `rebid_price_step_bps`, and `rebid_max_attempts` per `(network, l1_network)` pair.
- `prover.risc0.boundless.offer_params.{batch,aggregation}.pricing_mode` defaults to `manual`.
  `manual` requires `max_price_per_mcycle` and optionally accepts `min_price_per_mcycle`;
  `market` delegates price selection to the Boundless SDK price provider and optionally accepts a
  per-mcycle safety cap spelled either `absolute_max_price_per_mcycle` (canonical) or
  `max_price_per_mcycle` (legacy alias, same meaning) — setting both is rejected. The cap value
  must be positive and is multiplied by the quoted mcycle count; offers whose (possibly
  rebid-escalated) `maxPrice` exceeds that total cap are clamped to it instead of failing, with the
  min price lowered to the cap when needed to keep the offer well-formed. `market` must omit
  `min_price_per_mcycle`.
- `prover.risc0.boundless.offer_params.{batch,aggregation}.absolute_max_price_per_mcycle` is the
  absolute per-mcycle bid ceiling in both pricing modes: no attempt, initial or rebid-escalated,
  ever bids above it. In `manual` mode it is optional, must be at least `max_price_per_mcycle`, and
  clamps the bps rebid escalation; without it, manual escalation is unbounded by config. In
  `market` mode it is the canonical spelling of the safety cap. Once a rebid is clamped, later
  rebids repeat the ceiling price.
- `prover.risc0.boundless.offer_params.{batch,aggregation}.timeouts` is a tagged table selecting the
  timeout policy. `mode = "per_mcycle"` sets `lock_timeout_ms_per_mcycle` and
  `timeout_ms_per_mcycle` (scaled by the quoted mcycle count) and, under `market` pricing only, may
  set `dynamic_pricing_timeout_modifier >= 1.0` to multiply `lockTimeout` and `timeout` after
  dynamic pricing. `mode = "fixed"` sets `lock_timeout_secs` and `timeout_secs` directly.
- `prover.risc0.boundless.offer_params.{batch,aggregation}.ramp_up_period_sec` is the offer ramp-up
  duration in seconds (previously `ramp_up_period_blocks`, scaled by a per-deployment block time).
- Boundless offer tables reject unknown keys, so a stale offer-level field left over from the
  pre-cutover schema — for example `dynamic_pricing_timeout_modifier` at the offer level instead of
  inside `timeouts` — fails to boot rather than being silently ignored. Keys nested one level
  deeper, inside the tagged `timeouts` / `*_quote` tables, are **not** rejected (a serde limitation
  on internally-tagged enums), so double-check those tables by hand during migration.
- Expired Boundless requests are resubmitted automatically up to the shared
  `prover.risc0.boundless.rebid_max_attempts` budget, each resubmission escalating the max price by
  `prover.risc0.boundless.rebid_price_step_bps` (compounded), clamped to
  `absolute_max_price_per_mcycle` when it is set; min price is unchanged. `market` resubmissions are
  re-priced by the SDK price provider and then escalated by the same step, subject to the cap.
- `prover.sp1.cycle_limit` is the default SP1 network request cycle limit. Optional
  `prover.sp1.proposal_cycle_limit` and `prover.sp1.aggregation_cycle_limit` override it per
  stage; request-scoped `prover_args.sp1.cycle_limit` still takes precedence for compatibility.
- `prover.sp1.network_request_max_attempts` bounds the full SP1 network lifecycle, including a
  request resumed after restart. Exhausting the budget fails the task instead of submitting
  requests indefinitely. A request-scoped `prover_args.sp1.network_request_max_attempts` may lower
  this operator-owned cap but cannot raise it.
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
- Boundless storage upload is environment-driven and required whenever a Boundless network route
  is enabled. Set `BOUNDLESS_STORAGE_UPLOADER=gcs`,
  `GCS_BUCKET=<your-gcs-bucket>`, and
  `GCS_PUBLIC_URL=false` to use a private GCS bucket. The GCP
  project is selected by gcloud/ADC, for example `<your-gcp-project>`, and is
  not a raiko2 config value. `GCS_PUBLIC_URL=false` returns private `gs://` URLs,
  so downstream downloaders need GCS credentials. Set `GCS_PUBLIC_URL=true` only
  for publicly readable buckets. `STORAGE_UPLOADER` remains accepted for
  compatibility. Optional `GCS_URL` supports custom endpoints, and
  `GCS_CREDENTIALS_JSON` can provide service account JSON when ADC is not used.
  S3 support is excluded from default host builds; build `raiko2` with the
  non-default `boundless-s3` feature before selecting `BOUNDLESS_STORAGE_UPLOADER=s3`.
  An explicit unsupported S3 selection fails during host startup. When no uploader is explicitly
  selected, GCS, Pinata, and File settings take precedence over `S3_BUCKET`. An S3-only implicit
  configuration still fails startup in a default build and requires the `boundless-s3` feature.
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
