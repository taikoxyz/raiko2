# Runtime Retention Batch GC Implementation Plan

**Goal:** Bound runtime-state growth without letting artifact-store failures retain client task IDs,
starve retries, or leave legacy artifact records outside the collector.

**Architecture:** Keep the single-writer runtime authority and immutable object store, but separate
root retirement from artifact and pending-publication reclamation. Root removal is gated only by
exact task retirement and queue detachment; independent bounded cleanup lanes own external retries,
legacy orphan migration, and fair fresh progress.

**Tech Stack:** Rust, Tokio, Serde/TOML, Prometheus, GCS generation CAS, existing raiko2 runtime and
engine lifecycle APIs.

---

### Task 1: Add independently paced cleanup configuration

**Files:**
- Modify: `bin/raiko2/src/config/runtime.rs`
- Modify: `config.example.toml`
- Modify: `docs/API.md`

**Step 1: Write failing configuration tests**

Extend `config::runtime::tests` with exact default, override, and zero-value rejection cases for:

```rust
pub cleanup_interval_secs: u64,
pub cleanup_batch_size: usize,
```

Use conservative defaults of 30 seconds and 64 records. Reject zero and cap batch size at 1024 so a
misconfiguration cannot create an unbounded external fan-out.

**Step 2: Run the focused tests and verify failure**

Run: `cargo test -p raiko2 config::runtime`

Expected: FAIL because the new fields do not exist.

**Step 3: Implement and document the configuration**

Add Serde defaults and validation. Keep `terminal_task_ttl_secs = 21600`; do not reuse
`queue.maintenance_interval_ms` for retention scheduling.

**Step 4: Run the focused tests**

Run: `cargo test -p raiko2 config::runtime`

Expected: PASS.

**Step 5: Commit**

```bash
git add bin/raiko2/src/config/runtime.rs config.example.toml docs/API.md
git commit -m "feat(runtime): pace retention cleanup independently"
```

### Task 2: Decouple exact root removal from external cleanup

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `bin/raiko2/src/server/lifecycle.rs`
- Test: `bin/raiko2/src/server/task_cleanup.rs`

**Step 1: Write failing lifecycle tests**

Add a two-root regression where one root owns an artifact whose exact invalidation fails and the
other root has no external failure. Assert that both successfully detached root records are removed,
the failed artifact record remains invalidated, and a repeated deterministic request can register a
fresh incarnation immediately.

Add a detach-failure regression asserting that only the root whose projection failed remains
cancelled.

**Step 2: Run the focused tests and verify failure**

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server server::task_cleanup`

Expected: FAIL because `remove_terminal_retention_batch` gates every root on the complete external
batch.

**Step 3: Move root finalization before external I/O**

Keep `prepare_terminal_task_retention_batch` as the authoritative invalidation point. Under
`execution_lifecycle_gate`, detach each exact retired root and immediately call
`finalize_terminal_task_retention_batch` with only the successfully detached task snapshots. Release
the gate after that state commit.

Do not pass external failures into root-removal decisions. Preserve failed detach snapshots for the
root retry lane.

**Step 4: Run runtime and cleanup tests**

Run: `cargo test -p raiko2-runtime --lib batch_retention`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server server::task_cleanup`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/runtime/src/lib.rs bin/raiko2/src/server/lifecycle.rs bin/raiko2/src/server/task_cleanup.rs
git commit -m "fix(runtime): decouple root retention from artifact cleanup"
```

### Task 3: Add an independent artifact retention lane

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `bin/raiko2/src/server/lifecycle.rs`
- Modify: `bin/raiko2/src/server/task_cleanup.rs`

**Step 1: Write failing legacy-orphan tests**

Seed an active proof artifact, retire and remove its only task through the pre-batch shape, and then
run cleanup with no terminal task candidates. Assert that the artifact is selected, durably marked
invalidated, externally finalized, and removed from runtime state.

Add a shared-owner control asserting that an artifact referenced by any usable task is skipped.

**Step 2: Run the focused tests and verify failure**

Run: `cargo test -p raiko2-runtime --lib artifact_retention`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server legacy_orphan`

Expected: FAIL because artifact candidates are currently derived only from freshly retired tasks.

**Step 3: Add exact artifact scanning and preparation APIs**

Add a stable artifact cursor keyed by the canonical artifact state key and APIs equivalent to:

```rust
pub async fn list_reclaimable_proof_artifacts(
    &self,
    after: Option<&ArtifactRetentionCursor>,
    limit: usize,
) -> Result<Vec<ProofArtifactRecord>>;

pub async fn prepare_artifact_retention_batch(
    &self,
    expected: &[ProofArtifactRecord],
) -> Result<ArtifactRetentionPrepare>;
```

