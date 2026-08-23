# Tombstone-Free Artifact Lifecycle Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove proof and canonical-preflight tombstones while preserving crash recovery, exact GCS deletion, retryable publication, and single-process lifecycle ordering.

**Architecture:** Replace the 64-way artifact lock array with a reclaimable full-key lock registry. Use durable runtime `Invalidated` records as the proof deletion fence, exact generation-CAS manifest deletion as the external operation, and canonical preflight single-flight as the preflight ordering mechanism. Remove the online artifact-invalidation API after the runtime path no longer depends on marker reads.

**Tech Stack:** Rust, Tokio, Axum, GCS generation preconditions, serde JSON, bincode, existing raiko2 runtime and pipeline crates.

---

Read `docs/plans/2026-08-23-tombstone-free-artifact-lifecycle-design.md` before starting. Work from a clean branch based on current `origin/main`. Do not edit generated guest ELF files and do not build guest programs: this change is host/runtime-only.

### Task 1: Add A Reclaimable Full-Key Lifecycle Lock Registry

**Files:**
- Create: `crates/runtime/src/artifact_lock.rs`
- Modify: `crates/runtime/src/lib.rs:8-50`
- Modify: `crates/runtime/src/lib.rs:220-315`
- Modify: `crates/runtime/src/lib.rs:650-680`
- Modify: `crates/runtime/src/lib.rs:1236-1265`
- Test: `crates/runtime/src/artifact_lock.rs`
- Test: `crates/runtime/src/lib.rs:5360-5490`

**Step 1: Write failing registry tests**

Cover these behaviors with Tokio tests:

```rust
#[tokio::test]
async fn same_key_resolves_to_the_same_live_mutex() {
    let registry = ArtifactLifecycleLocks::default();
    let key = artifact_key("proof-a");
    let first = registry.resolve(&key);
    let second = registry.resolve(&key);
    assert!(Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn different_keys_do_not_share_a_mutex() {
    let registry = ArtifactLifecycleLocks::default();
    let first = registry.resolve(&artifact_key("proof-a"));
    let second = registry.resolve(&artifact_key("proof-b"));
    assert!(!Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn waiter_keeps_the_registry_entry_live() {
    // Hold the first guard, start a waiter, sweep, and prove a third resolver observes
    // the same Arc rather than creating a second mutex.
}

#[test]
fn dead_weak_entries_are_reclaimed() {
    // Drop all strong handles, sweep under the registry mutex, and assert the entry count is zero.
}
```

Also retain a regression that finds two keys mapping to the same old 64-way shard and proves they can
now make progress independently.

**Step 2: Run the focused test and confirm failure**

Run:

```bash
cargo test -p raiko2-runtime artifact_lock -- --nocapture
```

Expected: compilation fails because `ArtifactLifecycleLocks` does not exist.

**Step 3: Implement the registry without a new dependency**

Use a short-held standard mutex around weak entries and Tokio mutexes for asynchronous keyed guards:

```rust
#[derive(Debug, Default)]
pub(crate) struct ArtifactLifecycleLocks {
    entries: std::sync::Mutex<HashMap<ProofArtifactKey, Weak<tokio::sync::Mutex<()>>>>,
}

impl ArtifactLifecycleLocks {
    pub(crate) fn resolve(&self, key: &ProofArtifactKey) -> Arc<tokio::sync::Mutex<()>> {
        let mut entries = self.entries.lock().expect("artifact lock registry poisoned");
        if let Some(lock) = entries.get(key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        entries.insert(key.clone(), Arc::downgrade(&lock));
        lock
    }

    pub(crate) fn sweep(&self) -> usize {
        let mut entries = self.entries.lock().expect("artifact lock registry poisoned");
        let before = entries.len();
        entries.retain(|_, lock| lock.strong_count() > 0);
        before.saturating_sub(entries.len())
    }
}
```

Do not remove an entry merely because its runtime artifact record disappeared. A holder or waiter
must retain the same `Arc`. Use `lock_owned()` at call sites so the strong handle remains alive for
the complete guard lifetime.

Replace `artifact_lifecycle_locks: [Mutex<()>; 64]`, `DefaultHasher`, and the shard helper with the
registry. Add a helper returning an owned guard for one `ProofArtifactKey`, plus a deterministic
sorted-and-deduplicated batch helper for retention admission/finalization.

**Step 4: Run focused tests**

Run:

```bash
cargo test -p raiko2-runtime artifact_lock -- --nocapture
cargo test -p raiko2-runtime artifact_lifecycle -- --nocapture
```

