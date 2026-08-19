# Runtime Retention Batch GC Design

## Context

The GCS runtime backend persists one authoritative JSON snapshot containing runtime tasks, proof
artifact registrations, and pending publication intents. Every authoritative mutation rewrites the
whole snapshot with a generation precondition. Mainnet measurements showed that terminal task
metadata and old artifact registrations dominate the snapshot, while the information is normally
needed only until proposal proofs have been aggregated and consumed.

The existing cleanup loop retains terminal tasks for seven days and removes them one at a time. A
shorter TTL alone would reduce steady-state size, but the one-at-a-time lifecycle would rewrite the
large snapshot repeatedly during cleanup.

## Decision

Use a configurable six-hour terminal-task retention window and garbage-collect expired tasks in
batches. Expired proof artifacts are invalidated and removed when no retained runtime task still
references them. After cleanup, a repeated request creates a fresh task and may prove again; the
runtime does not rehydrate expired tasks from old GCS proof objects.

The six-hour window starts at the terminal task's `updated_at` timestamp. Active tasks are never
removed by this TTL, even if they run longer than six hours.

## Configuration

Add `runtime.terminal_task_ttl_secs` with a default of `21600`. The value must be greater than zero.
The GCS bucket lifecycle remains independent and continues to control deletion of unrelated or
unreachable objects.

## Batch Lifecycle

Each maintenance pass selects at most one bounded batch of exact terminal task snapshots whose
`updated_at` is outside the retention window.

1. Acquire the existing execution lifecycle gate.
2. In one authoritative mutation, verify each task's full observed snapshot, retire unchanged
   matches, mark newly unowned artifact registrations invalidated, and capture exact unowned pending
   publication intents.
3. Detach the retired roots from their engine queues, then release the execution lifecycle gate. A
   failed detach leaves that root retired for a later cleanup pass.
4. Finalize exact artifact invalidations using their content hash and object generation. External
   deletion is bounded-concurrent and does not hold the lifecycle gate. A failed deletion leaves the
   artifact invalidated for retry.
5. Delete exact unowned pending publication objects under their artifact lifecycle locks. A changed
   intent, new live owner, or deletion failure retains the terminal task and intent for retry.
6. In one authoritative mutation, remove successfully detached task records, successfully finalized
   artifact records, and exact pending publication intents. Shared pending intents retain owners that
   belong to live tasks.

The number of runtime-state writes is bounded by the number of cleanup phases, not by the number of
tasks or artifacts in the batch.

## Safety Invariants

- Non-terminal tasks are never selected by terminal retention.
- Task incarnation and complete-record equality fence stale cleanup observations.
- An artifact remains active while any retained non-failed, non-cancelled task references it.
- Artifact invalidation is authoritative before external object deletion.
- An invalidated artifact cannot satisfy a cache lookup while deletion is retried.
- Slow object-store operations never hold the process-wide execution lifecycle gate.
- Runtime draining fences batch cleanup through the existing namespace and lifecycle gates.
- Pending publication objects and intents are removed only after their last task owner becomes
  terminal and exact external deletion succeeds.
- A request whose observed task disappears during cleanup atomically returns to normal registration;
  it does not fail with an internal replacement error.

## Observability

Expose metrics for the current serialized runtime-state size and task/artifact/pending counts. Add
cleanup counters for selected, retired, removed, retained-on-failure, and artifact invalidation
outcomes, including pending publication removal and retry failures. Keep the existing structured
cleanup log as the per-pass summary.

The initial rollout should compare snapshot size, GCS write duration/conflicts, and cleanup failure
counts before considering a shorter three-hour window.

## Non-Goals

- Rehydrating expired completed tasks from immutable proof objects.
- Replacing the runtime authority backend with SQLite in this change.
- Changing the GCS bucket lifecycle policy.
- Introducing distributed ownership or supporting overlapping runtime processes.