The prepare mutation must exact-match each record, recheck usable owners, mark active/pending orphan
records invalidated, and return already-invalidated records unchanged for retry. It must never
invalidate a changed descriptor or an artifact with a usable owner.

**Step 4: Drive bounded external finalization**

Run artifact finalization outside the execution lifecycle gate with the existing concurrency bound.
Finalize successful exact artifact records in one authoritative mutation; retain only failed or stale
records.

**Step 5: Run focused tests**

Run the commands from Step 2.

Expected: PASS.

**Step 6: Commit**

```bash
git add crates/runtime/src/lib.rs bin/raiko2/src/server/lifecycle.rs bin/raiko2/src/server/task_cleanup.rs
git commit -m "feat(runtime): sweep unowned proof artifacts independently"
```

### Task 4: Make pending-publication reclamation independent and complete

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `bin/raiko2/src/server/lifecycle.rs`
- Test: `bin/raiko2/src/server/task_cleanup.rs`

**Step 1: Write failing pending-publication tests**

Cover these cases:

- an unowned pending intent is removed after its root record has already disappeared;
- one pending delete failure does not block unrelated artifact or root finalization;
- a canonical object plus pending intent with no artifact record is removed only when its content
  hash matches the exact pending intent;
- a canonical object represented by an artifact record is left exclusively to the artifact lane;
- a new live owner or changed intent between selection and deletion is preserved.

**Step 2: Run the focused tests and verify failure**

Run: `cargo test -p raiko2-runtime --lib pending_retention`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server pending_publication`

Expected: at least the missing-artifact-record case and independent-root case FAIL.

**Step 3: Add independent candidate selection**

List exact pending expectations directly from `RuntimeState::pending_publications`, selecting only
records with no live owner. Do not derive candidates from a terminal task batch.

**Step 4: Close the canonical-object crash window**

Under the per-artifact lifecycle lock, exact-check the intent and owners. When an artifact record
exists, delete only the pending object and intent. When no artifact record exists, inspect the
canonical descriptor and exact-invalidate it only if its content hash matches the intent before
deleting the exact pending object. A changed descriptor remains untouched for explicit cleanup.

**Step 5: Run focused tests**

Run the commands from Step 2.

Expected: PASS.

**Step 6: Commit**

```bash
git add crates/runtime/src/lib.rs bin/raiko2/src/server/lifecycle.rs bin/raiko2/src/server/task_cleanup.rs
git commit -m "fix(runtime): reclaim unowned pending publications independently"
```

### Task 5: Add fair retry and fresh-progress scheduling

**Files:**
- Modify: `bin/raiko2/src/server/task_cleanup.rs`
- Modify: `bin/raiko2/src/server/telemetry.rs`

**Step 1: Write failing sustained-arrival regressions**

Create more than one batch of terminal roots. Force one old detach and one artifact invalidation to
fail on every attempt while adding at least one newly expired record before each pass. Assert across
multiple passes that:

- failed records are retried without waiting for an empty page;
- fresh records continue to be removed;
- a permanently failing item cannot consume the complete batch;
- retry queues do not contain duplicate identities.

**Step 2: Run the focused tests and verify failure**

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server sustained_arrival`

Expected: FAIL because the current cursor retries only after an empty page.

**Step 3: Implement per-lane scheduler state**

Replace the standalone cursors with a cleanup-loop state containing fresh cursors plus deduplicated
round-robin retry queues for roots, artifacts, and pending intents. Each lane divides its configured
batch budget between retry and fresh work; unused capacity from either side is donated to the other.

Retry entries contain only stable identities. Reload the current exact record before every retry and
drop entries that disappeared, changed ownership, or are no longer eligible. Cursors and queues reset
on restart; authoritative state remains the source of truth.

**Step 4: Add bounded metrics**

Expose retry-queue lengths and fresh/retry attempt counters using only fixed lane/outcome labels.
Avoid task IDs, proof refs, and exact global-gauge values in parallel tests.

**Step 5: Run focused tests**

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server server::task_cleanup`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server server::telemetry`

Expected: PASS.

**Step 6: Commit**

```bash
git add bin/raiko2/src/server/task_cleanup.rs bin/raiko2/src/server/telemetry.rs
git commit -m "fix(runtime): schedule retention retries fairly"
```

