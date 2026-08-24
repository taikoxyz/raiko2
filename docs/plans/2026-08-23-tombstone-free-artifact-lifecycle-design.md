# Tombstone-Free Artifact Lifecycle Design

## Status

Proposed.

## Context

Raiko2 stores authoritative runtime metadata and immutable proof payloads in GCS. Proof and canonical
preflight objects currently use version-specific tombstones when a manifest is invalidated. Those
markers were introduced as a durable fence against stale publication, but they accumulate until the
bucket lifecycle removes them and duplicate authority already available elsewhere:

- proof deletion intent is persisted in the runtime snapshot as
  `ProofArtifactLifecycle::Invalidated`;
- canonical preflight requests are serialized by a process-local per-key single-flight coordinator;
- GCS manifest creation and deletion already use object-generation preconditions;
- the supported deployment model allows only one live process per
  `(runtime.environment, runtime.namespace)`.

The online `POST /v4/prover/invalidate-artifacts` endpoint has no known production caller and is not
required by the supported operational lifecycle. Active work is handled by `POST /v4/prover/clear`,
guest upgrades are handled by scoped startup cleanup, and terminal artifacts are reclaimed by
retention. Removing it deliberately gives up online range- or prefix-selective terminal-artifact
cleanup; an exceptional bad-proof cleanup uses scoped startup proof cleanup plus a `Recreate`
restart.

This design removes proof and preflight tombstones, removes the online artifact-invalidation API,
and makes same-artifact ordering explicit with true keyed lifecycle locks.

## Terminology And Storage Layout

GCS is the storage backend. A manifest and its content are separate objects in the same bucket.

Proof objects have the conceptual layout:

```text
<scope>/artifacts/<artifact-key>/manifest.manifest.json
<scope>/artifacts/<artifact-key>/content/<content-hash>.proof.json
```

Canonical preflight objects have the conceptual layout:

```text
<scope>/preflights/v<cache-compatibility-version>/<key-digest>/manifest.manifest.json
<scope>/preflights/v<cache-compatibility-version>/<key-digest>/content/<content-hash>.preflight.bincode
```

The content object contains the immutable payload. The small manifest is the visibility point and
contains the content hash and content object name. Writers upload content first, then create the
manifest with a generation precondition. A crash between those operations may leave unreachable
content, but no reader can adopt it without a valid manifest. Bucket lifecycle rules reclaim such
orphan content.

The authoritative runtime snapshot is a separate object:

```text
<scope>/work/runtime-state.runtime.json
```

It contains task records, proof artifact lifecycle records, and pending publication intents. Runtime
snapshot mutations remain globally serialized and generation-CAS protected.

## Deployment Assumption

One live process per `(runtime.environment, runtime.namespace)` is a hard prerequisite. Deployment
configuration must use one replica and a non-overlapping replacement strategy such as `Recreate`.
Canary and production processes must use different namespaces.

This design does not add distributed leases, owner epochs, or cross-process lifecycle locks. GCS
generation CAS remains a final stale-write safeguard, but concurrent writers in one namespace are
unsupported and must not be treated as a normal operating mode.

`Recreate` prevents two application processes from remaining live together, but it cannot revoke a
GCS request that the old process already submitted. Canonical preflight compatibility therefore does
not rely on startup cleanup observing every old request. Incompatible implementations use different
cache compatibility versions, as specified below.

## Decisions

### Remove all tombstones

Proof and canonical preflight stores will stop creating and reading
`invalidated/*.tombstone` objects. Exact manifest deletion becomes the only external invalidation
operation. Existing tombstones are ignored by the new runtime and may be removed after the rollback
window.

Tombstones are not replaced by another marker format.

### Treat the canonical preflight version as a compatibility boundary

The existing canonical preflight `v1` value is the complete cache compatibility version, not only a
serialization schema. Keep one version number rather than adding a second process or deployment
epoch. The version participates in the canonical key digest and the GCS object prefix.

Bump the version whenever an old cached value may be incompatible with the current implementation,
including changes to:

- canonical host derivation or manifest selection;
- witness generation or normalization;
- transaction-list truncation or canonicalization;
- fork interpretation not already represented by `chain_rules_fingerprint`;
- serialized canonical preflight or manifest formats.

`chain_rules_fingerprint` continues to separate chain configuration changes. The compatibility
version separates host-code semantics that cannot be derived from chain configuration.

