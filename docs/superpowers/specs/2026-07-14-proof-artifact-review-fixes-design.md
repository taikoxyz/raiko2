# Proof Artifact Review Fixes Design

## Goal

Close the correctness, restart-retention, cleanup-performance, observability, orphan-file, and
invalidation-race gaps found during review of the proof artifact TTL implementation without changing
the configured seven-day default or the independent `0` opt-outs.

## Constraints

- Keep `runtime.inactive_ttl_secs` and `runtime.proof_artifact_ttl_secs` independent.
- Preserve bounded cleanup work: at most 64 expired artifact candidates and 64 pending file deletions
  per maintenance tick.
- Keep artifact cleanup fail-closed when any allocated or running task has invalid metadata.
- Do not reuse a live-reference snapshot across maintenance ticks because submissions may add
  references between ticks.
- Keep the runtime root row after its proof artifact expires when root cleanup is disabled.
- Keep `docs/superpowers` out of the final pull-request diff after implementation is complete.

## Chosen Architecture

### Expired root proof API behavior

`runtime_tasks.proof_path` and `proof_artifacts.proof_path` intentionally identify the same persisted
proof file. Artifact expiry may therefore leave a retained completed root pointing at a missing file.
When `load_persisted_root_proof_material` receives `ErrorKind::NotFound`, it will return an
`ApiError::not_found` with an explicit `proof artifact expired or unavailable` message. Other file
read errors and invalid JSON remain internal errors. This preserves the root lifecycle record while
preventing an expected retention event from surfacing as HTTP 500.

### Publication age and restart restoration

The existing `proof_artifacts.updated_at` column remains the artifact publication-age clock and the
conditional-delete version. Normal publication uses `upsert_proof_artifact` and assigns wall-clock
`now` directly on conflict. It no longer forces `previous + 1`, so a future-skewed value can
self-heal.

Startup restoration uses a separate `restore_proof_artifact` operation. It updates route/path
metadata on conflict but preserves the existing `updated_at`. When recreating a missing artifact
row, it inserts the owning runtime record's `updated_at`, which approximates the original proof
completion time without refreshing it on every restart. Startup completes before cleanup is spawned,
so restoration does not require the artifact guard.

All production-time publication paths remain behind the cleanup guard. Because an expired row's old
timestamp cannot equal a new publication's current timestamp for a positive TTL, direct assignment
still makes the conditional delete reject a republished artifact.

### Cleanup query and lock scope

`RuntimeManager` gains a query that loads only `allocated` and `running` runtime rows. Live artifact
reference derivation uses that query instead of deserializing every terminal row.

The artifact expiry index becomes `(updated_at, network_pair, proof_ref)`. Cursor pagination uses the
SQLite row-value predicate
`(updated_at, network_pair, proof_ref) > (?after_updated_at, ?after_pair, ?after_ref)` so the same
index satisfies both the range and ordering without a temporary sorter.

Expired candidate selection occurs before acquiring the cleanup write guard. After acquiring the
guard, cleanup scans current active roots, then conditionally deletes the previously selected rows.
Any publication or submission that completes between selection and lock acquisition is visible to
the active scan and/or changes `updated_at`, so the retention and conditional-delete checks remain
safe. Live references are recomputed for each page; they are not hoisted across ticks.

### Invalid active metadata

The active-row scan returns both valid references and a count of metadata parse failures. If the
count is nonzero, the cleanup pass advances no cursor and deletes no artifact. It returns structured
stats with `invalid_metadata_records` and `retained_invalid_metadata` instead of discarding the count
inside a generic error. A Prometheus counter records invalid active metadata observations.

The failure remains global by design: without successfully parsing a live row, cleanup cannot know
which artifacts it references. Skipping only that row would violate fail-closed retention.

### Retryable file deletion tombstones

The runtime schema gains `proof_artifact_file_deletions`:

```sql
CREATE TABLE proof_artifact_file_deletions (
    proof_path TEXT PRIMARY KEY,
    queued_at INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT
);
```

Conditional artifact removal becomes one SQLite transaction: delete the matching visible artifact
row and enqueue its path in the deletion table. The artifact is therefore cache-invisible before the
filesystem mutation, while a crash cannot lose the path that still needs deletion.

Cleanup immediately attempts the file removal. Success and `NotFound` remove the tombstone. Other
errors increment `attempts` and store a bounded error string. Each later maintenance tick retries up
to 64 tombstones. Before deleting a tombstoned path, cleanup checks whether a current artifact row has
re-registered the same path; if so, it drops the obsolete tombstone without unlinking the newly
published file.

V4 invalidation uses the same row-first queue-and-delete mechanism rather than its current file-first
sequence.

### Invalidation serialization

Non-dry-run `POST /v4/prover/invalidate-artifacts` acquires the artifact cleanup guard's write side
around candidate collection, task removal, artifact row queuing, and file removal. This serializes the
destructive operation against cleanup, submissions, restoration/recovery, and proof publication.
Dry-run invalidation remains read-only and does not take the write guard.

## Testing

Tests will be added test-first and observed failing before production changes:

- a retained completed root whose proof file is missing returns a 404 error rather than 500;
- restart restoration preserves an existing artifact timestamp and uses the runtime timestamp for a
  missing row;
- normal publication replaces a future-skewed timestamp instead of advancing it;
- active-task listing excludes terminal rows;
- the composite cursor query returns stable pages and its migration creates the expected index;
- invalid live metadata reports counters, retains the batch, and leaves the cursor unchanged;
- a failed file deletion leaves a retry tombstone, a later pass removes it, and a re-published path is
  never unlinked by an old tombstone;
- V4 invalidation waits for the write guard and queues row-first file deletion.

Focused tests will be followed by formatting, runtime tests, all `raiko2` tests, queue/runtime lanes,
and workspace Clippy with `RISC0_SKIP_BUILD_KERNELS=1` where the local Apple toolchain requires it.

## Documentation

`docs/API.md` and `config.example.toml` will state that artifact age is based on publication rather
than restart, retained expired roots return not-found for unavailable proof material, malformed active
metadata pauses cleanup fail-closed, and failed file removals are retried from durable tombstones.

