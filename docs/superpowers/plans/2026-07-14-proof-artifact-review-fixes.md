# Proof Artifact Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make proof artifact expiry API-safe, restart-stable, query-efficient, observable, crash-recoverable, and serialized with explicit invalidation.

**Architecture:** Keep `proof_artifacts.updated_at` as the publication clock, but split normal publication from restoration so restarts preserve age. Move expired-candidate selection outside the cleanup lock, query only active roots, use an ordered composite index, and atomically queue file-deletion tombstones before unlinking. Missing root proof files become explicit not-found responses, while invalid metadata remains globally fail-closed with counters.

**Tech Stack:** Rust, Tokio, Axum, SQLite via `rusqlite`/`tokio-rusqlite`, Prometheus, Cargo.

## Global Constraints

- Keep `runtime.inactive_ttl_secs` and `runtime.proof_artifact_ttl_secs` independent.
- Preserve bounded cleanup work: at most 64 expired artifact candidates and 64 pending file deletions per maintenance tick.
- Keep artifact cleanup fail-closed when any allocated or running task has invalid metadata.
- Do not reuse a live-reference snapshot across maintenance ticks.
- Keep the runtime root row after its proof artifact expires when root cleanup is disabled.
- Keep `docs/superpowers` out of the final pull-request diff.
- Follow strict RED/GREEN TDD for every production behavior change.

---

### Task 1: Runtime publication age, active queries, ordered pagination, and tombstones

**Files:**
- Modify: `crates/runtime/src/lib.rs:26-70`
- Modify: `crates/runtime/src/lib.rs:529-780`
- Modify: `crates/runtime/src/lib.rs:1199-1243`
- Test: `crates/runtime/src/lib.rs:1390-1600`

**Interfaces:**
- Consumes: `ProofArtifactRegistration`, `ProofArtifactRecord`, `RuntimeTaskRecord`, `ProofArtifactCursor`.
- Produces:
  - `ProofArtifactFileDeletion { proof_path, queued_at, attempts, last_error }`
  - `restore_proof_artifact(registration, published_at) -> Result<()>`
  - `list_active_tasks() -> Result<Vec<RuntimeTaskRecord>>`
  - `queue_proof_artifact_file_deletion_if_unchanged(pair, proof_ref, expected_updated_at) -> Result<Option<String>>`
  - `queue_proof_artifact_file_deletion(pair, proof_ref) -> Result<Option<ProofArtifactRecord>>`
  - `list_proof_artifact_file_deletions(limit) -> Result<Vec<ProofArtifactFileDeletion>>`
  - `remove_proof_artifact_file_deletion(path) -> Result<()>`
  - `record_proof_artifact_file_deletion_failure(path, error) -> Result<()>`
  - `proof_artifact_path_is_registered(path) -> Result<bool>`

- [ ] **Step 1: Add failing publication/restoration tests**

Add tests that set an artifact timestamp to `now + 10_000`, call normal publication, and assert the
timestamp moves back below the future value. Add a second test that calls the wished-for
`restore_proof_artifact` twice and asserts the original supplied timestamp remains unchanged:

```rust
#[tokio::test]
async fn proof_artifact_publication_replaces_future_timestamp() -> anyhow::Result<()> {
    let runtime = RuntimeManager::new(unique_root("artifact-future-clock"))?;
    register_test_artifact(&runtime, "proposal-a").await?;
    let future = now_ts().saturating_add(10_000);
    set_artifact_updated_at(&runtime, "taiko_dev/ethereum", "proposal-a", future).await?;
    register_test_artifact(&runtime, "proposal-a").await?;
    let restored = runtime.get_proof_artifact("taiko_dev/ethereum", "proposal-a").await?.unwrap();
    assert!(restored.updated_at < future);
    Ok(())
}

#[tokio::test]
async fn proof_artifact_restore_preserves_publication_timestamp() -> anyhow::Result<()> {
    let runtime = RuntimeManager::new(unique_root("artifact-restore-age"))?;
    let registration = test_artifact_registration(&runtime, "proposal-a");
    runtime.restore_proof_artifact(registration.clone(), 123).await?;
    runtime.restore_proof_artifact(registration, 456).await?;
    assert_eq!(runtime.get_proof_artifact("taiko_dev/ethereum", "proposal-a").await?.unwrap().updated_at, 123);
    Ok(())
}
```