A delayed write from an incompatible old version lands under the old version prefix and is therefore
unreachable by the new implementation. A delayed write from an earlier process running the same
version is intentionally allowed: the complete canonical key, chain-rules fingerprint, construction
semantics, and validation rules are identical, so the object remains a valid cache candidate.

Consequently, `startup_cleanup = ["preflight"]` is best-effort cache eviction, not a cross-process
guarantee that the prefix remains empty after the operation returns. Requiring that stronger property
would need a per-process epoch or a distributed drain protocol, would eliminate cross-restart cache
reuse, and is outside this design.

Rollback to an older binary reads only that binary's compatibility version. During the rollback
window, retain manifests under the old version prefix. After that window closes and the prefix is
frozen, an operator lists only `manifest.manifest.json` objects under the old
`preflights/vN/` prefix and deletes the observed generations conditionally. Active/current-version
preflight manifests must never receive an age-based lifecycle rule. Immutable content made
unreachable by old-manifest removal is reclaimed by a bucket lifecycle rule configured before
rollout for the exact `.preflight.bincode` suffix; read correctness never depends on deleting
old-version objects first.

### Keep proof invalidation authority in runtime state

`ProofArtifactLifecycle::Invalidated` remains a durable intermediate state. It means:

- the exact descriptor is no longer readable or publishable through runtime state;
- its GCS manifest must be deleted or confirmed missing;
- the runtime artifact record must remain until external finalization succeeds.

The runtime record, not a GCS marker, is the crash-recovery source of truth.

### Use true keyed lifecycle locks

The existing artifact lifecycle lock maps artifact identities onto 64 mutex shards. Different
artifacts can therefore block each other after a shard collision. Replace this array with a
process-local keyed registry based on the complete artifact identity:

```text
(network_pair, pipeline_key, route, proof_ref)
```

The registry conceptually stores:

```text
HashMap<ProofArtifactKey, Weak<tokio::sync::Mutex<()>>>
```

Lookup, weak-reference upgrade, creation, and dead-entry removal are serialized by a short-lived
registry mutex. A holder or waiter keeps a strong `Arc`, so a second mutex cannot be created for the
same key while the first mutex is still observable.

The existing runtime cleanup loop performs one full registry sweep at the end of each cleanup pass;
no separate timer or task is added. The six-hour terminal TTL controls only when terminal root-task
metadata becomes eligible for retirement. Proof artifacts and pending publications are
ownership-driven: normal artifacts may remain indirectly retained by terminal roots during that
window, while `Invalidated` or unowned artifacts may be reclaimed without waiting six hours.
`runtime.cleanup_interval_secs` controls how often the shared maintenance pass and lock sweep run.
The sweep removes only entries whose weak reference can no longer be upgraded, and it runs even when
a retention lane reports an error.

Registry cleanup must not be coupled directly to runtime artifact-record removal: a holder or waiter
may still exist after the durable record disappears. Cleanup is allowed only when the weak reference
can no longer be upgraded.

The keyed mutex is local ordering, not distributed authority. Hash-map collisions may affect lookup
cost but never cause different keys to share one lifecycle mutex because full-key equality remains
required.

### Keep external deletion outside locks when durable invalidation fences publication

GCS manifest deletion is a network operation and may time out. Retention therefore uses a durable
three-phase transition rather than holding a lifecycle lock through external deletion.

#### Phase 1: logical invalidation

1. Select a bounded artifact batch from an authoritative runtime snapshot.
2. Resolve and acquire the selected keyed locks in deterministic key order.
3. In one runtime-state mutation, recheck exact records and retained ownership.
4. Mark eligible exact descriptors `Invalidated`.
5. Release the keyed locks.

The batch mutation preserves the existing bounded runtime-state write behavior. Locks cover only
same-key admission and one authoritative state write, not GCS object deletion.

#### Phase 2: external finalization

1. Delete each exact GCS manifest by generation and content hash, with bounded concurrency.
2. Do not hold keyed lifecycle locks during these requests.
3. Treat `Removed` and `Missing` as successful finalization.
4. Treat `Stale` as an invariant violation requiring reinspection; never delete the observed newer
   descriptor.
5. On transport failure, retain `Invalidated` runtime state and enqueue the exact descriptor for a
   later retry.

During this phase, a same-key publication may acquire its lifecycle lock, but it observes
`Invalidated` and returns a retryable cleanup-pending error before writing content or a manifest.