### Task 6: Make restart recovery cleanup-safe

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `bin/raiko2/src/server/state/mod.rs`
- Test: `crates/runtime/src/lib.rs`
- Test: `bin/raiko2/src/server/state/mod.rs`

**Step 1: Write failing restart tests**

Simulate an invalidated old descriptor, an identical republished canonical object with a new
generation, and a durable pending intent owned by a live task. Restart the runtime and assert that it
restores the current pending publication before cleanup and does not fail startup on the stale old
descriptor.

Also inject a transient invalidation failure and assert that initialization succeeds while the
artifact remains invalidated and eligible for maintenance retry.

**Step 2: Run the focused tests and verify failure**

Run: `cargo test -p raiko2-runtime --lib restart_retention`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server startup_retention`

Expected: FAIL because startup currently runs fail-fast invalidation reconciliation before pending
runtime restoration.

**Step 3: Reorder recovery and remove cleanup from readiness**

Restore recoverable pending runtime state before invalidated-artifact cleanup. Treat exact external
deletion as maintenance: keep invalidated state unreadable, log cleanup failures, and let the
independent artifact lane retry after startup.

Never treat a stale descriptor as permission to delete the current object. Reconcile it only through
the current canonical descriptor plus exact durable pending owner checks.

**Step 4: Run focused restart tests**

Run the commands from Step 2.

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/runtime/src/lib.rs bin/raiko2/src/server/state/mod.rs
git commit -m "fix(runtime): defer artifact cleanup during restart"
```

### Task 7: Align documentation and stabilize telemetry tests

**Files:**
- Modify: `docs/API.md`
- Modify: `docs/operations.md`
- Modify: `docs/architecture.md`
- Modify: `config.example.toml`
- Modify: `bin/raiko2/src/server/telemetry.rs`
- Modify: `docs/plans/2026-08-19-runtime-retention-batch-gc-design.md`

**Step 1: Remove stale seven-day terminal-retention statements**

Keep the seven-day orphan-task window where it is still correct. Change terminal root retention to
the configurable six-hour default and document that artifact/pending cleanup is ownership-driven and
independent of root records.

**Step 2: Stabilize the global-registry metric test**

Assert metric family and bounded label presence rather than exact gauge values that other parallel
tests can overwrite.

**Step 3: Verify documentation and telemetry tests**

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server server::telemetry`

Run: `rg -n "seven-day|seven days" docs`

Expected: telemetry tests PASS; remaining seven-day references describe only the orphan-task policy
or historical context.

**Step 4: Commit**

```bash
git add docs config.example.toml bin/raiko2/src/server/telemetry.rs
git commit -m "docs(runtime): align retention operations"
```

### Task 8: Verify the integrated change

**Files:**
- Verify only.

**Step 1: Format and check the patch**

Run: `cargo fmt --all`

Run: `git diff --check origin/main`

Expected: PASS.

**Step 2: Run focused runtime and server tests**

Run: `cargo test -p raiko2-runtime`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server`

Expected: runtime and server suites PASS.

**Step 3: Run the repository verification lanes**

Run: `cargo test -p raiko2-queue -p raiko2-runtime`

Run: `cargo test -p raiko2-provider -p raiko2-pipeline -p preflight`

Run: `cargo clippy --workspace -- -D warnings`

Run: `cargo fmt --all -- --check`

Expected: PASS.

**Step 4: Review path and filename hygiene**

Inspect every added line for absolute paths, user-specific directories, machine-specific values, and
person-identifying fixture names. Replace any occurrence with repository-relative or temporary paths
and neutral fixture labels.

**Step 5: Update the PR**

Summarize the changed lifecycle model, list every verification command and result, and explicitly
map the new regressions to the review findings before pushing the branch.

### Task 9: Make task-record ownership the retention boundary

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `bin/raiko2/src/server/lifecycle.rs`
- Test: `crates/runtime/src/lib.rs`

**Step 1: Write failing ownership tests**

Assert that failed and cancelled task records retain their proof artifacts and pending publications
until the terminal record expires. Assert that shared proof remains retained while any task record
references it, regardless of status, and becomes reclaimable after the final owner record is removed.

**Step 2: Replace usable-owner checks with retained-record ownership**

Build an ownership index once per runtime-state snapshot. Artifact and pending selection use exact
task incarnations from this index rather than scanning the task table once per candidate. Root
preparation retires only task records; it must not invalidate referenced artifacts. Pending
retention follows authoritative task references even when a durable intent's owner list is stale;
publication activation continues to use the stricter live-owner predicate.

**Step 3: Run the runtime tests**

Run: `cargo test -p raiko2-runtime --lib retention`

