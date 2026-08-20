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
batches. A task record owns every proof artifact and pending publication it references for the
record's complete lifetime, including `Succeeded`, `Failed`, and `Cancelled` states. Root retirement
and external artifact reclamation are separate lifecycles: removing an expired task releases its
ownership, and a proof-object deletion failure must not retain the retired client-visible task
record.

Proof artifacts and pending publications are swept independently of task selection. This both
reclaims records orphaned by the pre-batch collector and lets external cleanup retry without
occupying deterministic public task IDs. After root cleanup, a repeated request creates a fresh task
and may prove again; the runtime does not rehydrate expired tasks from old GCS proof objects.

The six-hour window starts at the terminal task's `updated_at` timestamp. Active tasks are never
removed by this TTL, even if they run longer than six hours. An active task older than the same
window is abnormal and emits one deduplicated structured warning per observed task incarnation.
This is an operational failure signal, not a second reclamation policy.

## Configuration

Use `runtime.terminal_task_ttl_secs` with a default of `21600`,
`runtime.cleanup_interval_secs` with a default of `30`, and `runtime.cleanup_batch_size` with a
default of `64` and maximum of `1024`. All values must be greater than zero. Cleanup pacing does not
reuse the queue maintenance interval because a retention batch can rewrite the complete authoritative
snapshot more than once. The GCS bucket lifecycle remains independent and continues to control
deletion of unrelated or unreachable objects.

## Batch Lifecycle

Each maintenance pass has independent bounded budgets for root retirement, artifact reclamation,
pending-publication reclamation, and overdue-active observation. Root retirement runs first so proof
and pending records whose last owner expires can be reclaimed in the same pass. Every cleanup lane
reserves work for retries and fresh cursor progress, so neither a permanently failing record nor a
continuous stream of newly expired records can starve the other side.

### Root retirement

1. Acquire the existing execution lifecycle gate.
2. In one authoritative mutation, verify each task's full observed snapshot and retire unchanged
   matches. Root preparation does not invalidate proof artifacts or pending publications because
   the task still owns them until exact root removal commits.
3. Detach each retired root from its engine queue. A detach failure retains only that exact cancelled
   root in the root retry lane.
4. In one authoritative mutation, remove every successfully detached exact root snapshot and prune
   its pending-publication ownership. Release the execution lifecycle gate before any object-store
   cleanup. No artifact or pending-publication failure can retain a successfully detached root.

### Artifact reclamation

1. Build one ownership index from the authoritative task table, then select a bounded mix of retry
   artifacts and fresh artifact records. Fresh selection scans the artifact table itself rather than
   deriving keys only from newly retired roots.
2. In one authoritative mutation, recheck each exact record and its retained task owners. Mark
   ownerless active or pending records invalidated; already-invalidated ownerless records remain
   eligible for retry. Task status does not affect ownership.
3. Finalize exact invalidations outside the execution lifecycle gate using content hash and object
   generation. A failed or stale invalidation retains only that artifact record.
4. Remove only exact successfully finalized artifact records in one authoritative mutation.

This lane is also the forward migration for active artifact records whose task owners were removed
by the previous collector.

### Pending-publication reclamation

1. Independently select pending intents with no retained task incarnation owner, using the same
   retry/fresh fairness rule and the same ownership index. Task status and the intent's potentially
   stale owner list do not affect retention ownership; an authoritative task-record reference is
   sufficient.
2. Under the per-artifact lifecycle lock, recheck the exact intent, owner incarnations, and content
   hash. If a canonical object exists without an artifact record, exact-invalidate that descriptor
   without adopting it into runtime state, so the historical publication crash window cannot leave
   an untracked manifest or turn a concurrently replaced descriptor into stale local authority.
3. Delete the exact pending object and remove only the exact durable intent. A changed intent, a new
   retained task reference, or an external failure retains only that pending intent for retry.