Pending-publication cleanup is the narrow exception. A pending intent has no `Invalidated` lifecycle
fence, and a new task may adopt identical content without changing its object generation. Its exact
object deletion therefore holds only that artifact's keyed lock across the delete. Publications for
other keys remain independent and pending cleanup remains bounded by the retention worker concurrency.

#### Phase 3: runtime finalization

1. Resolve and acquire keyed locks for descriptors whose manifests were removed or already missing.
2. In one runtime-state mutation, recheck that each record is still the same `Invalidated`
   descriptor.
3. Remove exact artifact records and unowned pending intents.
4. Release the keyed locks.

Only after this phase may a new publication create generation B for the same artifact key. There is
no supported interval in which generation B is published while generation A cleanup remains active.

An old cleanup candidate can still exist in a process-local retry queue. It must recheck the exact
runtime descriptor after acquiring the key lock. If generation B is current, the stale A candidate is
dropped without touching B.

## Proof Publication

Proof publication continues to checkpoint pending bytes before canonical publication where the
current execution path requires recovery. The canonical publication path acquires the keyed
lifecycle lock and applies these rules:

1. If the exact artifact is `Pending` or `Active` and its descriptor matches, reuse is allowed under
   the existing owner and payload checks.
2. If the artifact is `Invalidated`, do not write GCS. Return a retryable cleanup-pending error.
3. Otherwise publish immutable content, create or read the manifest, validate the canonical payload,
   and register the exact descriptor in runtime state before releasing the lock.
4. If an existing manifest has no matching runtime record or recoverable pending-publication intent,
   persist an `Invalidated` record for that exact descriptor and return a retryable error. Retention
   or startup reconciliation then removes it before reproving.
5. A changed descriptor is never adopted merely because it exists in GCS.

The publication path no longer queries external tombstones before or after local registration.
Cleanup-pending is an internal scheduling outcome: it must keep the publication eligible for retry
and must not turn the proposal or aggregate root into a permanent `Failed` or `Cancelled` result.

## Pending Publication Recovery

A pending-publication intent is durable authority for its private pending object and expected
canonical content hash. Reclamation remains same-key serialized:

- if a matching canonical artifact lifecycle record exists, the artifact lane owns manifest cleanup;
- if no artifact record exists and the canonical descriptor matches the pending content hash, create
  an `Invalidated` runtime artifact record before external deletion;
- exact-delete the private pending object by generation only after its last retained owner is gone;
- keep the intent on any external failure so restart or maintenance can retry.

No tombstone is needed because the runtime snapshot retains the unfinished intent or invalidated
artifact descriptor.

## Canonical Preflight Cache

Preflight build failure and cached-entry validation failure are different cases.

### Build failure

A new preflight build that fails returns the leader error to all current waiters. The single-flight
entry is removed, and no content, manifest, tombstone, or negative-cache record is written. A later
request elects a new leader and recomputes.

### Invalid cached entry

If a previously published entry fails manifest, content-hash, key-digest, decode, or canonical
validation:

1. The per-key single-flight leader refuses to use it.
2. The leader exact-deletes the observed manifest generation without creating a tombstone.
3. On `Removed` or `Missing`, the leader rebuilds and may publish a new manifest.
4. On `Stale`, the leader reloads the current winner once and validates it.
5. On storage failure, the leader may build and return a validated in-memory preflight, but it skips
   cache publication because the unusable manifest may still own the create-only path.
6. Current waiters receive the same validated result or build error.
7. A later request retries cleanup and rebuilding normally; failures are never negative-cached.

The existing preflight single-flight spans cache load, validation, deletion, rebuilding, and
publication for the complete canonical key, so a second process-local keyed mutex is unnecessary.
Cross-process same-namespace preflight writers remain unsupported by the deployment model; GCS
generation CAS prevents stale exact deletion but is not a distributed workflow protocol.

The single-flight key includes the cache compatibility version. Same-version callers still share one
leader result. Old-version and new-version callers cannot contend for or load the same manifest, even
if an old create-if-absent request completes after the new process has started.

## API And Store Cleanup

Remove:

- `POST /v4/prover/invalidate-artifacts` and its request/response wire types;
- route, ACL documentation, endpoint tests, and invalidation-range/prefix helpers used only by that
  endpoint;
- proof-store `is_invalidated` operations;
- proof and preflight invalidation-name builders;
- in-memory invalidation sets;
- GCS marker creation and marker readback;
- `AlreadyInvalidated` results whose only distinction came from tombstone presence.

