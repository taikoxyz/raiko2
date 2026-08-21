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

Retention admission is represented independently from execution status. Preparing a root for
removal must preserve its `RunnerStatus`, proof URI, and execution error; a client continues to see
the original terminal result until exact root removal commits. The dedicated retention state is a
process-local two-phase deletion marker: it participates in exact in-process comparisons but is not
persisted in the authoritative snapshot. After a restart, an unchanged expired terminal root is
selected again normally. The marker is never interpreted as proposal or aggregate execution progress.

## Configuration

Use `runtime.terminal_task_ttl_secs` with a default of `21600`,
`runtime.cleanup_interval_secs` with a default of `30`, and `runtime.cleanup_batch_size` with a
default of `64` and maximum of `1024`. All values must be greater than zero. Cleanup pacing does not
reuse the queue maintenance interval because a retention batch can rewrite the complete authoritative
snapshot more than once. The pre-existing seven-day orphan-management pass processes at most 64
records per pass and uses a smaller configured cleanup batch when requested because it may perform
per-task lifecycle mutations. The GCS bucket lifecycle remains independent and continues to control
deletion of unrelated or unreachable objects.

## Batch Lifecycle

Each maintenance pass has independent bounded budgets for root retirement, artifact reclamation,
pending-publication reclamation, and overdue-active observation. Root retirement runs first so proof
and pending records whose last owner expires can be reclaimed in the same pass. Every cleanup lane
reserves work for retries and fresh cursor progress, so neither a permanently failing record nor a
continuous stream of newly expired records can starve the other side.

### Root retirement

1. Acquire the existing execution lifecycle gate.
2. In one in-process authoritative mutation, verify each task's full observed snapshot and mark
   unchanged terminal matches as `removing` in the independent retention lifecycle. Preserve the
   task's execution status, proof URI, error, and terminal timestamp. The process-local marker is
   omitted from persisted snapshots, so a restart re-admits the same terminal root instead of
   depending on a newer schema. Root preparation does not invalidate proof artifacts or pending
   publications because the task still owns them until exact root removal commits.
3. Detach each prepared root from its engine queue. A detach failure retains only that exact root,
   with its original client-visible terminal result, in the root retry lane.
4. In one authoritative mutation, remove every successfully detached exact root snapshot and prune
   its pending-publication ownership. Release the execution lifecycle gate before any object-store
   cleanup. No artifact or pending-publication failure can retain a successfully detached root.

### Artifact reclamation

1. Build one ownership index from the authoritative task table, then select a bounded mix of retry
   artifacts and fresh artifact records. Fresh selection scans the artifact table itself rather than
   deriving keys only from newly retired roots.
2. In one authoritative mutation, recheck each exact record and its retained task owners. Mark only
   ownerless active or pending records invalidated. An already-invalidated record is a committed
   logical deletion and remains eligible for external finalization even if an older task record still
   references it. Task status does not affect ownership before invalidation commits.
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
   hash. If an artifact record exists, leave its canonical manifest exclusively to the artifact lane.
   If no artifact record exists, exact-invalidate only a canonical descriptor with the same content
   hash as the pending intent. A changed untracked descriptor is left untouched for explicit
   namespace cleanup rather than being adopted as local invalidation authority.
3. Revalidate the exact intent and absence of a retained owner, then delete the exact pending-object
   generation currently observed at its private pending key. The object hash may differ from an older
   intent after an interrupted or partial publication; generation matching prevents deletion of a
   later concurrent write. Remove only the exact durable intent. A changed intent, a new retained
   task reference, or an external failure retains only that pending intent for retry.

The number of runtime-state writes in the root, artifact, and pending retention lanes is bounded by
cleanup phases, not by the number of records in a batch. External retention cleanup never performs
one full-snapshot mutation per item. The separate orphan-management pass remains per-task and is
therefore capped independently at 64 records.

## Restart And Retry

Cleanup cursors, retry queues, and overdue-active warning deduplication are process-local
optimizations, not authority. Restarting resets them and scans authoritative state from the
beginning. Retry selection is non-destructive: an identity remains queued until its lane completes
successfully or authoritative state proves it no longer eligible. Fresh cursors advance only after a
complete successful selection. Each retry lane is capped at 4096 identities; the authoritative scan
remains the recovery source for work that cannot enter a full process-local queue. Overdue-active
observation retains the first 4096 task incarnations seen by one process without FIFO eviction, so a
small configured batch cannot make the same tasks cycle through warning logs.

