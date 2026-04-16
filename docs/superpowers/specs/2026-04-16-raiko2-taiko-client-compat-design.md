# Raiko2 Compatibility Design for Existing taiko-client POST Polling

## Summary

`raiko2` currently exposes an asynchronous `v3` API:

- `POST /v3/proof/batch/shasta` registers a root task and returns `registered + task_id`
- `GET /v3/tasks/{id}` returns the evolving task state and final proof material

The existing `taiko-client` Shasta proving flow does not follow the task API. It repeatedly sends the
same `POST /v3/proof/batch/shasta` request and expects that endpoint to behave like a proof-oriented
polling interface:

- first response: `registered`
- subsequent responses while still running: `work_in_progress`
- terminal success: `completed + proof`
- terminal failure: top-level `error/message`

This design makes `raiko2` transparently compatible with that behavior without requiring any
`taiko-client` change and without changing the `/v3/tasks/{id}` contract.

## Goals

- Make the existing `taiko-client` Shasta proving flow work against `raiko2` without client changes.
- Keep `/v3/tasks/{id}` as the canonical async inspection API.
- Prevent duplicate root-task registration for repeated identical `POST /v3/proof/batch/shasta`
  requests.
- Make duplicate-request handling safe under concurrent identical submissions.

## Non-Goals

- Changing `taiko-client`.
- Changing engine task IDs, queue semantics, worker execution order, or proof generation logic.
- Changing the `/v3/tasks/{id}` response shape.
- Introducing a new legacy endpoint or a compatibility-only route.
- Extending this change to non-existent batch endpoints outside the current Shasta route.

## Approaches Considered

### 1. Transparent compatibility on the existing `POST /v3/proof/batch/shasta` route

Chosen.

The existing route becomes an idempotent register-or-query endpoint for the same canonical request:

- first request registers the root task
- repeated identical requests return a legacy-compatible projection of the existing root task

This matches the behavior expected by the existing `taiko-client` and requires no client changes.

### 2. Compatibility behind a config flag or request header

Rejected.

This keeps the current async semantics cleaner but does not satisfy the operational requirement of
making the existing `taiko-client` work unchanged.

### 3. Add a new legacy endpoint

Rejected.

The existing `taiko-client` already calls `POST /v3/proof/batch/shasta`. A new endpoint still
requires client changes and does not solve the immediate compatibility problem.

## Chosen Design

### High-Level Behavior

`POST /v3/proof/batch/shasta` becomes an idempotent endpoint keyed by a canonical request
fingerprint.

For a given canonical request:

- if no matching root task exists, `raiko2` registers and enqueues it exactly once
- if a matching root task already exists, `raiko2` does not create a new task and instead returns a
  legacy-compatible proof response derived from the current root-task state

The existing `/proof/batch/shasta` alias inherits the same behavior because it uses the same handler.

### Legacy-Compatible Response Projection

Repeated identical `POST` requests must return a response shape that the existing `taiko-client`
already understands.

The projection uses the existing top-level fields:

- `status`
- `proof_type`
- `data.status`
- `data.proof`

The compatibility response shape is:

```json
{
  "status": "ok",
  "proof_type": "risc0",
  "data": {
    "status": "completed",
    "proof": {
      "proof": "0x...",
      "kzg_proof": "",
      "quote": ""
    }
  }
}
```

`data.proof` remains an object, not a bare string, because the existing `taiko-client` expects
`data.proof.proof`.

### Status Mapping

For a repeated identical `POST /v3/proof/batch/shasta`, `raiko2` maps the existing root-task state
into one of the following compatibility responses.

#### Newly registered / not yet progressed

Return:

- top-level `status = "ok"`
- `data.status = "registered"`
- optional `task_id`

This preserves the current first-response behavior and remains compatible with the old client retry
logic.

#### In progress

Return:

- top-level `status = "ok"`
- `data.status = "work_in_progress"`

This must be used once the request has observable runtime or engine progress.