The number of runtime-state writes is bounded by cleanup phases, not by the number of ordinary tasks
or artifacts in a batch. External cleanup never performs one full-snapshot mutation per item. A
canonical-object-without-record crash window is handled under the per-artifact lock and folded into
the lane's batched state transition.

## Restart And Retry

Cleanup cursors, retry queues, and overdue-active warning deduplication are process-local
optimizations, not authority. Restarting resets them and scans authoritative state from the
beginning. Retry selection is non-destructive: an identity remains queued until its lane completes
successfully or authoritative state proves it no longer eligible. Fresh cursors advance only after a
complete successful selection. Each retry lane is capped at 4096 identities; the authoritative scan
remains the recovery source for work that cannot enter a full process-local queue.

Startup must restore recoverable pending publications before attempting cleanup-only invalidations.
An invalidated artifact is already unreadable from runtime state, so object-store cleanup is not a
readiness prerequisite. Transient cleanup failures are logged and retried by maintenance rather than
crash-looping the service. A stale descriptor is reconciled against the current canonical descriptor
and live pending owners; it is never accepted as permission to delete a changed object.

Retention ownership and publication liveness are deliberately separate predicates. Retention treats
every extant task incarnation as an owner, including failed and cancelled records. Publication
activation and recovery continue to require a live task owner, so terminal records cannot make an
in-flight publication usable merely because they retain it from garbage collection.

If aggregation discovers that a succeeded proposal dependency no longer has a readable proof
artifact, the queue atomically resets that dependency to ready and the running aggregate to pending.
The proposal republishes its proof before aggregation resumes. This makes accidental or retention-
driven artifact loss recoverable in the current process instead of relying on a restart and duplicate
client submission.

## Safety Invariants

- Non-terminal tasks are never selected by terminal retention.
- Task incarnation and complete-record equality fence stale cleanup observations.
- An artifact or pending publication remains retained while any task record references it,
  regardless of task status.
- Artifact invalidation is authoritative before external object deletion.
- An invalidated artifact cannot satisfy a cache lookup while deletion is retried.
- Root removal depends only on exact retirement and queue detachment, never on external deletion.
- A failed external operation retains only its exact artifact or pending intent.
- Retry work and fresh cursor progress both receive bounded capacity on every maintenance pass.
- Selecting a retry or fresh item cannot lose it when another lane or later phase fails.
- Slow object-store operations never hold the process-wide execution lifecycle gate.
- Runtime draining fences batch cleanup through the existing namespace and lifecycle gates.
- Pending publication objects and intents are removed only after their last task-owner record is
  removed and exact external deletion succeeds.
- A request whose observed task disappears during cleanup atomically returns to normal registration;
  it does not fail with an internal replacement error.
- An active task older than the terminal retention window is logged but never selected for removal.
- Overdue-active observation runs before orphan cancellation or any retention state transition in a
  cleanup pass.
- A missing aggregate input artifact requeues its succeeded proposal dependency before aggregation
  retries.

## Observability

Expose metrics for the current serialized runtime-state size and task/artifact/pending counts. Add
cleanup counters for selected, retired, removed, retained-on-failure, artifact invalidation, and
failed maintenance passes, including pending publication removal and retry failures. Per-lane metrics
expose bounded fresh/retry attempts, success/failure/stale outcomes, and current process-local retry
queue lengths. Keep the existing structured cleanup log as the per-pass summary. Overdue active-task
warnings include only bounded identifiers and age, are deduplicated per process, and have a
fixed-label counter suitable for alerting.

The initial rollout should explicitly configure the cleanup interval and batch size, then compare
snapshot size, GCS write duration/conflicts, and cleanup failure counts. The six-hour TTL must not
turn the accumulated seven-day backlog into an unpaced sequence of full-snapshot writes. Only after
the backlog drains should operators consider a shorter three-hour window.

## Non-Goals

- Rehydrating expired completed tasks from immutable proof objects.
- Replacing the runtime authority backend with SQLite in this change.
- Changing the GCS bucket lifecycle policy.
- Introducing distributed ownership or supporting overlapping runtime processes.