Removing the endpoint is an explicit operational simplification, not a requirement of removing
tombstones. The accepted incident path for a bad terminal artifact is
`runtime.startup_cleanup = ["proof"]` followed by a non-overlapping restart. Its blast radius is the
configured namespace's entire runtime-state snapshot plus proof manifests: all tasks, artifact
records, pending publications, and Boundless/SP1 provider-request checkpoints are discarded. Existing
remote requests continue, so client resubmission may duplicate paid proving work. Preflight objects
remain untouched unless `preflight` is separately selected.

Retain:

- `POST /v4/prover/clear` for active task cancellation;
- scoped `runtime.startup_cleanup` for upgrade-time proof or preflight cleanup;
- runtime `Invalidated` lifecycle records as unfinished proof-deletion authority;
- descriptor and generation CAS for exact deletion;
- immutable content objects and bucket lifecycle reclamation.

Rename invalidation-oriented store APIs and result types to exact-deletion terminology where doing
so removes ambiguity. The result model must distinguish at least `Removed`, `Missing`, and `Stale`.

## Crash And Retry Matrix

### Proof

| Crash point | Durable state | Recovery |
| --- | --- | --- |
| Before logical invalidation | A remains active or pending | Re-evaluate ownership on the next retention pass |
| After `Invalidated(A)`, before manifest delete | Runtime fences A; manifest A remains | Restart reconciliation exact-deletes A |
| After manifest delete, before runtime finalization | Runtime still fences A; manifest is missing | Restart treats missing as finalized and removes exact runtime record |
| After runtime finalization | No runtime record or manifest A remains | A is fully reclaimed; a later request may publish B |
| During publication before manifest create | Pending intent may remain; content may be orphaned | Recover pending intent or rebuild; lifecycle removes unreachable content |
| After manifest create, before runtime registration | Manifest exists; pending intent or task may remain | Recover exact publication, or persist `Invalidated` and remove the orphan manifest |

### Preflight

| Crash point | Durable state | Recovery |
| --- | --- | --- |
| Build fails | No cache state | Later request recomputes |
| Before invalid cached manifest delete | Invalid manifest A remains | Later request validates, deletes, and recomputes |
| After manifest delete, before rebuild | Cache miss | Later request recomputes |
| After content upload, before manifest create | Unreachable immutable content | Later request recomputes; bucket lifecycle removes orphan content |
| After manifest create | Valid manifest B is visible | Later request validates and reuses B |
| Incompatible old-version create completes after cutover | Manifest remains under the old version prefix | Current readers cannot address it; after the rollback window, generation-aware old-prefix cleanup removes the manifest and content lifecycle reclaims the payload |
| Same-version create completes after restart cleanup | Compatible manifest is visible again | Later request validates and may reuse it |

## Observability

Add or preserve bounded-cardinality metrics for:

- keyed lock wait and hold durations;
- live and dead keyed-lock registry entry counts;
- proof exact-delete outcomes: removed, missing, stale, and failure;
- artifacts currently in `Invalidated` lifecycle;
- cleanup-pending publication rejections;
- proof retention retry queue length;
- preflight invalid-cache, exact-delete, rebuild, and uncached-fallback outcomes;
- startup reconciliation successes as pull metrics and failures as structured startup error logs.

Logs may include task ID, proposal range, proof type, artifact key fields, descriptor generation, and
content hash as structured fields. Metrics must not use those values as labels.

After rollout, tombstone-write metrics or bucket queries should remain at zero. Existing tombstone
count is a migration metric, not runtime authority.

## Tests

### Keyed lifecycle registry

- same artifact key always resolves to one mutex;
- different keys remain independent even when their hashes would have shared the old shard;
- a waiter keeps the keyed mutex alive while another holder exits;
- dead weak entries are reclaimed without allowing two live mutexes for the same key;
- registry cleanup never removes an entry that can still be upgraded.
- the existing runtime cleanup pass invokes registry sweeping even when a retention lane fails;
- repeated maintenance bounds registry entries to live keys plus keys created since the last sweep.

### Proof lifecycle

- publication racing logical invalidation either commits before admission or observes
  `Invalidated`; it never publishes B during A cleanup;