`pending` and `proving` from the task API are internal status vocabulary. They are not returned by
the compatibility projection because the old client specifically recognizes `work_in_progress`.

#### Completed

Return:

- top-level `status = "ok"`
- `data.status = "completed"`
- `data.proof` as an object

Proof selection rules:

- when `aggregate=true`, return the aggregate proof
- otherwise, return the root proof if present
- otherwise, for a single-proposal Shasta task, fall back to `proposals[0].proof`

`proof_type` must be the resolved canonical proof type from the route or stored task metadata, not
the raw request value prior to `zk_any` resolution.

#### Failed or cancelled

Return a top-level error response:

- top-level `status = "error"`
- top-level `proof_type = <resolved proof type>`
- top-level `error` and/or `message` populated from the root-task error

Failure is intentionally not encoded as `data.status = "failed"` because the old client already uses
top-level `error/message` for terminal failure handling.

#### `zk_any_not_drawn`

The current behavior remains unchanged:

```json
{
  "status": "ok",
  "proof_type": "native",
  "data": {
    "status": "zk_any_not_drawn"
  }
}
```

No idempotent root-task lookup is performed for not-drawn requests because no root task exists.

## Canonical Request Fingerprint

### Purpose

The fingerprint identifies whether two incoming Shasta batch requests are semantically the same task
for compatibility and idempotency purposes.

### Rules

The fingerprint must be computed only after request validation and canonical route resolution.

It must include every normalized field that can affect:

- route selection
- proof semantics
- task ownership or public task metadata
- final proof contents or proving mode

The fingerprint input includes:

- resolved network pair key
- resolved canonical route
- aggregate flag
- resolved execution mode
- blob proof type
- prover
- graffiti
- normalized public prover args / overrides
- normalized proposals
  - proposal ID
  - checkpoint
  - L1 inclusion block number
  - L2 block numbers
  - last anchor block number

The fingerprint must not include:

- generated public task IDs
- timestamps
- runtime metadata
- engine task IDs

### `zk_any` Rule

`zk_any` must be resolved before fingerprinting.

The fingerprint stores the final canonical route or proof type selected by the draw. It must not use
the raw `proof_type=zk_any` input. This prevents one drawn request from accidentally colliding with a
different resolved route.

## Runtime Persistence and Idempotent Registration

### New Root-Task Field

Add an optional `request_fingerprint` field to runtime root tasks.

Only root tasks created from `POST /v3/proof/batch/shasta` use this field in this change. Other task
types may leave it `NULL`.

### Schema Change

Extend `runtime_tasks` with:

- `request_fingerprint TEXT`

Create a partial unique index:

```sql
CREATE UNIQUE INDEX IF NOT EXISTS runtime_tasks_request_fingerprint_uq
ON runtime_tasks(request_fingerprint)
WHERE request_fingerprint IS NOT NULL;
```

### Migration Requirement

`RuntimeManager` currently only initializes the table if it does not exist. This change therefore
requires an explicit startup migration path that:

- adds `request_fingerprint` if the column is missing
- creates the partial unique index if it does not exist

The migration must be idempotent across restarts.

### Required Runtime API

The runtime layer needs a real idempotent registration helper instead of a handler-only
check-then-insert pattern.

Required behavior:

- insert a new root task only if the fingerprint is not already present
- if the fingerprint already exists, return the existing root task instead of creating a duplicate
- avoid leaving stray task directories or request files on uniqueness conflicts

This is important because the current `register_task` path creates the task workspace before writing
the SQLite row. A uniqueness conflict that is detected only after filesystem side effects would leave
orphaned task directories.

The runtime helper may be expressed as:

- `register_task_if_absent`
- `register_root_task_by_fingerprint`
- or an equivalent API

The exact naming is not important. The behavior is.

## Handler Flow

### `request_batch_shasta_proof`

The handler flow becomes:

1. Parse and validate the request.
2. Build `CanonicalBatchSubmission`.
3. Resolve `zk_any` if present.
4. Compute `request_fingerprint`.
5. Ask runtime for an existing root task by that fingerprint.
6. If one exists:
   return the compatibility projection immediately.