- [ ] **Step 2: Run the publication/restoration tests RED**

Run:

```bash
cargo test -p raiko2-runtime proof_artifact_publication_replaces_future_timestamp -- --nocapture
cargo test -p raiko2-runtime proof_artifact_restore_preserves_publication_timestamp -- --nocapture
```

Expected: the future-clock assertion fails under the monotonic `previous + 1` clause, and the restore test fails to compile because `restore_proof_artifact` does not exist.

- [ ] **Step 3: Implement normal and restore-specific upserts**

Change normal conflict handling to `updated_at = excluded.updated_at`. Add a restoration method whose conflict clause updates routing/path fields without assigning `updated_at`:

```rust
pub async fn restore_proof_artifact(
    &self,
    registration: ProofArtifactRegistration,
    published_at: i64,
) -> Result<()> {
    let conn = self.connection().await?;
    conn.call(move |conn| {
        conn.execute(
            r"INSERT INTO proof_artifacts
              (network_pair, proof_ref, pipeline_key, route, proof_path, updated_at)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6)
              ON CONFLICT(network_pair, proof_ref) DO UPDATE SET
                pipeline_key = excluded.pipeline_key,
                route = excluded.route,
                proof_path = excluded.proof_path",
            params![registration.network_pair, registration.proof_ref,
                registration.pipeline_key.as_str(), registration.route.to_string(),
                registration.proof_path, published_at],
        )?;
        Ok(())
    }).await.context("failed to restore proof artifact")?;
    Ok(())
}
```

- [ ] **Step 4: Run publication/restoration tests GREEN**

Run the two commands from Step 2. Expected: both pass.

- [ ] **Step 5: Add failing active-query and tombstone transaction tests**

Add tests that register active and terminal runtime rows and assert `list_active_tasks` returns only
`allocated`/`running`. Add a tombstone test that conditionally removes an artifact, asserts the
visible row is absent, and asserts one deletion record preserves its file path. Add a re-registration
test asserting `proof_artifact_path_is_registered` becomes true after a new publication.

```rust
let removed_path = runtime
    .queue_proof_artifact_file_deletion_if_unchanged(pair, proof_ref, artifact.updated_at)
    .await?
    .expect("matching row queued");
assert!(runtime.get_proof_artifact(pair, proof_ref).await?.is_none());
let queued = runtime.list_proof_artifact_file_deletions(64).await?;
assert_eq!(queued[0].proof_path, removed_path);
```

- [ ] **Step 6: Run active-query and tombstone tests RED**

Run:

```bash
cargo test -p raiko2-runtime runtime_manager_lists_only_active_tasks -- --nocapture
cargo test -p raiko2-runtime runtime_manager_queues_artifact_file_deletion_atomically -- --nocapture
```

Expected: compilation fails because the new runtime interfaces do not exist.

- [ ] **Step 7: Implement active query and durable tombstones**

Create the tombstone table in `migrate_runtime_schema`. Implement `list_active_tasks` with:

```sql
SELECT task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
       proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
       remote_tx_hash, proof_path, error, metadata_json, request_fingerprint, updated_at
FROM runtime_tasks
WHERE runner_status IN ('allocated', 'running')
ORDER BY updated_at ASC, task_id ASC
```

Implement queueing inside a SQLite transaction: select the matching artifact, insert its path into
`proof_artifact_file_deletions`, delete the visible row, and commit. Bound stored error text to 1024
bytes before assigning `last_error`.

- [ ] **Step 8: Replace the expiry index and cursor predicate**

Migration SQL:

```sql
DROP INDEX IF EXISTS proof_artifacts_updated_at_idx;
CREATE INDEX IF NOT EXISTS proof_artifacts_expiry_cursor_idx
ON proof_artifacts(updated_at, network_pair, proof_ref);
```

Cursor predicate:

```sql
WHERE updated_at <= ?1
  AND (updated_at, network_pair, proof_ref) > (?2, ?3, ?4)
ORDER BY updated_at ASC, network_pair ASC, proof_ref ASC
LIMIT ?5
```

