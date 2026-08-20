# Runtime Retention Batch GC Design

## Context

The GCS runtime backend persists one authoritative JSON snapshot containing runtime tasks, proof
artifact registrations, and pending publication intents. Every authoritative mutation rewrites the
whole snapshot with a generation precondition. Mainnet measurements showed that terminal task
metadata and old artifact registrations dominate the snapshot, while the information is normally
needed only until proposal proofs have been aggregated and consumed.

Before this change, the cleanup loop retained terminal tasks for seven days and removed them one at
a time. A shorter TTL alone would reduce steady-state size, but the one-at-a-time lifecycle would
rewrite the large snapshot repeatedly during cleanup.

## Decision

Use a configurable six-hour terminal-task retention window and garbage-collect expired tasks in
batches. Root retirement and external artifact reclamation are separate lifecycles: a proof-object
deletion failure must not retain a client-visible task record, while an invalidated artifact record
or pending publication intent remains an independent retry anchor.

Proof artifacts and pending publications are swept independently of task selection. This both
reclaims records orphaned by the pre-batch collector and lets external cleanup retry without
occupying deterministic public task IDs. After root cleanup, a repeated request creates a fresh task
and may prove again; the runtime does not rehydrate expired tasks from old GCS proof objects.

The six-hour window starts at the terminal task's `updated_at` timestamp. Active tasks are never
removed by this TTL, even if they run longer than six hours.

## Configuration

Use `runtime.terminal_task_ttl_secs` with a default of `21600`,
`runtime.cleanup_interval_secs` with a default of `30`, and `runtime.cleanup_batch_size` with a
default of `64` and maximum of `1024`. All values must be greater than zero. Cleanup pacing does not
reuse the queue maintenance interval because a retention batch can rewrite the complete authoritative
snapshot more than once. The GCS bucket lifecycle remains independent and continues to control
deletion of unrelated or unreachable objects.

## Batch Lifecycle

Each maintenance pass has independent bounded budgets for root retirement, artifact reclamation,
and pending-publication reclamation. Every lane reserves work for retries and fresh cursor progress,
so neither a permanently failing record nor a continuous stream of newly expired records can starve
the other side.

### Root retirement

1. Acquire the existing execution lifecycle gate.
2. In one authoritative mutation, verify each task's full observed snapshot, retire unchanged
   matches, and mark newly unowned artifact registrations invalidated. Invalidating before detachment
   prevents an expired cache entry from being adopted by a concurrent replacement.
3. Detach each retired root from its engine queue. A detach failure retains only that exact cancelled
   root in the root retry lane.
4. In one authoritative mutation, remove every successfully detached exact root snapshot and prune
   its pending-publication ownership. Release the execution lifecycle gate before any object-store
   cleanup. No artifact or pending-publication failure can retain a successfully detached root.

### Artifact reclamation

1. Select a bounded mix of retry artifacts and fresh artifact records. Fresh selection scans the
   artifact table itself rather than deriving keys only from newly retired roots.
2. In one authoritative mutation, recheck each exact record and its usable owners. Mark unowned
   active or pending records invalidated; already-invalidated records remain eligible for retry.
3. Finalize exact invalidations outside the execution lifecycle gate using content hash and object
   generation. A failed or stale invalidation retains only that artifact record.
4. Remove only exact successfully finalized artifact records in one authoritative mutation.

This lane is also the forward migration for active artifact records whose task owners were removed
by the previous collector.

### Pending-publication reclamation

1. Independently select pending intents with no live owner, using the same retry/fresh fairness rule.
2. Under the per-artifact lifecycle lock, recheck the exact intent, owner incarnations, and content
   hash. If a canonical object exists without an artifact record, first record and finalize its exact
   invalidation so the historical publication crash window cannot leave an untracked manifest.
3. Delete the exact pending object and remove only the exact durable intent. A changed intent, a new
   live owner, or an external failure retains only that pending intent for retry.

The number of runtime-state writes is bounded by the number of cleanup phases, not by the number of
ordinary tasks or artifacts in a batch. Repairing a rare canonical-object-without-record crash window
may require an additional exact state transition.

## Restart And Retry

Cleanup cursors and retry queues are process-local optimizations, not authority. Restarting resets
them and scans authoritative state from the beginning.

Startup must restore recoverable pending publications before attempting cleanup-only invalidations.
An invalidated artifact is already unreadable from runtime state, so object-store cleanup is not a
readiness prerequisite. Transient cleanup failures are logged and retried by maintenance rather than
crash-looping the service. A stale descriptor is reconciled against the current canonical descriptor
and live pending owners; it is never accepted as permission to delete a changed object.

## Safety Invariants

- Non-terminal tasks are never selected by terminal retention.
- Task incarnation and complete-record equality fence stale cleanup observations.
- An artifact remains active while any retained non-failed, non-cancelled task references it.
- Artifact invalidation is authoritative before external object deletion.
- An invalidated artifact cannot satisfy a cache lookup while deletion is retried.
- Root removal depends only on exact retirement and queue detachment, never on external deletion.
- A failed external operation retains only its exact artifact or pending intent.
- Retry work and fresh cursor progress both receive bounded capacity on every maintenance pass.
- Slow object-store operations never hold the process-wide execution lifecycle gate.
- Runtime draining fences batch cleanup through the existing namespace and lifecycle gates.
- Pending publication objects and intents are removed only after their last task owner becomes
  terminal and exact external deletion succeeds.
- A request whose observed task disappears during cleanup atomically returns to normal registration;
  it does not fail with an internal replacement error.

## Observability

Expose metrics for the current serialized runtime-state size and task/artifact/pending counts. Add
cleanup counters for selected, retired, removed, retained-on-failure, and artifact invalidation
outcomes, including pending publication removal and retry failures. Per-lane metrics expose bounded
fresh/retry attempts, success/failure/stale outcomes, and current process-local retry queue lengths.
Keep the existing structured cleanup log as the per-pass summary.

The initial rollout should explicitly configure the cleanup interval and batch size, then compare
snapshot size, GCS write duration/conflicts, and cleanup failure counts. The six-hour TTL must not
turn the accumulated seven-day backlog into an unpaced sequence of full-snapshot writes. Only after
the backlog drains should operators consider a shorter three-hour window.

## Non-Goals

- Rehydrating expired completed tasks from immutable proof objects.
- Replacing the runtime authority backend with SQLite in this change.
- Changing the GCS bucket lifecycle policy.
- Introducing distributed ownership or supporting overlapping runtime processes.