Expected: PASS.

### Task 10: Make cleanup scheduling failure-atomic

**Files:**
- Modify: `bin/raiko2/src/server/task_cleanup.rs`
- Modify: `bin/raiko2/src/server/lifecycle.rs`
- Test: `bin/raiko2/src/server/task_cleanup.rs`

**Step 1: Write failing scheduler tests**

Cover a later-lane error after root selection, an error while loading mixed retry/fresh candidates,
and a root skipped during exact finalization. Assert that retry identities remain queued and fresh
cursors do not advance past unprocessed work.

**Step 2: Implement selection acknowledgement**

Peek retry identities without removing them. Carry selected retry and fresh identities through lane
execution, acknowledge retries only after successful completion, and enqueue all unprocessed fresh
identities on lane failure. Advance fresh cursors only after selection succeeds. Run root retirement
before artifact and pending cleanup.

**Step 3: Run cleanup tests**

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server server::task_cleanup`

Expected: PASS.

### Task 11: Bound cleanup writes and stale reconciliation

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `bin/raiko2/src/server/lifecycle.rs`
- Test: `bin/raiko2/src/server/task_cleanup.rs`

**Step 1: Restore a representative write-count regression**

Include multiple roots, artifacts, and pending publications. Assert that increasing batch cardinality
does not increase authoritative runtime-state writes linearly.

**Step 2: Remove per-item authoritative mutations**

Fold pending cleanup and stale artifact reconciliation into lane-level exact finalization. Never adopt
a newly observed canonical descriptor into an `Invalidated` local record. A changed descriptor is
stale work and must remain protected for publication reconciliation.

**Step 3: Run lifecycle and cleanup tests**

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server retention`

Expected: PASS.

### Task 12: Observe overdue active tasks without deleting them

**Files:**
- Modify: `bin/raiko2/src/server/task_cleanup.rs`
- Modify: `bin/raiko2/src/server/telemetry.rs`
- Test: `bin/raiko2/src/server/task_cleanup.rs`

**Step 1: Write a failing active-task test**

Seed an allocated or running task older than six hours. Assert that cleanup preserves it and emits
only one observation for the task incarnation across repeated passes. A new incarnation may emit a
new observation.

**Step 2: Add bounded observation state**

Scan at most one configured batch per pass. Deduplicate warnings by task incarnation in a bounded
process-local set, emit structured age/status/pipeline/route fields, and increment a fixed-label
counter. Run this observation before orphan cancellation or any retention state transition. The
observation must not mutate, retire, or remove the active task. Restart may emit the warning again
because process-local observation state is not authority.

**Step 3: Run cleanup and telemetry tests**

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server server::task_cleanup`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server server::telemetry`

Expected: PASS.

### Task 13: Keep aggregate dependency recovery outside retention

**Files:**
- Modify: `crates/engine/src/lib.rs`
- Test: `crates/engine/src/lib.rs`

**Step 1: Add a failing lifecycle-isolation regression**

Run a proposal to success, remove its proof artifact outside the runtime lifecycle, and execute the
dependent aggregate. Assert that the aggregate fails through its ordinary execution policy without
resetting the succeeded proposal. External proof-store damage is recovered by explicit startup
cleanup and request resubmission, not by an aggregate-owned proposal lifecycle.

**Step 2: Remove cross-lifecycle queue recovery**

Remove the aggregate missing-artifact task-store transition and its scheduler wrapper. Proposal
retry remains owned by the proposal execution policy. Aggregate readiness remains owned by normal
dependency completion, and retention never changes either task's execution state.

**Step 3: Run queue and engine tests**

Run: `cargo test -p raiko2-queue`

Run: `cargo test -p raiko2-engine missing_aggregate_artifact_is_terminal`

Expected: PASS.

### Task 14: Keep cleanup lanes single-purpose

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `bin/raiko2/src/server/lifecycle.rs`
- Modify: `bin/raiko2/src/server/task_cleanup.rs`
- Modify: `docs/plans/2026-08-19-runtime-retention-batch-gc-design.md`
- Test: `crates/runtime/src/lib.rs`
- Test: `bin/raiko2/src/server/task_cleanup.rs`

**Step 1: Add failing lane-boundary regressions**

Assert that pending retention never deletes a canonical object represented by an artifact record,
that the artifact lane remains its sole owner, that an orphan pass processes at most its independent
64-record bound, and that overdue-active observation does not evict earlier identities merely because
the configured retention batch is small.

**Step 2: Separate cleanup responsibilities**