Extend the pagination test to inspect `pragma_index_info('proof_artifacts_expiry_cursor_idx')` and
assert the ordered columns are `updated_at`, `network_pair`, `proof_ref`.

- [ ] **Step 9: Run Task 1 GREEN and commit**

Run:

```bash
cargo fmt --all -- --check
cargo test -p raiko2-runtime
```

Expected: all runtime tests pass. Commit:

```bash
git add crates/runtime/src/lib.rs
git commit -m "fix(runtime): preserve artifact age and deletion retries"
```

---

### Task 2: Restart-stable restoration and explicit expired-proof API response

**Files:**
- Modify: `bin/raiko2/src/server/state/mod.rs:535-662`
- Modify: `bin/raiko2/src/server/handlers/proof_api.rs:1649-1761`
- Test: `bin/raiko2/src/server/state/mod.rs:1090-1270`
- Test: `bin/raiko2/src/server/handlers/proof_api.rs:3400-3520`

**Interfaces:**
- Consumes: Task 1 `RuntimeManager::restore_proof_artifact`.
- Produces: restart restoration that never refreshes an existing artifact age and an HTTP 404 for a missing persisted root proof.

- [ ] **Step 1: Add failing restoration integration tests**

Strengthen restoration tests to set an artifact row to timestamp `123`, run
`restore_proof_artifacts_from_runtime_tasks`, and assert it remains `123`. Add a missing-row case whose
runtime record has `updated_at = 456` and assert the restored artifact row receives `456`.

- [ ] **Step 2: Run restoration tests RED**

Run:

```bash
RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 restore_proof_artifacts -- --nocapture
```

Expected: existing timestamps advance to current time because restoration calls normal upsert.

- [ ] **Step 3: Route restoration through the restore-specific API**

Replace both restoration upserts with:

```rust
runtime
    .restore_proof_artifact(
        ProofArtifactRegistration {
            network_pair: metadata.network_pair.clone(),
            proof_ref,
            pipeline_key: record.pipeline_key,
            route: record.route,
            proof_path: artifact_path.display().to_string(),
        },
        record.updated_at,
    )
    .await?;
```

- [ ] **Step 4: Run restoration tests GREEN**

Run the Step 2 command. Expected: all restoration tests pass.

- [ ] **Step 5: Add the failing missing-root-proof test**

Call `load_persisted_root_proof_material` with a completed record whose `proof_path` points to a
nonexistent file and assert:

```rust
let err = load_persisted_root_proof_material(&record)
    .await
    .expect_err("expired proof must be not found");
assert_eq!(err.status, StatusCode::NOT_FOUND);
assert!(err.message.contains("proof artifact expired or unavailable"));
```

- [ ] **Step 6: Run the missing-root-proof test RED**

Run:

```bash
RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 missing_persisted_root_proof_is_not_found -- --nocapture
```

Expected: status is `500 Internal Server Error`.

- [ ] **Step 7: Map only `NotFound` to the explicit expiry response**

Implement:

```rust
let bytes = match fs::read(path).await {
    Ok(bytes) => bytes,
    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
        return Err(ApiError::not_found(format!(
            "proof artifact expired or unavailable: {path}"
        )));
    }
    Err(err) => {
        return Err(ApiError::internal(format!("failed to read proof file {path}: {err}")));
    }
};
```

- [ ] **Step 8: Run Task 2 GREEN and commit**

Run:

```bash
RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 restore_proof_artifacts -- --nocapture
RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 missing_persisted_root_proof_is_not_found -- --nocapture
```

Commit:

```bash
git add bin/raiko2/src/server/state/mod.rs bin/raiko2/src/server/handlers/proof_api.rs
git commit -m "fix(server): keep artifact expiry restart-stable"
```

---

### Task 3: Efficient fail-closed cleanup and retry processing

**Files:**
- Modify: `bin/raiko2/src/server/task_cleanup.rs:20-330`
- Modify: `bin/raiko2/src/server/task_cleanup.rs:665-705`
- Modify: `bin/raiko2/src/server/telemetry.rs:1-280`
- Test: `bin/raiko2/src/server/task_cleanup.rs:790-1125`