The seven-day orphan lane is deliberately fail-stop. A persistent artifact-reconciliation or
lifecycle error indicates an internally inconsistent task that requires operator intervention in this
serial proving system; later retention lanes do not advance past it. The failing task identity and
stage are logged, and a fixed-label blocked gauge remains set until a later orphan pass succeeds.
Operators may repair the external state or perform a one-shot
`runtime.startup_cleanup = ["proof"]`, which clears the authoritative task table and active proof
manifests before runtime initialization. Ordinary proving failures and transient orphan absence do not
enter this path.

Startup must restore recoverable pending publications before attempting cleanup-only invalidations.
An invalidated artifact is already unreadable from runtime state, so object-store cleanup is not a
readiness prerequisite. Transient cleanup failures are logged and retried by maintenance rather than
crash-looping the service. A stale descriptor is reconciled against the current canonical descriptor
and live pending owners; it is never accepted as permission to delete a changed object.

Retention ownership and publication liveness are deliberately separate predicates. Retention treats
every extant task incarnation as an owner, including failed and cancelled records. Publication
activation and recovery continue to require a live task owner, so terminal records cannot make an
in-flight publication usable merely because they retain it from garbage collection.

Retention never changes proposal or aggregate execution state. Proposal retry belongs to the proposal
execution policy, and an aggregate remains pending until its declared proposal dependencies succeed.
A succeeded proposal must already have a readable active artifact. External deletion or corruption
after success is an explicit storage-recovery event handled by proof startup cleanup and request
resubmission, not by aggregate-owned reproving.

## Safety Invariants

- Non-terminal tasks are never selected by terminal retention.
- Task incarnation and complete-record equality fence stale cleanup observations.
- Retention admission never changes `RunnerStatus`, proof URI, execution error, or terminal time.
- A failed queue detach leaves the original client-visible terminal result readable and retries only
  the independent retention transition.
- An artifact or pending publication remains retained while any task record references it,
  regardless of task status.
- Artifact ownership is rechecked before invalidation admission. Once exact invalidation is
  authoritative, a later task cannot resurrect that descriptor.
- A committed artifact invalidation can always finish external deletion; an obsolete task reference
  cannot turn the invalidated manifest back into a retained active artifact.
- Artifact invalidation is authoritative before external object deletion.
- An invalidated artifact cannot satisfy a cache lookup while deletion is retried.
- Root removal depends only on exact retirement and queue detachment, never on external deletion.
- A failed external operation retains only its exact artifact or pending intent.
- Retry work and fresh cursor progress both receive bounded capacity on every maintenance pass.
- Selecting a retry or fresh item cannot lose it when another lane or later phase fails.
- Slow object-store operations never hold the process-wide execution lifecycle gate.
- Runtime draining fences batch cleanup through the existing namespace and lifecycle gates.
- Pending publication objects and intents are removed only after their last task-owner record is
  removed and exact-generation external deletion succeeds.
- A request whose observed task disappears during cleanup atomically returns to normal registration;
  it does not fail with an internal replacement error.
- An active task older than the terminal retention window is logged but never selected for removal.
- Overdue-active observation runs before orphan cancellation or any retention state transition in a
  cleanup pass.
- A persistent orphan reconciliation failure blocks later retention lanes until external repair or
  explicit proof startup cleanup; it does not masquerade as an ordinary task failure.
- Retention never resets, retries, or otherwise mutates proposal or aggregate execution state.

## Observability

Expose metrics for the current serialized runtime-state size and task/artifact/pending counts. Add
cleanup counters for selected, retired, removed, retained-on-failure, artifact invalidation, and
failed maintenance passes, including pending publication removal and retry failures. Per-lane metrics
expose bounded fresh/retry attempts, success/failure/stale outcomes, and current process-local retry
queue lengths. Keep the existing structured cleanup log as the per-pass summary. Overdue active-task
warnings include only bounded identifiers and age, are deduplicated per process, and have a
fixed-label counter suitable for alerting. Expose orphan fail-stop as a fixed-label blocked gauge that
returns to zero after the orphan lane completes successfully.

The initial rollout should explicitly configure the cleanup interval and batch size, then compare
snapshot size, GCS write duration/conflicts, and cleanup failure counts. The six-hour TTL must not
turn the accumulated seven-day backlog into an unpaced sequence of full-snapshot writes. Only after
the backlog drains should operators consider a shorter three-hour window.

## Non-Goals

- Rehydrating expired completed tasks from immutable proof objects.
- Replacing the runtime authority backend with SQLite in this change.
- Changing the GCS bucket lifecycle policy.
- Introducing distributed ownership or supporting overlapping runtime processes.