Expected: all matching tests pass; the old shard-collision test is replaced by the independence test.

**Step 5: Commit**

```bash
git add crates/runtime/src/artifact_lock.rs crates/runtime/src/lib.rs
git commit -m "refactor(runtime): use keyed artifact lifecycle locks"
```

### Task 2: Replace Proof Tombstones With Exact Manifest Deletion

**Files:**
- Modify: `crates/runtime/src/artifact_store.rs:55-90`
- Modify: `crates/runtime/src/artifact_store.rs:201-235`
- Modify: `crates/runtime/src/artifact_store.rs:274-305`
- Modify: `crates/runtime/src/artifact_store.rs:618-680`
- Modify: `crates/runtime/src/artifact_store/gcs.rs:300-335`
- Modify: `crates/runtime/src/artifact_store/gcs.rs:900-1095`
- Test: `crates/runtime/src/artifact_store.rs:1090-1380`
- Test: `crates/runtime/src/artifact_store/gcs_tests.rs:820-940`
- Modify test doubles implementing `ProofObjectStore` under `crates/runtime/src/` and `bin/raiko2/src/server/`

**Step 1: Write failing memory-store and GCS-store tests**

Add tests proving:

- exact current descriptor deletion returns `Removed`;
- deleting an absent manifest returns `Missing` without consulting a marker;
- deleting descriptor A after descriptor B is current returns `Stale` and preserves B;
- no object whose name ends in `.tombstone` is created;
- deleting and recreating the same key permits generation B without marker cleanup.

Use a transport spy in GCS tests to assert the write sequence contains content/manifest operations but
no invalidation-marker create.

**Step 2: Run focused tests and confirm failure**

Run:

```bash
cargo test -p raiko2-runtime artifact_store -- --nocapture
```

Expected: new tests fail against marker-based `invalidate_exact` and `is_invalidated`.

**Step 3: Consolidate the proof-store API**

Replace `ExactInvalidationResult` with:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExactDeleteResult {
    Removed,
    Missing,
    Stale,
}
```

Keep one descriptor-aware operation:

```rust
async fn delete_exact(
    &self,
    key: &ProofArtifactKey,
    descriptor: &ProofArtifactDescriptor,
) -> Result<ExactDeleteResult>;
```

Remove `invalidate_exact`, `is_invalidated`, `MemoryStoreInner::invalidations`, the GCS
`invalidation_name` helper, and all marker read/create logic.

The GCS implementation must:

1. read the current manifest and generation;
2. return `Missing` if absent;
3. compare URI, content hash, and generation and return `Stale` on mismatch;
4. call generation-conditional delete;
5. on an ambiguous delete error, read back the manifest and classify missing, unchanged, or changed;
6. never delete immutable content synchronously.

Update all test stores and wrappers mechanically without changing their injected-failure semantics.

**Step 4: Run runtime and server compile tests**

Run:

```bash
cargo test -p raiko2-runtime artifact_store -- --nocapture
cargo test -p raiko2 --bin raiko2 --no-default-features -- server::proof_artifact
```

Expected: focused store tests pass and every `ProofObjectStore` implementation compiles.

**Step 5: Commit**

```bash
git add crates/runtime/src bin/raiko2/src/server
git commit -m "refactor(runtime): delete proof manifests without tombstones"
```

### Task 3: Remove Canonical Preflight Tombstones And Preserve Recompute Semantics

**Files:**
- Modify: `crates/pipeline/src/forks/shasta/preflight_cache.rs:20-100`
- Modify: `crates/pipeline/src/forks/shasta/preflight_cache.rs:295-525`
- Test: `crates/pipeline/src/forks/shasta/preflight_cache.rs:600-980`
- Modify: `crates/runtime/src/artifact_store.rs:430-475`
- Modify: `crates/runtime/src/artifact_store/gcs.rs:340-382`
- Modify: `crates/runtime/src/artifact_store/gcs.rs:640-715`
- Modify: `crates/runtime/src/artifact_store/gcs.rs:732-890`
- Test: `crates/runtime/src/artifact_store.rs:930-1040`
- Test: `crates/runtime/src/artifact_store/gcs_tests.rs:400-480`

**Step 1: Add failing preflight coordinator tests**

Test these separately:

```rust
#[tokio::test]
async fn failed_build_is_not_negative_cached_and_next_request_rebuilds() { /* two calls */ }

