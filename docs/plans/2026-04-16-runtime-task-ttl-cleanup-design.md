# Runtime Task TTL Cleanup Design (raiko2)

> Historical design document. It may not match the current implementation. Use `README.md`,
> `docs/API.md`, and `config.example.toml` as the current source of truth.

## Goal

Prevent unbounded growth of persisted root-task state in `raiko2` by introducing automatic cleanup
for inactive terminal runtime tasks.

The cleanup model should behave like a key TTL:

- every persisted root task has an inactivity window
- once a terminal task stays inactive past that window, it becomes eligible for deletion
- cleanup behavior must be identical for both memory and Redis queue backends

## Non-Goals

- Adding TTL semantics to the engine queue store itself.
- Deleting or timing out live tasks (`allocated` or `running`).
- Replacing explicit `POST /proof/prune` with TTL cleanup.
- Introducing capacity-based LRU eviction in this change.
- Changing proof-status semantics or route ownership.

## Decision

Adopt a runtime-owned inactive TTL for persisted root tasks.

The canonical owner of TTL policy is the runtime layer, not the queue backend. Queue backend choice
(`memory` or `redis`) must not affect retention behavior.

The default policy is:

- `runtime.inactive_ttl_secs = 7200`
- `0` disables automatic cleanup
- only terminal tasks are eligible:
  - `completed`
  - `failed`
  - `cancelled`

Cleanup execution reuses the existing maintenance cadence but runs through a single dedicated
runtime cleanup loop rather than attaching to each engine maintenance worker.

## Configuration

Extend runtime configuration with:

```toml
[runtime]
root = "./data/runtime"
inactive_ttl_secs = 7200
```

Rules:

- `inactive_ttl_secs > 0` enables automatic cleanup
- `inactive_ttl_secs = 0` disables automatic cleanup
- validation rejects negative values only if the type ever changes from `u64`

The config remains runtime-owned because the policy applies to persisted root-task state and task
directories, not to queue records.

## TTL Semantics

The inactivity clock is derived from the existing `updated_at` column in `runtime.sqlite`.

Eligibility formula:

- `updated_at + inactive_ttl_secs <= now`

This intentionally avoids introducing a second persisted time source such as `expires_at`.

`updated_at` is already advanced by:

- initial task registration
- runtime status synchronization
- runtime metadata mutations

This makes the TTL semantics equivalent to "delete terminal tasks that have had no runtime activity
for N seconds".

## Architecture

### 1. Runtime query surface

`crates/runtime` adds a read-only query named:

- `list_expired_terminal_tasks(now_ts, ttl_secs, limit)`

Behavior:

- filters to terminal statuses only
- filters to `updated_at <= now_ts - ttl_secs`
- sorts by:
  - `updated_at ASC`
  - `task_id ASC`
- limits the batch size with a deterministic cap

This makes runtime the single source of truth for TTL eligibility.

### 2. Cleanup execution owner

`bin/raiko2` owns the destructive side effects:

- resolving the correct engine for a root task
- deleting child engine tasks when safe
- deleting the runtime row
- deleting the runtime task directory

This belongs in server state because runtime alone does not have access to the pipeline engines.

### 3. Dedicated cleanup loop

`AppState::new()` starts one runtime cleanup loop for the whole process.

Properties:

- reuses `queue.maintenance_interval_ms` as its tick cadence
- runs once on startup, then on every interval
- does not run once per pipeline engine

This avoids duplicate cleanup work because the current engine maintenance loop exists per engine,
and `raiko2` can host multiple engines simultaneously.

## Safe Deletion Rules

TTL cleanup must never delete live work.

### Root task eligibility

Eligible root tasks:

- `completed`
- `failed`
- `cancelled`

Ineligible root tasks:

- `allocated`
- `running`

This means TTL cleanup is strictly a retention policy, not a timeout policy.

### Child engine task deletion

TTL cleanup must not reuse the broad `/proof/prune` semantics unchanged.

`/proof/prune` is allowed to remove all child engine tasks because it is an explicit operator
command. TTL cleanup is automatic and therefore must be more conservative.

For each proposal-stage task and optional aggregate task:

1. inspect whether another live root task still references the same child engine task
2. if another live root exists, skip child deletion
3. otherwise call `engine.remove(...)`

"Live root task" means a distinct root task with status:

- `allocated`
- `running`

This preserves the current shared-child safety model and prevents automatic cleanup from removing
engine state that is still needed by another root task.

### Root removal order

For each expired root task:

1. load runtime record and metadata
2. resolve the engine from stored metadata and pipeline key
3. attempt safe child removal
4. remove the runtime row
5. remove the task directory

The root task may be removed even when some child engine tasks are intentionally retained because
they are still referenced by another live root task.

## Failure Semantics

The cleanup loop is best-effort and retryable.

### Non-fatal retention cases

The loop may remove the root task while skipping some child engine deletions when:

- a child engine task is still referenced by another live root task

This is a valid outcome, not an error.

### Retry cases

The loop must retain the root task for a later retry when any required destructive step fails:

- runtime metadata cannot be parsed
- engine resolution fails
- child `engine.remove(...)` fails for a child that should be removable
- runtime row deletion fails
- task directory deletion fails

This biases toward safety. The system should prefer leaving an old task behind rather than
producing partial cleanup with orphaned state.

## Batch Size and Scheduling

The initial design uses an internal batch limit rather than a public config option.

Recommended default:

- `RUNTIME_TTL_CLEANUP_BATCH_LIMIT = 64`

Rationale:

- TTL is the primary policy
- batch size is an implementation guardrail
- avoiding an extra config key keeps the first version smaller and easier to reason about

If backlog exceeds one batch, later ticks continue draining it.

## Observability

Each cleanup tick should emit a compact summary log containing:

- `scanned`
- `expired`
- `removed_roots`
- `skipped_shared_children`
- `retained_failures`

The first version does not need a new public API endpoint. Operator visibility can come from logs
and existing task-list behavior.

## Data and Compatibility

No schema migration is required.

The existing runtime schema already contains everything needed:

- `runner_status`
- `task_dir`
- `metadata_json`
- `updated_at`

This design intentionally avoids adding:

- `expires_at`
- per-task retention overrides
- backend-specific TTL storage

## Testing and Validation

### Unit and integration tests

Add coverage for:

- runtime config default:
  - `inactive_ttl_secs = 7200`
- runtime config disable path:
  - `inactive_ttl_secs = 0`
- expired terminal root cleanup:
  - a `completed` task older than 2 hours is removed
- live-task protection:
  - `allocated` and `running` tasks are not removed even when older than 2 hours
- shared-child protection:
  - child engine tasks referenced by another live root are not removed
- retry behavior:
  - if child removal fails, the root task remains for the next tick
- startup behavior:
  - expired terminal tasks are drained on the first cleanup pass after boot

### Verification commands

- `cargo fmt --all`
- `cargo clippy --workspace -- -D warnings`
- `cargo nextest run --workspace`

## Implementation Notes

- Keep TTL eligibility logic in `crates/runtime`.
- Keep engine interaction and destructive cleanup orchestration in `bin/raiko2`.
- Reuse the existing live-reference check semantics for shared child tasks.
- Do not couple runtime retention policy to queue backend implementation details.
