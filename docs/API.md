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
- `POST /v3/proof/prune`
- `GET /v3/tasks/{id}` (`raiko2` extension)
- `POST /v3/tasks/{id}/cancel` (`raiko2` extension)
- `GET /health`
- `GET /metrics`
- `GET /ready`

The optional admin surface is disabled unless `server.admin_api_key` is configured:

- `GET /admin/ballot`
- `POST /admin/ballot`

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
x-api-key: <server.admin_api_key>
```

These endpoints mirror old `raiko` dynamic ballot control for `proof_type=zk_any`. The payload is a
JSON object whose keys are `Sp1` and `Risc0`, and whose values are `[probability, per_day]` tuples.
Only those two proof types are accepted.

## Submit Shasta Batch Proof

```http
POST /v3/proof/batch/shasta
Content-Type: application/json
```

Registers a Shasta batch root task. The server expands it into proposal prove tasks and, when
`aggregate=true`, an aggregation task.

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
  - `sp1 -> sp1/local`
  - `risc0 -> risc0/<server default runner>`
  - `zk_any -> admission-time draw to sp1 or risc0`
  - `native -> unsupported legacy error response`
  - `sgx -> unsupported legacy error response`
  - `sgxgeth -> unsupported legacy error response`
- `proof_type=zk_any` is only supported on `POST /v3/proof/batch/shasta`.
- `proof_type=zk_any` is only valid when `aggregate=false`. It is an admission-time draw for
  proposal proving. When drawn, the selected concrete proof type (`sp1` or `risc0`) is the
  canonical route, task key, and proof artifact key.
- `aggregate=true` requires a concrete `proof_type` such as `sp1` or `risc0`. Aggregate requests
  may reuse existing proposal proof artifacts for that concrete proof type.
- Boundless is the network runner for `proof_type=risc0`; it is not a separate `proof_type`.
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
- `proof_type` must be a concrete proof type: `risc0` or `sp1`.
- `proof_type=zk_any` is not supported for aggregate requests.
- `network` and `l1_network` are optional for backward compatibility with old `raiko` clients.
  When omitted, the server uses the first configured entry in `rpc.pairs` as the default pair.
  If either field is provided, both fields must be provided together.
- `proof_type=sp1` requires each proof to include `proof`, `input`, `uuid`, and `extra_data`.
- Hosted `proof_type=sp1` aggregate requests expect Compressed proposal proofs and emit a Plonk
  aggregation proof.
- `proof_type=risc0` on the hosted Boundless route requires each proof to include `quote`.
- `sp1.mode=prove` requires `sp1.verify=true` on the hosted API.
- `sp1.prover=network` with `sp1.verify=true` requires pair-level SP1 verifier config on the
  selected `(network, l1_network)`.

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
```

Removes all registered root tasks, their child engine tasks, their runtime rows, and their task
directories. Reusable proof artifacts under `cache/proofs/...` are retained.

### Response

```json
{
  "status": "ok"
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
    "route": "risc0/boundless",
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
  `native/local`, `sp1/local`, `risc0/local`, or `risc0/boundless`.
- `data.execution_mode` is present for SP1 tasks and distinguishes `prove` from `execute`.
- `data.runtime.runner_status` is the persisted root runtime lifecycle stored in `runtime.sqlite`:
  `allocated`, `running`, `completed`, `failed`, or `cancelled`.
- `data.status` is the proof-oriented root status shown to API clients:
  `pending`, `proving`, `completed`, `failed`, or `cancelled`.
- `proof_ref` and `proof_path`, when present, point at the persisted proof artifact for the
  resolved concrete route. For `zk_any` requests these fields use the selected `sp1` or `risc0`
  artifact key, never `zk_any`.
- `proposals[].runtime` and `aggregate.runtime` expose runner-specific runtime metadata when it
  exists. For `risc0/boundless`, that includes `provider_request_id`, `remote_tx_hash`,
  `expires_at`, `image_ref`, `deployment`, `offchain`, `quoted_mcycles_count`, and
  `evaluated_mcycles_count`.
- Terminal root tasks may be automatically removed from `runtime.sqlite` and `tasks/...` after
  `runtime.inactive_ttl_secs` of inactivity. Active root tasks are never removed by TTL cleanup.
  Completed proof artifacts are stored independently under `cache/proofs/...` and are indexed by
  stable proof refs, so aggregation can reuse them after engine task cleanup or process restart.
- When `data.execution_mode=execute`, proposal completion returns `proof = null` and places the
  execute report under `proposals[].extra_data.sp1`.
- `engine_state_present=false` means the API is serving the last runtime snapshot even though the
  in-memory engine no longer has a live task state object for that stage.

## Cancel Root Task

```http
POST /v3/tasks/{id}/cancel
```

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
- `rpc.pairs[*].l2_rpc` is the canonical read/state RPC used for blocks and account/state proofs.
- `rpc.pairs[*].l2_provider` selects the L2 execution-client family. It defaults to `reth`;
  set it to `geth` when `l2_rpc`/`l2_witness_rpc` points at a geth endpoint with native
  `debug_executionWitness`, or `geth_local_witness` when blocks and accounts come from a regular
  geth endpoint and witnesses must always be assembled locally.
- `rpc.pairs[*].l2_witness_rpc` is optional. When set, witness/debug traffic uses that endpoint
  while the rest of the provider keeps using `l2_rpc`.
- `prover.boundless.batch_quoted_mcycles` controls proposal quote cycles for `risc0/boundless`
  when set; `prover.boundless.aggregation_quoted_mcycles` controls aggregation quote cycles.
  `rpc.pairs[*].boundless` can override either value for one `(network, l1_network)` pair.
- `prover.sp1.cycle_limit` is the default SP1 network request cycle limit. Optional
  `prover.sp1.proposal_cycle_limit` and `prover.sp1.aggregation_cycle_limit` override it per
  stage; request-scoped `prover_args.sp1.cycle_limit` still takes precedence for compatibility.
- `rpc.client.timeout_ms` defaults to `600000` to tolerate slow preflight witness and
  `eth_getProof` RPC calls. It controls provider RPC calls, not remote prover request deadlines.
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
  still fail fast. The queue task timeout remains the outer deadline for the whole stage.
- `rpc.pairs[*].sp1_verifier_rpc_url` and `rpc.pairs[*].sp1_verifier_address` are optional
  pair-level settings that enable hosted `sp1.prover=network` verification through a remote
  verifier contract. Leaving them unset keeps that pair closed for hosted SP1 network proving.
- `queue.task_timeout_secs` defaults to `14400` and is the total deadline for each queue task,
  independent of proof type. Queue-level retry is disabled; each stage owns its own retry/resume
  behavior within this timeout, so remote proof submissions are not blindly replayed by the
  scheduler. `prove` and `aggregate` stages still use the configured queue lease and renew it while
  the worker is healthy; if a worker exits, the next lease holder resumes from persisted remote
  submission metadata or submits a fresh request when the previous remote request expired.
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