**Interfaces:**
- Consumes: Task 1 active-task and tombstone APIs.
- Produces: `ProofArtifactCleanupStats.invalid_metadata_records`, `retained_invalid_metadata`, bounded `ProofArtifactFileDeletionStats`, and `telemetry::record_artifact_cleanup_invalid_metadata(count)`.

- [ ] **Step 1: Add failing invalid-metadata stats test**

Change the existing invalid-live-metadata regression to expect `Ok(stats)` rather than `Err`, then
assert the cursor is unchanged, both artifacts survive, `invalid_metadata_records == 1`, and
`retained_invalid_metadata == 2`.

- [ ] **Step 2: Run invalid-metadata test RED**

Run:

```bash
RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 artifact_cleanup_fails_closed_on_invalid_live_metadata -- --nocapture
```

Expected: cleanup returns an error instead of structured stats.

- [ ] **Step 3: Implement active-only scan with structured fail-closed stats**

Make `live_proof_artifact_refs` use `runtime.list_active_tasks()`. Collect parse failures instead of
using `?`. When failures are nonzero, return stats without assigning the cursor or deleting rows:

```rust
if live.invalid_metadata_records != 0 {
    telemetry::record_artifact_cleanup_invalid_metadata(live.invalid_metadata_records as u64);
    return Ok(ProofArtifactCleanupStats {
        scanned: artifacts.len(),
        invalid_metadata_records: live.invalid_metadata_records,
        retained_invalid_metadata: artifacts.len(),
        ..ProofArtifactCleanupStats::default()
    });
}
```

Add an unlabeled Prometheus `IntCounter` named
`raiko2_artifact_cleanup_invalid_metadata_total` and increment it by the supplied count.

- [ ] **Step 4: Move candidate selection outside the write guard**

Keep the empty-page fast path before locking. Acquire the write guard only after a nonempty page is
selected, then scan active roots and conditionally queue deletions. Preserve cursor advancement only
after successful active metadata validation.

- [ ] **Step 5: Add failing deletion-retry tests**

Extend the file-delete-failure test to assert one tombstone remains. Add a second pass after replacing
the undeletable directory with a normal file and assert the tombstone and file are removed. Add a
republish test: queue an old deletion, publish the same path again, run retry processing, and assert
the current row/file survives while the obsolete tombstone is cleared.

- [ ] **Step 6: Run deletion-retry tests RED**

Run:

```bash
RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 artifact_file_deletion -- --nocapture
```

Expected: no durable tombstone or retry behavior exists.

- [ ] **Step 7: Implement bounded tombstone processing**

Add `run_proof_artifact_file_deletion_pass(runtime, guard, limit)` and call it on every maintenance
tick, even when both TTL values are `0`. For each queued path while holding the write guard:

```rust
if runtime.proof_artifact_path_is_registered(&deletion.proof_path).await? {
    runtime.remove_proof_artifact_file_deletion(&deletion.proof_path).await?;
    stats.skipped_republished += 1;
    continue;
}
match fs::remove_file(&deletion.proof_path).await {
    Ok(()) => { /* clear tombstone; files_removed += 1 */ }
    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
        /* clear tombstone; files_missing += 1 */
    }
    Err(err) => {
        runtime.record_proof_artifact_file_deletion_failure(&deletion.proof_path, &err.to_string()).await?;
        stats.failures += 1;
    }
}
```

After queueing an expired artifact row, use the same helper to attempt immediate deletion; only clear
the tombstone on success or missing.

- [ ] **Step 8: Run Task 3 GREEN and commit**

Run:

```bash
cargo fmt --all -- --check
RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 task_cleanup -- --nocapture
RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 telemetry -- --nocapture
```

Commit:

```bash
git add bin/raiko2/src/server/task_cleanup.rs bin/raiko2/src/server/telemetry.rs
git commit -m "fix(server): make artifact cleanup durable and bounded"
```

---

### Task 4: Serialize V4 invalidation and use durable row-first deletion

**Files:**
- Modify: `bin/raiko2/src/server/handlers/proof_api/v4.rs:145-230`
- Modify: `bin/raiko2/src/server/handlers/proof_api/v4.rs:593-653`
- Test: `bin/raiko2/src/server/handlers/proof_api/v4.rs:1510-1850`