7. Otherwise, register the new root task through the runtime idempotent registration helper.
8. If runtime reports that another identical request already won the race:
   load the existing task and return the compatibility projection.
9. If the current request created the task:
   enqueue the submission plan and return the standard `registered + task_id` response.

### Compatibility Projection Helper

Add a focused helper in `proof_api.rs` that:

- loads `HoodiTaskData` for the existing root task
- converts the root task state into the legacy-compatible `POST` response projection

This helper must reuse the existing task-loading and root-state resolution logic rather than
duplicating state inference rules.

## Code Touchpoints

### `crates/runtime/src/lib.rs`

Add:

- `request_fingerprint` field to runtime task registration and record types
- schema migration for the new column and index
- lookup by request fingerprint
- idempotent root-task registration helper

### `bin/raiko2/src/server/handlers/proof_api.rs`

Add:

- fingerprint computation
- existing-task lookup by fingerprint
- compatibility response projection
- idempotent registration handling for duplicate requests

Do not change:

- engine task construction
- queue submission logic
- `/v3/tasks/{id}` response shape

### `bin/raiko2/src/server/handlers/proof_types.rs`

Add dedicated legacy projection types for the compatibility `POST` response, for example:

- `LegacyProofEnvelope`
- `LegacyProofData`
- `LegacyProofError`

Do not overload `HoodiTaskData` or mutate the `/v3/tasks/{id}` shape to satisfy the legacy client.

### `docs/API.md`

Document that `POST /v3/proof/batch/shasta` is now idempotent and may return:

- `registered`
- `work_in_progress`
- `completed`
- top-level `error`

Also document that `/v3/tasks/{id}` remains the canonical async inspection API.

## Proof Material Projection

The compatibility `POST` response must return proof material as:

```json
{
  "proof": "0x...",
  "kzg_proof": "",
  "quote": ""
}
```

For this change:

- `proof` is required on completed success
- `kzg_proof` and `quote` may be empty strings if the root runtime does not yet persist those values

This keeps the response compatible with the existing `taiko-client` decoder while preserving space
for fuller proof metadata in the future.

## Testing Plan

### Runtime Tests

Add tests that verify:

- the runtime migration adds `request_fingerprint` safely on existing databases
- lookup by fingerprint returns the expected root task
- idempotent registration returns the existing task on duplicate fingerprints
- concurrent duplicate registration does not create duplicate root tasks

### End-to-End Handler Tests

Add coverage for:

- first `POST /v3/proof/batch/shasta` returns `registered + task_id`
- repeated identical request while still idle or newly allocated returns `registered`
- repeated identical request with progress returns `work_in_progress`
- repeated identical request after completion returns `completed + data.proof`
- repeated identical request after failure returns top-level `status=error` with `message`
- `aggregate=true` returns the aggregate proof on terminal success
- `zk_any_not_drawn` remains unchanged
- `/v3/tasks/{id}` remains unchanged

### Regression Boundaries

The change must not:

- create duplicate engine tasks for identical concurrent requests
- coalesce requests that differ in route, pair, mode, prover args, or normalized proposals
- change `GET /v3/tasks/{id}`
- change `GET /v3/proof/report`
- change `GET /v3/proof/list`
- change `POST /v3/proof/prune`
- change queue or worker execution semantics

## Rollout Notes

This is a server-only compatibility change.

Operationally:

- existing `taiko-client` instances can continue repeating `POST /v3/proof/batch/shasta`
- new clients can still use `/v3/tasks/{id}`
- metrics and logs should explicitly distinguish:
  - new registration
  - duplicate request reuse
  - compatibility projection response kind

## Open Questions

None for the initial implementation scope.

The server can return empty `kzg_proof` and `quote` fields in the compatibility projection until the
runtime persists richer proof metadata. That is sufficient for the current `taiko-client` behavior.