- publication during external deletion returns cleanup-pending without writing GCS;
- cleanup-pending publication does not terminally fail or cancel its proposal or aggregate root;
- delete failure retains the exact invalidated record and retry work;
- crash recovery completes both pre-delete and post-delete invalidated states;
- a stale A candidate cannot delete or remove generation B;
- aggregate input, aggregate output, and proposal proof artifacts obey the same lifecycle;
- pending-publication recovery does not adopt a changed untracked descriptor;
- startup proof cleanup remains generation protected and does not create markers.

### Preflight lifecycle

- failed builds are not negative-cached and a later request rebuilds;
- concurrent callers share one failed or successful leader result;
- invalid cached A is exact-deleted and rebuilt as B without a marker;
- delete failure returns a validated uncached result and leaves A unusable;
- stale deletion reloads and validates the current winner;
- content written without a manifest is not discoverable as a cache hit;
- startup preflight cleanup remains generation protected and does not create markers.
- an incompatible old-version manifest remains unreachable after a delayed create;
- same-version callers continue to share one single-flight result after restart;
- changing only the cache compatibility version changes both the key digest and object prefix.
- old-version cleanup selects only manifests under a frozen old prefix, uses observed generations,
  and cannot delete current-version manifests.

### API

- the removed invalidation route is not registered;
- `clear` retains its existing ACL and active-task behavior;
- API documentation and example configuration contain no obsolete invalidation endpoint.

## Rollout

1. Merge the runtime and API changes with proof/preflight marker reads and writes fully removed.
2. Verify deployment manifests enforce one replica, `Recreate`, and unique namespaces for parallel
   canary and production services.
3. Inspect the target bucket lifecycle policy before enabling shared preflight caching. Remove or
   narrow any generic `.json` age rule that reaches the active `preflights/vN/` manifest prefix, and
   add a finite-retention rule that explicitly matches immutable `.preflight.bincode` content.
4. Confirm that every release containing an incompatible preflight semantic or format change bumps
   the canonical preflight cache compatibility version.
5. Deploy canary, then production, and observe invalidated-record age, exact-delete failures,
   cleanup-pending responses, keyed-lock wait time, and runtime-state CAS failures.
6. Keep historical tombstones and old preflight-version objects during the binary rollback window.
   The new binary ignores old markers and cannot address old-version preflights.
7. After the rollback window, remove existing `invalidated/*.tombstone` objects and manifests under
   each frozen old `preflights/vN/` prefix with scoped, generation-aware operational cleanup. Do not
   add an age-based lifecycle rule to the active/current preflight manifest prefix.
8. Remove tombstone-specific bucket lifecycle rules only after historical markers are gone. Keep
   lifecycle rules for immutable proof and preflight content.

Rollback to a tombstone-reading binary after historical markers are deleted is unsupported. Rollback
before marker cleanup remains possible under the same single-process deployment invariant.

## Safety Invariants

- A failed preflight build never creates durable negative state.
- Incompatible preflight implementations never address the same canonical cache key or object prefix.
- Same-version delayed preflight publication is compatible and remains subject to canonical validation.
- Runtime `Invalidated(A)` is durable before manifest A deletion begins.
- Publication cannot write generation B while the exact key is `Invalidated(A)`.
- Cleanup-pending is retryable execution state, not a terminal proof result.
- Invalidated canonical proof-manifest deletion does not hold a keyed lifecycle lock across GCS I/O.
- Pending-object deletion holds its keyed lifecycle lock so identical-content owner adoption cannot
  race an exact delete.
- Runtime record A is removed only after manifest A is removed or confirmed missing.
- Every destructive object operation is generation protected.
- A stale cleanup observation cannot delete a changed descriptor.
- Same-key publication, pending recovery, invalidation admission, and runtime finalization share one
  keyed lifecycle protocol.
- Different artifact keys never share a lifecycle mutex merely because of a hash collision.
- Lock-registry reclamation cannot create two live mutexes for one key.
- No tombstone is used as publication, deletion, or recovery authority.
- Single-process namespace ownership remains a deployment invariant rather than an application lease.

## Non-Goals

- Supporting active/active writers in one runtime namespace.
- Replacing the runtime-state backend with SQLite or another database.
- Deleting immutable content synchronously with its manifest.
- Rehydrating terminal tasks from old proof objects.
- Changing proof formats, public inputs, guest ELF contents, or verifier behavior.
- Changing bucket lifecycle policy in the application code.
- Providing absolute empty-prefix semantics for startup preflight cleanup across delayed same-version
  GCS requests.