**Interfaces:**
- Consumes: Task 1 unconditional deletion queue and tombstone-clearing APIs; Task 3 file deletion behavior.
- Produces: non-dry-run invalidation serialized by the write guard, with cache row removal committed before file unlink.

- [ ] **Step 1: Add failing guard and row-first tests**

Add a test that holds `state.artifact_cleanup_guard.read()`, spawns non-dry-run
`invalidate_artifacts_inner`, and asserts it does not complete within 25 ms. Add a storage-failure test
that makes file removal fail, then asserts the artifact row is gone and a tombstone remains.

- [ ] **Step 2: Run invalidation tests RED**

Run:

```bash
RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 invalidate_artifacts_waits_for_cleanup_guard -- --nocapture
RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 invalidate_artifacts_queues_failed_file_deletion -- --nocapture
```

Expected: invalidation completes while the read guard is held, and no tombstone is created.

- [ ] **Step 3: Acquire the write guard for non-dry-run invalidation**

At the start of `invalidate_artifacts_inner`, before candidate collection:

```rust
let _artifact_guard = if req.dry_run {
    None
} else {
    Some(state.artifact_cleanup_guard.write().await)
};
```

This holds the guard through task and artifact mutation. Dry-run behavior remains lock-free.

- [ ] **Step 4: Replace file-first invalidation with queue-first deletion**

For each matched artifact, call `queue_proof_artifact_file_deletion` first. On `Some(record)`, update
the removed-row count and then attempt the file unlink. Clear the tombstone on success or missing;
record the failure and leave the tombstone on any other error.

- [ ] **Step 5: Run Task 4 GREEN and commit**

Run:

```bash
cargo fmt --all -- --check
RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 invalidate_artifacts -- --nocapture
```

Commit:

```bash
git add bin/raiko2/src/server/handlers/proof_api/v4.rs
git commit -m "fix(server): serialize artifact invalidation"
```

---

### Task 5: Documentation, complete verification, temporary-doc removal, and push

**Files:**
- Modify: `docs/API.md:1071-1080`
- Modify: `config.example.toml:54-60`
- Delete before push: `docs/superpowers/specs/2026-07-14-proof-artifact-review-fixes-design.md`
- Delete before push: `docs/superpowers/plans/2026-07-14-proof-artifact-review-fixes.md`

**Interfaces:**
- Consumes: completed behavior from Tasks 1-4.
- Produces: accurate operator documentation, a clean final diff, and an updated remote draft PR branch.

- [ ] **Step 1: Update API and configuration documentation**

Document these exact semantics:

- artifact age advances on proof publication, not process restart;
- a retained completed root returns not-found when its proof artifact has expired;
- malformed active metadata pauses artifact deletion fail-closed and increments a cleanup counter;
- failed filesystem removals remain durably queued and are retried;
- `0` still disables new TTL expiry independently, while already queued file deletions continue.

- [ ] **Step 2: Run focused and full verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p raiko2-runtime
cargo test -p raiko2-queue -p raiko2-runtime
RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2
RISC0_SKIP_BUILD_KERNELS=1 cargo clippy --workspace -- -D warnings
git diff --check origin/main...HEAD
```

Expected: all commands exit `0`.

- [ ] **Step 3: Commit operator documentation**

```bash
git add docs/API.md config.example.toml
git commit -m "docs: clarify proof artifact expiry semantics"
```

- [ ] **Step 4: Remove temporary Superpowers documents**

Use `apply_patch` to delete the two listed files, then commit:

```bash
git add docs/superpowers
git commit -m "chore: remove proof artifact review planning docs"
```

Verify the final base-to-head diff contains no `docs/superpowers` path:

```bash
git diff --name-only origin/main...HEAD | rg '^docs/superpowers/' && exit 1 || true
git status --short
```

- [ ] **Step 5: Review and push**

Run a final whole-branch review against `origin/main...HEAD`. Resolve any Critical or Important
findings test-first, repeat Step 2, then push without force:

```bash
git push origin codex/issue-154-proof-artifact-ttl-cleanup
```

Confirm draft PR #177 points at the new head:

```bash
gh pr view 177 --repo taikoxyz/raiko2 --json url,isDraft,state,headRefOid
```