#[tokio::test]
async fn invalid_cached_entry_is_deleted_then_rebuilt_without_marker() { /* A -> B */ }

#[tokio::test]
async fn delete_failure_returns_validated_uncached_build_and_skips_publish() { /* A remains */ }

#[tokio::test]
async fn stale_delete_reloads_and_validates_current_winner_once() { /* A observation, B winner */ }
```

Assert that concurrent waiters share the same leader error or rebuilt value and that a later request
after a failed build invokes the builder again.

**Step 2: Run tests and confirm failure**

Run:

```bash
cargo test -p raiko2-pipeline preflight_cache -- --nocapture
cargo test -p raiko2-runtime canonical_preflight -- --nocapture
```

Expected: tombstone-oriented result assertions fail.

**Step 3: Implement tombstone-free preflight deletion**

Rename `CanonicalPreflightInvalidateResult` and
`invalidate_canonical_preflight_exact` to deletion terminology with `Removed`, `Missing`, and
`Stale` outcomes. Remove preflight invalidation sets and GCS invalidation-name/read/create helpers.

Change `try_load_canonical` to return a richer internal outcome so `load_or_build_canonical` knows
whether publication remains safe:

```rust
enum CanonicalLoad {
    Hit(CanonicalShastaPreflightV1),
    Miss { publish: bool },
}
```

- invalid A deleted or missing: `Miss { publish: true }`;
- delete transport failure: `Miss { publish: false }`;
- stale descriptor: reload once, validate the winner, otherwise use `publish: false`;
- ordinary cache read failure: preserve existing best-effort rebuild behavior;
- build or validation failure: return the error and let single-flight remove the completed entry.

When `publish` is false, return the validated in-memory core without calling
`put_canonical_preflight_if_absent`.

**Step 4: Run focused tests**

Run:

```bash
cargo test -p raiko2-pipeline preflight_cache -- --nocapture
cargo test -p raiko2-runtime canonical_preflight -- --nocapture
```

Expected: all focused tests pass, including failed-build recomputation.

**Step 5: Commit**

```bash
git add crates/pipeline/src/forks/shasta/preflight_cache.rs crates/runtime/src/artifact_store.rs crates/runtime/src/artifact_store/gcs.rs crates/runtime/src/artifact_store/gcs_tests.rs crates/runtime/src/lib.rs
git commit -m "refactor(preflight): rebuild invalid cache entries without tombstones"
```

### Task 4: Fence Proof Publication With Durable Invalidated State

**Files:**
- Modify: `crates/runtime/src/publication.rs:1-220`
- Modify: `crates/runtime/src/lib.rs:1280-1430`
- Modify: `crates/runtime/src/lib.rs:1830-1930`
- Modify: `crates/runtime/src/lib.rs:2450-2570`
- Test: `crates/runtime/src/publication.rs:230-end`
- Test: `crates/runtime/src/lib.rs:8350-8560`
- Modify: `bin/raiko2/src/server/state/runtime_observer.rs:100-140`
- Modify: `bin/raiko2/src/server/state/runtime_observer.rs:735-1085`
- Test: `bin/raiko2/src/server/state/runtime_observer_state_tests.rs`

**Step 1: Add failing publication race tests**

Cover:

- `Invalidated(A)` causes publication to return cleanup-pending before any content/manifest write;
- cleanup-pending maps to `ProofCommitAttempt::Retryable`, not `Invalidated`;
- exhausted local publication retries leave the root `Allocated`, never `Cancelled`;
- a current `Pending` or `Active` matching descriptor is still reusable;
- an untracked canonical manifest is recorded as `Invalidated` and is not adopted;
- no publication path reads a tombstone.

Use the existing blocking/failure-injection stores to pause publication at pre-write and
post-manifest boundaries.

**Step 2: Run focused tests and confirm failure**

Run:

```bash
cargo test -p raiko2-runtime publication -- --nocapture
cargo test -p raiko2 --bin raiko2 --no-default-features runtime_observer -- --nocapture
```

Expected: tests fail because current publication writes before consulting local lifecycle and maps
`ProofArtifactPublicationInvalidated` to cancellation.

**Step 3: Add a distinct retryable cleanup-pending error**

Introduce and export:

```rust
#[derive(Debug)]
pub struct ProofArtifactCleanupPending {
    proof_ref: String,
}
```

Do not reuse `ProofArtifactPublicationInvalidated`; that type currently selects the destructive
invalidated/cancelled observer path. Map `ProofArtifactCleanupPending` to
`ProofCommitAttempt::Retryable`.

**Step 4: Reorder publication under the keyed lock**

Before `put_if_absent`, inspect the exact local lifecycle. If it is `Invalidated`, return
`ProofArtifactCleanupPending`. Remove both external `proof_artifact_descriptor_is_invalidated`
checks from `commit_proof_artifact_publication`.

After manifest publication, retain exact local registration and owner checks. If the returned
manifest has no matching runtime record or recoverable outbox, persist an exact `Invalidated` record
and return cleanup-pending. Do not delete the manifest inline while holding the keyed lock.

Keep proposal, aggregate input, and aggregate output on the same method; do not create lane-specific
publication implementations.

**Step 5: Run focused tests**

Run:

```bash
cargo test -p raiko2-runtime publication -- --nocapture
cargo test -p raiko2 --bin raiko2 --no-default-features runtime_observer -- --nocapture
```

Expected: race tests pass and cleanup-pending preserves retryable root state.

**Step 6: Commit**

```bash
git add crates/runtime/src/publication.rs crates/runtime/src/lib.rs bin/raiko2/src/server/state/runtime_observer.rs bin/raiko2/src/server/state/runtime_observer_state_tests.rs
git commit -m "fix(runtime): fence publication with durable artifact state"
```

### Task 5: Make Retention A Three-Phase Keyed Batch

**Files:**
- Modify: `crates/runtime/src/lib.rs:2200-2535`
- Modify: `crates/runtime/src/lib.rs:3500-3605`
- Modify: `bin/raiko2/src/server/lifecycle.rs:356-520`
- Modify: `bin/raiko2/src/server/lifecycle.rs:568-640`
- Modify: `bin/raiko2/src/server/task_cleanup.rs:300-475`
- Test: `crates/runtime/src/lib.rs:6400-7750`
- Test: `bin/raiko2/src/server/task_cleanup.rs:1750-end`

**Step 1: Add failing interleaving and crash tests**

Add deterministic tests for:

1. publication finishes before invalidation admission, causing ownership recheck to retain A;
2. logical invalidation wins, causing publication to receive cleanup-pending;
3. publication during external delete performs no store write;
4. delete failure leaves exact `Invalidated(A)` and retry identity;
5. crash/reload before external delete resumes deletion;
6. crash/reload after manifest removal removes the exact runtime record;
7. a stale retry candidate A cannot remove current runtime record or manifest B;
8. multiple artifact keys use one mutation for admission and one for successful finalization.

**Step 2: Run tests and confirm failure**

Run:

```bash
cargo test -p raiko2-runtime retention -- --nocapture
cargo test -p raiko2 --bin raiko2 --no-default-features task_cleanup -- --nocapture
```

Expected: tests expose missing keyed admission/finalization and marker-dependent finalization.

**Step 3: Implement Phase 1 keyed admission**

Resolve candidate keys, sort and deduplicate them, acquire owned guards in that order, then perform
the existing exact ownership recheck and one batch mutation to mark `Invalidated`. Release all guards
before object-store deletion.

Never hold the process-wide execution lifecycle gate or keyed guards while waiting for proof manifest
deletion.

**Step 4: Implement Phase 2 exact deletion**

Replace `finalize_proof_artifact_invalidation` marker semantics with exact manifest deletion. Keep
bounded concurrency at eight. Classify:

- `Removed`/`Missing`: eligible for Phase 3;
- `Stale`: recheck authoritative state and drop only a stale candidate, never the observed object;
- error: retain the exact runtime record and enqueue retry.

**Step 5: Implement Phase 3 keyed runtime finalization**

Acquire keyed guards for successful exact descriptors in deterministic order. In one runtime-state
mutation, remove only records still equal to `Invalidated(A)` and prune only unowned matching pending
intents. Release guards after the mutation commits.

Update `reconcile_invalidated_proof_artifacts` so restart performs Phase 2 and Phase 3; it must not
leave successfully deleted invalidated records in the snapshot.

**Step 6: Run focused tests**

Run:

```bash
cargo test -p raiko2-runtime retention -- --nocapture
cargo test -p raiko2 --bin raiko2 --no-default-features task_cleanup -- --nocapture
```

Expected: all race, crash, stale-candidate, and batch-write tests pass.

**Step 7: Commit**

```bash
git add crates/runtime/src/lib.rs bin/raiko2/src/server/lifecycle.rs bin/raiko2/src/server/task_cleanup.rs
git commit -m "fix(runtime): finalize artifact retention without tombstones"
```

### Task 6: Align Pending Publication And Startup Recovery

**Files:**
- Modify: `crates/runtime/src/lib.rs:2580-3390`
- Modify: `crates/runtime/src/lib.rs:4400-4520`
- Modify: `bin/raiko2/src/server/state/mod.rs:160-230`
- Modify: `bin/raiko2/src/server/state/runtime_observer.rs:720-940`
- Test: `crates/runtime/src/lib.rs:7800-9200`
- Test: `bin/raiko2/src/server/state/runtime_observer.rs:4400-end`

**Step 1: Add failing recovery tests**

Cover:

- matching untracked canonical content becomes durable `Invalidated` authority before deletion;
- changed untracked canonical content is not adopted or deleted;
- pending object deletion failure preserves its durable intent;
- restart with manifest missing and `Invalidated(A)` completes runtime cleanup;
- startup proof cleanup creates no tombstones;
- aggregate inputs and final aggregate proof use the same recovery rules.

**Step 2: Run tests and confirm failure**

Run:

```bash
cargo test -p raiko2-runtime pending_publication -- --nocapture
cargo test -p raiko2 --bin raiko2 --no-default-features startup_cleanup -- --nocapture
```

Expected: marker-dependent assertions or incomplete runtime-record cleanup fail.

**Step 3: Implement recovery changes**

Replace pending canonical invalidation with durable exact `Invalidated` registration followed by the
same Phase 2/Phase 3 helpers used by retention. Do not duplicate deletion logic in the pending lane.

Keep private pending-object deletion generation protected. `runtime.startup_cleanup` remains a
direct scoped, generation-aware namespace cleanup and must neither read nor create tombstones.

**Step 4: Run focused tests**

Run:

```bash
cargo test -p raiko2-runtime pending_publication -- --nocapture
cargo test -p raiko2 --bin raiko2 --no-default-features startup_cleanup -- --nocapture
```

Expected: all recovery tests pass.

**Step 5: Commit**

```bash
git add crates/runtime/src/lib.rs bin/raiko2/src/server/state
git commit -m "fix(runtime): recover pending publications without markers"
```

### Task 7: Remove The Online Artifact Invalidation API

**Files:**
- Modify: `bin/raiko2/src/server/routes/v4.rs:7-23`
- Modify: `bin/raiko2/src/server/handlers/proof_api/v4.rs:145-810`
- Modify: `bin/raiko2/src/server/handlers/proof_types/v4.rs:85-150`
- Modify: `bin/raiko2/src/server/e2e.rs:1080-1160`
- Delete: `bin/raiko2/src/server/e2e/invalidation_prefix.rs`
- Delete: `bin/raiko2/src/server/e2e/invalidation_range.rs`
- Modify: `docs/API.md:20-40`
- Modify: `docs/API.md:130-155`
- Modify: `docs/API.md:570-650`

**Step 1: Add a failing route-absence test**

Add an API test asserting `POST /v4/prover/invalidate-artifacts` is not registered while
`POST /v4/prover/clear` remains registered and ACL protected.

**Step 2: Run the test and confirm failure**

Run:

```bash
cargo test -p raiko2 --bin raiko2 --no-default-features invalidate_artifacts_route_is_absent -- --nocapture
```

Expected: request reaches the existing route instead of returning not-found.

**Step 3: Remove the route and exclusive implementation surface**

Delete the handler, request/response structs, selector helpers, range/prefix logic, and tests used only
by the endpoint. Do not remove `ServerAclFeature::ProverClear` because `/v4/prover/clear` still uses
it. Do not change active-task clear behavior.

Update `docs/API.md` so operator guidance points to:

- `/v4/prover/clear` for active work;
- `runtime.startup_cleanup = ["proof"]` for upgrade-time proof cleanup;
- retention for normal terminal cleanup.

**Step 4: Run server tests**

Run:

```bash
cargo test -p raiko2 --bin raiko2 --no-default-features invalidate_artifacts_route_is_absent -- --nocapture
cargo test -p raiko2 --bin raiko2 --no-default-features clear_prover -- --nocapture
```

Expected: route-absence and clear behavior tests pass.

**Step 5: Commit**

```bash
git add bin/raiko2/src/server docs/API.md
git commit -m "refactor(api): remove online artifact invalidation"
```

### Task 8: Add Lifecycle Observability And Update Design References

**Files:**
- Modify: `bin/raiko2/src/server/telemetry.rs:110-180`
- Modify: `bin/raiko2/src/server/telemetry.rs:350-455`
- Modify: `bin/raiko2/src/server/telemetry.rs:700-end`
- Modify: `bin/raiko2/src/server/task_cleanup.rs:180-230`
- Modify: `docs/plans/2026-08-19-runtime-retention-batch-gc-design.md`
- Modify: `docs/plans/2026-07-30-canonical-preflight-cache-design.md`
- Modify: `docs/plans/2026-08-23-tombstone-free-artifact-lifecycle-design.md`

**Step 1: Add failing telemetry tests**

Assert bounded-label metrics exist for:

- lifecycle lock wait and hold duration;
- lock registry live/dead/swept entries;
- exact proof delete outcomes;
- cleanup-pending publication;
- current invalidated artifacts and retry queue length;
- preflight invalid-cache deletion and uncached fallback.

Do not add proposal IDs, task IDs, proof refs, hashes, generations, routes, or network pairs as
unbounded metric labels unless an existing bounded label set already permits them.

**Step 2: Run telemetry tests and confirm failure**

Run:

```bash
cargo test -p raiko2 --bin raiko2 --no-default-features telemetry -- --nocapture
```

Expected: new metric names are absent.

**Step 3: Implement metrics and update prior design documents**

Add counters/gauges/histograms using the existing telemetry registration style. Update older design
documents where they state tombstones are permanent authority; link to the superseding design rather
than rewriting their historical context.

**Step 4: Run telemetry and documentation checks**

Run:

```bash
cargo test -p raiko2 --bin raiko2 --no-default-features telemetry -- --nocapture
git diff --check
```

Expected: telemetry tests pass and the diff has no whitespace errors.

**Step 5: Commit**

```bash
git add bin/raiko2/src/server/telemetry.rs bin/raiko2/src/server/task_cleanup.rs docs/plans
git commit -m "feat(metrics): observe tombstone-free artifact cleanup"
```

### Task 9: Run Cross-Crate Verification And Review The Migration Boundary

**Files:**
- Verify: all modified files
- Verify: `README.md`
- Verify: `docs/API.md`
- Verify: `config.example.toml`

**Step 1: Format**

Run:

```bash
cargo fmt --all
git diff --check
```

Expected: both commands succeed.

**Step 2: Run focused crate tests**

Run:

```bash
cargo test -p raiko2-pipeline
cargo test -p raiko2-runtime
cargo test -p raiko2-queue
cargo test -p raiko2 --bin raiko2 --no-default-features
```

Expected: all tests pass.

**Step 3: Run repository-required checks for shared runtime and API behavior**

Run:

```bash
cargo clippy --workspace -- -D warnings
cargo test -p raiko2-provider -p raiko2-pipeline -p preflight
cargo test -p raiko2-queue -p raiko2-runtime
cargo test -p raiko2 --bin raiko2 --features fixture-server
```

Expected: all commands pass. Do not run guest builds because no guest/proof-format contract changed.

**Step 4: Audit removed concepts and path hygiene**

Run:

```bash
rg -n "tombstone|is_invalidated|invalidate_exact|invalidate_canonical_preflight_exact|/v4/prover/invalidate-artifacts|ARTIFACT_LIFECYCLE_LOCK_SHARDS" crates/runtime crates/pipeline bin/raiko2 docs/API.md config.example.toml
rg -n '(/(home|Users)/)|[A-Z]:\\Users\\' crates/runtime crates/pipeline bin/raiko2 docs
```

Expected: the first command returns only historical design discussion or intentionally renamed test
fixtures; production code contains no marker or removed endpoint path. The second command finds no
new machine-specific or person-identifying paths.

**Step 5: Review rollout requirements**

Before merge, record these operator checks in the PR description:

- one replica per runtime namespace;
- `Recreate` replacement strategy;
- canary and production use different namespaces;
- historical tombstones remain during the binary rollback window;
- after the rollback window, delete old marker objects with a scoped generation-aware operation;
- monitor cleanup-pending, invalidated-record age, exact-delete failures, lock wait, and runtime-state
  CAS failures.

Do not modify GKE resources or bucket lifecycle policy from this repository.

**Step 6: Commit final formatting or documentation corrections**

```bash
git add crates/runtime crates/pipeline bin/raiko2 docs README.md config.example.toml
git commit -m "chore(runtime): finalize tombstone-free lifecycle migration"
```

Skip this commit if verification produced no changes.
