# Proof Artifact TTL Cleanup Design

## Status

Approved on 2026-07-14 for [GitHub issue #154](https://github.com/taikoxyz/raiko2/issues/154).

## Problem

Raiko2 stores completed proof artifacts in two independent places:

- an index row in `runtime.sqlite` under `proof_artifacts`; and
- a JSON proof file under `runtime/cache/proofs/...`.

Root runtime tasks already expire according to `runtime.inactive_ttl_secs`, but their completed
proof artifacts deliberately survive so later aggregation, retry, and restart flows can reuse
them. Without a separate retention policy, long-running hosts accumulate artifact rows and files
indefinitely.

## Goals

- Remove expired proof artifact rows in bounded batches.
- Best-effort remove the matching proof files.
- Retain every artifact referenced by an `allocated` or `running` root task.
- Preserve proposal-proof reuse by aggregation and recovery flows.
- Keep artifact retention independent from root-task retention.
- Make automatic artifact cleanup configurable and disable it when its TTL is `0`.
- Emit compact structured cleanup statistics and contextual failure logs.

## Non-goals

- Changing proof artifact key derivation or file layout.
- Removing or changing `POST /v4/prover/invalidate-artifacts`.
- Changing `runtime.inactive_ttl_secs` or root-task cleanup semantics.
- Adding a new public API endpoint or response field.
- Cleaning unindexed files that have no `proof_artifacts` row.

## Configuration

Add `runtime.proof_artifact_ttl_secs: u64` with these semantics:

- Default: `604800` seconds (seven days).
- `0`: disable automatic proof artifact cleanup.
- The setting is independent of `runtime.inactive_ttl_secs`. Either cleanup can run while the
  other is disabled.
- Expiration is evaluated from the artifact row's `updated_at` value. Re-registering an artifact
  refreshes that value through the existing upsert behavior.

Document the setting in `config.example.toml` and the runtime semantics section of `docs/API.md`.

## Architecture and Ownership

### Runtime storage layer

`crates/runtime` owns storage mechanics and remains unaware of Raiko2 server metadata:

- Define a proof artifact cursor containing `updated_at`, `network_pair`, and `proof_ref`.
- Add a bounded query for artifacts at or before the TTL cutoff, ordered by the cursor fields.
- Add conditional row removal that succeeds only when the row still has the `updated_at` value
  observed by the cleanup pass.
- Keep the existing unconditional removal API for manual invalidation.

The bounded query uses the existing artifact timestamp index and the row's composite identity.
The stable ordering prevents duplicate or skipped candidates within a scan cycle.

### Server policy layer

`bin/raiko2` owns retention policy because it understands `TaskMetadata` and proof-reference
derivation:

- Extend `task_metadata` with one helper that returns every artifact reference held by a root.
- Include canonical and legacy proposal refs, canonical and legacy aggregate refs, and persisted
  external aggregate-input refs.
- Pair every proof ref with `metadata.network_pair`; refs from one network pair do not retain an
  artifact from another pair.
- Treat only `allocated` and `running` roots as live references.

The helper becomes the single server-side definition of artifact references used by cleanup and
may replace small duplicate ref-collection logic where doing so is directly useful to this change.
Unrelated refactoring is out of scope.

### Cleanup/submission coordination

Add a process-local asynchronous read/write guard shared by submission and automatic artifact
cleanup:

- Submission and recovery paths take the guard in shared mode before reading reusable artifacts
  and hold it until the corresponding non-terminal runtime root is registered or restored.
- Artifact cleanup takes the guard in exclusive mode across its live-reference snapshot and batch
  deletion.

This ordering gives two safe outcomes when cleanup races with a new aggregate request:

1. Cleanup removes the artifact first, so submission observes a cache miss and uses the existing
   reproving path.
2. Submission registers its root first, so cleanup observes the live reference and retains the
   artifact.

Conditional row deletion provides a second safety check for concurrent artifact upserts that do
not participate in the submission guard, such as proof completion observers.

## Cleanup Flow

The existing maintenance task keeps independent cursors for root-task and artifact cleanup. It is
spawned when at least one TTL is nonzero and runs an immediate pass followed by the configured
queue maintenance interval.

For each artifact pass:

1. Return immediately with empty statistics when `proof_artifact_ttl_secs == 0`.
2. Take the exclusive artifact-cleanup guard.
3. Query at most 64 expired artifact rows using the artifact cursor.
4. Load runtime roots and build a set of `(network_pair, proof_ref)` identities referenced by
   `allocated` or `running` roots.
5. If any non-terminal root metadata cannot be parsed, abort this artifact pass without deleting
   candidates. This fails closed because the unknown metadata could reference any candidate.
6. Retain candidates present in the live-reference set.
7. For every other candidate, conditionally delete its SQLite row using the observed
   `updated_at`.
8. If the row changed or disappeared, retain/skip the candidate without touching its file.
9. After successful row deletion, remove the recorded proof file:
   - success increments `files_removed`;
   - `NotFound` increments `files_missing`;
   - any other error increments `file_delete_failures` and emits a warning.
10. Advance the cursor to the last queried candidate. When a query returns no candidates, reset
    the cursor so retained artifacts can be reconsidered after their roots become terminal.

Deleting the row before the file guarantees that a file error cannot wedge future row cleanup.
It can leave an unindexed file after a filesystem failure; the failure is visible to operators and
automatic cleanup continues with later candidates.

## Errors and Observability

Add an artifact cleanup statistics structure with these fields:

- `scanned`
- `removed_rows`
- `retained_active`
- `retained_changed`
- `files_removed`
- `files_missing`
- `file_delete_failures`
- `record_delete_failures`

Behavior:

- Silent when all values are zero.
- Emit one structured `info` event for a non-idle pass.
- Emit contextual `warn` events for individual row or file failures.
- Continue to later candidates after an individual failure.
- Return an error for candidate-query failure or invalid non-terminal metadata; this aborts only
  artifact cleanup for the tick. Root-task cleanup still runs and logs independently.

The structured fields are the operator-facing cleanup counters requested by the issue. No new
Prometheus metric family is required.

## Testing

### Configuration tests

- `RuntimeConfig::default()` uses `604800` seconds.
- Explicit `proof_artifact_ttl_secs = 0` deserializes and disables only artifact cleanup.

### Runtime storage tests

- Expiration cutoff excludes fresh rows and includes expired rows.
- Cursor ordering handles identical timestamps without duplicates or omissions.
- A zero TTL or zero limit returns no candidates.
- Conditional removal deletes an unchanged row.
- Conditional removal retains a row whose `updated_at` changed after candidate selection.

### Server cleanup tests

- An expired artifact removes both its row and file.
- An expired artifact referenced by a non-terminal proposal root is retained.
- Canonical, legacy, aggregate, and external aggregate-input references are recognized.
- Terminal roots do not retain artifacts.
- A missing file still results in row deletion and `files_missing` incrementing.
- A filesystem deletion failure still results in row deletion and increments
  `file_delete_failures`.
- Invalid non-terminal metadata aborts the pass without deleting artifacts.
- A zero artifact TTL leaves artifacts untouched while root-task cleanup can still run.
- The shared submission guard prevents cleanup from deleting an artifact between cache lookup and
  root registration.

Existing manual invalidation tests remain unchanged and continue to prove that the endpoint is
available.

## Verification

Run the smallest focused checks first, then the cross-crate checks required by repository policy:

```bash
cargo fmt --all
cargo test -p raiko2-runtime
cargo test -p raiko2 task_cleanup
cargo test -p raiko2 config
cargo clippy --workspace -- -D warnings
cargo test -p raiko2-queue -p raiko2-runtime
```

If the focused `raiko2` filters do not cover all affected server tests, run `cargo test -p raiko2`
before opening the pull request.

## Acceptance Criteria Mapping

- Configurable retention and `0` disabling: `RuntimeConfig`, config example, and API docs.
- Expired row/file removal: bounded runtime query plus server cleanup pass.
- Active-reference retention: centralized metadata ref derivation and exclusive cleanup snapshot.
- Aggregate-flow safety: submission read guard, non-terminal reference retention, and conditional
  deletion.
- Missing-file cleanup: row-first deletion and explicit `NotFound` handling.
- Compact operator counters: one structured statistics event per non-idle pass.
- Required tests: configuration, storage, cleanup, reference, missing-file, disablement, and race
  coverage described above.