Make pending retention delete only its pending object and intent when an artifact record exists.
Retain the historical no-artifact-record canonical cleanup as a compatibility case. Keep canonical
artifact invalidation and removal in the artifact lane. Restore a fixed orphan-management batch
limit and commit its cursor only after a complete successful page. Keep overdue-active observation
bounded independently from retention batch sizing.

**Step 3: Run focused runtime and server tests**

Run: `cargo test -p raiko2-runtime --lib`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server server::task_cleanup`

Expected: PASS.

### Task 15: Separate retention admission from execution status

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `bin/raiko2/src/server/lifecycle.rs`
- Modify: `bin/raiko2/src/server/task_cleanup.rs`
- Modify: `bin/raiko2/src/server/telemetry.rs`
- Modify: `docs/API.md`
- Modify: `docs/operations.md`
- Modify: `docs/plans/2026-08-19-runtime-retention-batch-gc-design.md`
- Test: `crates/runtime/src/lib.rs`
- Test: `bin/raiko2/src/server/task_cleanup.rs`
- Test: `bin/raiko2/src/server/telemetry.rs`

**Step 1: Add failing terminal-retention regressions**

Assert that batch preparation preserves `Completed` and `Failed`, including proof URI and error,
while setting only a dedicated retention state. Assert that a queue-detach failure leaves those
business fields readable and that a later healthy pass removes the exact task.

**Step 2: Add failing orphan fail-stop observability regressions**

Run two passes against the same permanently failing orphan and assert that the cursor remains pinned,
later terminal retention does not run, the error names the task and reconciliation stage, and the
blocked gauge remains set. Replace the failed dependency, rerun the pass, and assert that retention
resumes and the gauge clears.

**Step 3: Implement the independent retention lifecycle**

Add a Serde-defaulted task retention enum that is omitted from persisted JSON. Mark exact terminal
snapshots as removing without changing `RunnerStatus`, proof URI, error, or `updated_at`. Require the
exact removing snapshot during in-process batch finalization; after restart, select the unchanged
terminal root again. Keep ordinary task cancellation semantics separate from retention removal.

**Step 4: Make orphan fail-stop explicit and bounded**

Add task/stage context to orphan reconciliation failures and maintain a fixed-label blocked gauge.
Keep the cursor rollback and early return: this is an intentional operator-intervention boundary, not
a retry-queue lane. Document that the per-pass orphan bound is `min(cleanup_batch_size, 64)`.

**Step 5: Fix adjacent review assertions and documentation**

Replace the vacuous pending-deletion assertion with its exact expected result. Document overdue
warning saturation and the explicit proof startup-cleanup recovery path without adding task IDs or
other unbounded metric labels.

**Step 6: Run focused and workspace verification**

Run: `cargo test -p raiko2-runtime --lib`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server server::task_cleanup`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server server::telemetry`

Run: `cargo test -p raiko2-queue -p raiko2-engine -p raiko2-runtime --quiet`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server --quiet`

Run: `cargo clippy --workspace -- -D warnings`

Run: `cargo fmt --all -- --check`

Expected: PASS.

### Task 16: Complete interrupted retention without rollback coupling

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `bin/raiko2/src/server/task_cleanup.rs`
- Modify: `docs/plans/2026-08-19-runtime-retention-batch-gc-design.md`
- Test: `crates/runtime/src/lib.rs`
- Test: `bin/raiko2/src/server/task_cleanup.rs`

**Step 1: Keep retention admission process-local**

Assert that `Removing` participates in in-process full-record equality but is omitted from persisted
JSON. A restarted runtime must default the marker to `Retained` and reselect the unchanged expired
terminal root normally.

**Step 2: Finalize committed artifact invalidations regardless of ownership**

Seed an `Invalidated` artifact whose original failed task record still references it. Assert that the
artifact lane selects and finalizes the already-committed invalidation while active artifacts remain
protected by every retained task reference.

**Step 3: Repair pending intent/object hash mismatch**

Seed an unowned pending intent whose expected hash differs from the exact object currently stored at
its pending key. Assert that retention deletes the observed exact object generation and then removes
the unchanged intent record.

**Step 4: Run focused and workspace verification**

Run: `cargo test -p raiko2-runtime --lib`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server server::task_cleanup`

Run: `cargo test -p raiko2-queue -p raiko2-engine -p raiko2-runtime --quiet`

Run: `cargo test -p raiko2 --bin raiko2 --features fixture-server --quiet`

Run: `cargo clippy --workspace -- -D warnings`

Run: `cargo fmt --all -- --check`

Expected: PASS.
