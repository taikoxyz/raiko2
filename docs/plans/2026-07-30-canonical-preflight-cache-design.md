# Canonical Preflight Cache and Scoped Startup Cleanup Design

## Status

Approved design for implementation. This document defines the cache identity, trust boundary,
storage lifecycle, startup cleanup behavior, and verification requirements. It does not change the
guest input wire format, guest ELF files, proof formats, public inputs, or on-chain verification.

## Problem

Every enabled Shasta proof lane currently owns a separate `ShastaSpec` and independently performs
the same expensive work for a proposal:

- discover and authenticate the L1 proposal event,
- fetch L2 blocks and derivation sources,
- derive transaction lists,
- fetch execution witnesses,
- hydrate L1 ancestors and checkpoint witness data, and
- compact the result into a `GuestInput`.

As a result, requesting SGX, SGXGETH, SP1, RISC0, native, or Boundless proofs for the same proposal
can repeat the same provider and witness work. Persisted artifacts do not currently provide a
canonical, proof-lane-independent preflight cache.

The current `runtime.reset_namespace_on_start` option is also too broad. It deletes runtime state,
proof manifests, and every remaining object in the namespace. A guest upgrade needs stale proofs
removed, but normally does not invalidate canonical preflight data. Full namespace deletion is both
unnecessarily slow and unable to express this distinction.

## Goals

1. Build a canonical preflight result once and share it across all proof lanes.
2. Keep persisted preflight data independent of proof type, verifier address, SGX identity, ZK image
   identity, prover address, and graffiti.
3. Materialize a complete `GuestInput` only for the current request and discard it after proving.
4. Preserve the existing guest input and proof formats.
5. Add exact startup cleanup scopes for active proof artifacts and canonical preflight artifacts.
6. Let GCS lifecycle rules reclaim immutable, unreachable content.
7. Make cache corruption or stale data a liveness event, never a new trust root.
8. Avoid distributed coordination under the repository's single-process-per-namespace deployment
   model.

## Non-Goals

- Persisting encoded `GuestInput` objects.
- Sharing mutable runtime ownership across namespaces or active processes.
- Adding a distributed lease, owner epoch, or cross-process single-flight protocol.
- Changing blob proving strategy selection. `ProofOfEquivalence` is currently the only accepted blob
  proof type and is not a cache dimension.
- Changing the `/v4/prover/invalidate-artifacts` request contract in the first implementation.
- Physically deleting all immutable content during normal startup cleanup.
- Rebuilding guest ELF files solely for this host-side cache and cleanup change.

## Terminology

**Canonical preflight core**
: The proof-lane-independent, fetched and derived data needed to materialize a Shasta
  `GuestInput`. It excludes request presentation fields, proof carry data, and host-only chain
  configuration.

**Materialized guest input**
: A transient `GuestInput` created from a canonical core plus the current trusted host chain spec and
  request or lane fields.

**Active manifest**
: The mutable, generation-protected object that makes an immutable content object discoverable.
  Deleting the active manifest invalidates the cache entry immediately.

**Locator**
: The lightweight phase that resolves and authenticates the proposal's canonical L1 inclusion
  identity before the persistent cache key can be computed.

## Design Summary

The proving path becomes:

```text
request
  -> validate request and resolve current host chain specs
  -> resolve canonical proposal locator
  -> compute CanonicalPreflightKeyV1
  -> shared local single-flight
      -> load and validate cached CanonicalShastaPreflightV1
      -> or build, validate, and publish it
  -> materialize GuestInput with current request and lane values
  -> rebuild ProofCarryData from the trusted host chain spec
  -> run lane-specific validation and optional live RPC checkpoint check
  -> encode and prove
  -> drop GuestInput
```

The canonical core is shared by every lane. There is no persisted `INPUT` artifact class:

```text
CanonicalShastaPreflightV1
  + current trusted host chain spec
  + current request prover, graffiti, checkpoint, and last anchor
  + current proof type and verifier
  = transient GuestInput
```

## Canonical Core

Introduce a strongly typed, versioned representation such as
`CanonicalShastaPreflightV1`. It must not be represented as a normal `GuestInput` with zeroed or
placeholder carry data, because that makes it too easy for proof-specific fields to leak into the
cache contract.

The core contains:

- proposal ID and explicit L2 block range,
- canonical L1 inclusion header and decoded proposal event,
- authenticated L1 ancestor headers,
- canonical derivation data sources and manifest block data,
- fetched L2 blocks,
- execution witnesses and account snapshots,
- compact proposal ancestor headers,
- compact proposal state-node pool, and
- the canonical source spans or equivalent data needed by materialization and validation.

The core excludes:

- `ProofCarryData`,
- proof type,
- verifier address forks,
- SGX instance ID, MRENCLAVE, MRSIGNER, and provider endpoint,
- SP1 or RISC0 ELF, image, or verification IDs,
- `actual_prover` and graffiti,
- RPC and beacon endpoint URLs,
- display names and other host presentation configuration, and
- a host `ChainSpec` copy inside each cached `StatelessInput`.

The last point is important. Current `StatelessInput` values contain a full `chain_spec`. The cache
representation must either use a dedicated witness type without that field or strip it during
encoding and reinsert the current trusted host-resolved chain spec during materialization. Cached
verifier maps or RPC values must never be copied into a new guest input.

`actual_prover` and graffiti do not affect witness generation, so they are injected during
materialization. `checkpoint` and `last_anchor_block_number` affect the authenticated execution
boundary; they are also injected during materialization but remain part of the cache key.

## Cache Identity

Use a dedicated `CanonicalPreflightKeyV1`. Serialize it deterministically and hash the serialization
with SHA-256. Do not use Rust's default `Hash` implementation or a release/package version.

The key includes:

- cache schema version,
- canonical network-pair identity defined by the L1 and L2 chain IDs,
- proposal ID,
- explicit L2 block range,
- L1 inclusion block number,
- last anchor block number,
- optional checkpoint value,
- canonical L1 inclusion header hash,
- a digest of the normalized proposal event, and
- a fingerprint of preflight-effective chain rules.

The chain-rules fingerprint contains only semantics that can change fetched or derived preflight
data:

- fork schedule and effective maximum spec,
- EIP-1559 parameters used by execution or derivation,
- L1 contract fork schedule used for event lookup,
- L2 anchor and checkpoint predeploy derivation when applicable,
- genesis and slot timing used by derivation,
- chain IDs, and
- the Taiko-chain classification.

The fingerprint excludes:

- verifier address forks,
- RPC and beacon URLs,
- chain display names,
- proof backend configuration, and
- guest or provider identities.

Tests must make this boundary explicit: a verifier-only, endpoint-only, or name-only change preserves
the key; a fork, derivation contract, chain ID, timing, checkpoint, or last-anchor change does not.
Configured pair names are bounded telemetry and log labels only; they are not persistent cache
identity.

The stored envelope contains the complete unhashed key, schema version, content hash, and payload.
Every read verifies both the content hash and exact key equality before decoding the core.

## Canonical Locator and L1 Reorganizations

The persistent key cannot be computed safely from proposal ID alone because the L1 proposal
inclusion can be reorganized. Before consulting GCS, the host performs a lightweight locator phase:

1. validate the explicit L2 block range and request metadata,
2. fetch the first proposal L2 block needed for event discovery,
3. fetch the L1 inclusion header and proposal event,
4. validate their proposal and chain linkage, and
5. compute the inclusion header hash and normalized event digest.

This phase is lane independent and has its own local single-flight keyed by the request locator
fields. It may repeat cheap L1 discovery after process restart, but it avoids repeating L2 block,
blob, transaction-list, and witness work. A changed inclusion hash or event digest after a reorg
produces a different canonical preflight key.

## Materialization and Trust Boundary

Materialization constructs a normal `GuestInput` from the canonical core and current request state:

1. insert the current host-resolved L2 chain spec into every transient `StatelessInput`,
2. inject `actual_prover`, graffiti, checkpoint, and last-anchor values,
3. resolve the current lane's proof type,
4. construct `ProofCarryData` using `build_proof_carry_data_with_chain_spec`, and
5. run the existing carry and request consistency checks.

The preflight cache must never call `build_proof_carry_data_from_witness_spec`. That helper resolves a
verifier from witness-embedded input and is suitable only for explicitly trusted local tooling.
Production materialization always uses the current host-resolved trusted chain spec.

The optional `preflight.verify_checkpoint_l2_rpcs` live cross-check still runs after materialization
on both cache hits and misses. It is deliberately not satisfied by cached results.

## Validation and Publication

GCS is an optimization, not a trust root. A cache hit receives the same proof-independent validation
as a freshly built core.

Split validation into two layers:

1. **Canonical validation** authenticates manifest linkage, L1 and L2 headers, witness completeness,
   stateless execution, checkpoints, and all proof-independent invariants.
2. **Lane validation** checks the materialized proof carry data, verifier selection, proof type, and
   other request-specific invariants.

The existing full validation entrypoint can remain as a composition of both layers.

On a cache miss, publication occurs only after canonical validation:

```text
build core
  -> materialize validation view
  -> canonical validation
  -> publish immutable content and active manifest
  -> materialize lane GuestInput
  -> lane validation
  -> prove
```

This prevents one faulty provider response from poisoning all lanes until TTL expiry. On a hit,
decode, key, hash, or canonical-validation failure invalidates the active preflight manifest with
generation CAS and rebuilds the core. Corrupt data is never accepted.

## Test and Fixture Overrides

`ShastaManifestBuilder` supports internal overrides such as proposal events, L1 headers, data
sources, manifest payloads, blob payloads, offsets, and manifest-validation flags through
`ProofContext.config`.

If any such override is present, persistent preflight caching is bypassed. These paths are intended
for fixtures and development. Including arbitrary override payloads in a production cache key would
make the trust boundary and cache identity difficult to audit.

## Shared Local Single-Flight

All lane-specific `ShastaSpec` instances for one network pair receive the same
`PreflightCoordinator`. The coordinator is created by server wiring, not independently by each lane.

Required behavior:

- one locator leader per locator key,
- one core build leader per canonical key,
- concurrent requests with the same key become followers and await the leader result,
- leader cancellation removes the in-flight entry and allows a follower to retry,
- failures are not negative-cached,
- completed in-flight entries are removed,
- the coordinator does not retain large cores indefinitely, and
- the canonical L1/L2 chain-ID pair is always part of identity.

Only preflight work is shared. Each request keeps its own runtime proof task, route, cancellation,
result, and proof publication lifecycle. The in-flight entry is not a persisted task row and is not a
proof cache entry. Requests with different block ranges, checkpoints, last anchors, chain-rule
fingerprints, or canonical L1 inclusion identities do not wait on one another.

No distributed lease is added. The repository requires a single live process per
`(runtime.environment, runtime.namespace)`, so deployment sequencing supplies the process ownership
boundary. GCS generation checks protect exact object versions and cleanup races.

Large witness payloads can create memory spikes when several lanes materialize at once. The
implementation must measure serialized core size and waiter count, avoid permanent duplicate
retention, and continue to respect existing proving concurrency limits.

## Persistent Layout

Use the existing runtime environment and namespace scope with a separate preflight prefix:

```text
<scope>/preflights/v1/<key-hash>/manifest.manifest.json
<scope>/preflights/v1/<key-hash>/content/<content-sha256>.preflight.bincode
```

The manifest contains the full key, content hash, schema version, and content-object identity. It is
written and deleted with GCS generation preconditions. The content object is immutable and may be
written with create-if-absent semantics.

Use deterministic bincode-compatible serialization for the first version. Compression is a measured
follow-up, not a design assumption: witness data may compress well, but compression level, CPU cost,
and peak allocation must be benchmarked before adopting zstd or another codec.

Schema `v1` changes only when canonical core serialization or semantics change. It does not change
for every host release, verifier redeployment, SGX image, or ZK guest build; otherwise the cache
cannot survive the upgrades it is designed to tolerate.

## Cache Failure Semantics

The proof path follows these rules:

- cache miss: build normally;
- transient cache read failure: emit a bounded warning and metric, then build without the cache;
- cache write failure: continue proving from the validated in-memory core;
- decode, key, hash, or validation failure: never use the entry; CAS-invalidate and rebuild;
- build failure: return the original preflight error; do not negative-cache it; and
- conflicting valid create: load and validate the winning manifest.

Proof publication can still fail independently if the configured runtime store is unavailable. The
preflight cache does not hide broader persistent-store outages.

## Startup Cleanup Scopes

Replace the boolean `runtime.reset_namespace_on_start` with an explicit scope list:

```toml
[runtime]
startup_cleanup = ["proof"]
```

For derivation or witness-rule upgrades:

```toml
[runtime]
startup_cleanup = ["proof", "preflight"]
```

The external representation is a list so invalid combinations and unknown names fail during config
parsing. Internally, use a bitmask with:

- `PROOF`,
- `PREFLIGHT`, and
- `ALL = PROOF | PREFLIGHT`.

There is no `INPUT` bit because materialized guest inputs are not persisted. Scopes are exact and do
not imply one another.

### `PROOF`

At startup:

1. reset authoritative persisted runtime task state to empty,
2. delete active proposal and aggregate proof manifests for every lane, and
3. leave proof content objects and invalidation records unreachable for lifecycle TTL.

Runtime state is removed first so no completed task can return a stale proof after its manifest is
deleted.

### `PREFLIGHT`

Delete only active canonical preflight manifests. Preserve runtime task state and all proof
manifests. Immutable preflight content remains unreachable for lifecycle TTL.

### `ALL`

Apply `PROOF` and `PREFLIGHT` in the safe order above. This is still a logical cache invalidation,
not a physical namespace wipe.

Startup cleanup runs before runtime initialization, listeners, or workers. Any cleanup failure aborts
startup. Retrying the same cleanup is idempotent. The deployment invariant requires the previous
process to be fully stopped before a replacement starts.

The old emergency physical namespace reset may remain as an internal/manual recovery operation, but
it is not exposed by normal startup configuration.

## Cleanup Performance

The existing full reset deletes work state, proof manifests, and then every remaining object with a
concurrency of 16. Scoped cleanup avoids the expensive final content pass entirely.

The first implementation should use bounded parallel manifest deletion with an internal concurrency
of 64, generation preconditions, and GCS page iteration. This value is not initially operator
configurable. Metrics will show whether it needs adjustment. The dominant speedup should come from
deleting only small active manifests rather than from increasing concurrency.

GCS lifecycle TTL remains responsible for immutable proof and preflight content. Operators must
configure and monitor that policy independently.

## Existing Invalidation API

`POST /v4/prover/invalidate-artifacts` remains request-scoped and proof-only in the first
implementation. Startup cleanup solves release-wide invalidation; the HTTP API solves targeted
runtime repair. They share storage helpers where practical but do not gain ambiguous combined
semantics.

## Metrics

Add bounded-cardinality Prometheus metrics:

- `raiko2_preflight_cache_requests_total{pair,result}` where result is
  `hit|miss|bypass|error`,
- preflight cache load, build, and canonical-validation duration histograms,
- serialized canonical core size histogram,
- single-flight leader and waiter counters,
- current single-flight waiter gauge,
- startup cleanup matched, removed, and failed object counters by scope, and
- startup cleanup duration histogram by scope.

Do not label metrics with proposal ID, block number, key hash, verifier address, or other
high-cardinality request values. Those details remain in structured logs.

## Required Tests

### Cache identity and materialization

- Concurrent SGX, SGXGETH, SP1, RISC0, and native requests for one proposal perform provider/witness
  construction once.
- A verifier-only change reuses the core and materializes the new verifier.
- An `actual_prover` or graffiti change reuses the core and changes the transient input.
- A checkpoint or last-anchor change misses the cache.
- An L1 inclusion header or proposal-event digest change misses the cache.
- A fork, derivation contract, chain ID, or timing change misses the cache.
- An RPC URL, verifier map, or display-name change hits the cache.
- Hit and fresh-build paths produce byte-equivalent `GuestInput` values for the same current request
  and lane.
- Materialization always uses the current trusted chain spec carry builder.
- Internal manifest overrides bypass persistent caching.

### Validation and repair

- A corrupt content hash, wrong full key, decode failure, or canonical-validation failure is rejected,
  CAS-invalidated, and rebuilt.
- A bad provider result is not published before canonical validation.
- Cache hits still run canonical validation and the optional live checkpoint RPC check.
- Errors are not negative-cached.
- Leader cancellation allows a follower to complete.

### Cleanup

- `PROOF` removes runtime state and all active proposal/aggregate proof manifests while preserving
  preflight manifests.
- `PREFLIGHT` removes active preflight manifests while preserving runtime state and proof manifests.
- `ALL` applies both scopes.
- Immutable content remains after logical cleanup for lifecycle TTL.
- A sibling environment or namespace is untouched.
- Partial cleanup failure aborts startup, and retry succeeds idempotently.
- Memory and GCS stores implement identical scope semantics.
- Unknown or removed startup config fields are rejected.

## Rollout

1. Land the storage types, cache identity, and memory/GCS parity tests without enabling cache reads.
2. Add canonical build/materialization separation and proof-independent validation.
3. Inject one coordinator into every lane and enable read-through caching with metrics.
4. Replace `reset_namespace_on_start` with `startup_cleanup` and update operator documentation.
5. Exercise one proposal concurrently through all configured lanes and verify a single preflight
   build.
6. Validate `["proof"]` and `["proof", "preflight"]` against a non-production namespace.
7. Enable in a canary namespace and compare preflight latency, GCS request volume, serialized size,
   memory peaks, and proof results before wider rollout.

This is a breaking configuration migration only for operators currently setting
`reset_namespace_on_start`. Serde must reject the old field rather than silently ignoring it.

## Decisions

- Cache canonical fetched/derived data, not a placeholder `GuestInput`.
- Persist no standalone input artifact.
- Share one cache across proof lanes.
- Keep proof lane, verifier, guest identity, prover, and graffiti out of the cache key; include the
  blob proof scheme because it changes canonical manifest semantics.
- Keep checkpoint and last anchor in the key.
- Bind the key to canonical L1 inclusion identity to survive reorgs safely.
- Validate every cache hit and publish only after canonical validation.
- A leader failure is returned to every waiter in that flight. The failed flight is removed, so a
  later request elects a new leader and retries; leader cancellation instead lets an existing
  waiter re-elect immediately.
- `runtime.preflight_cache = "off"` bypasses persistent cache access and local singleflight as an
  incident-response control.
- Use host-trusted chain spec materialization and carry construction.
- Use local single-flight only.
- Delete active manifests at startup and let lifecycle TTL remove immutable content.
- Use exact `PROOF | PREFLIGHT` startup scopes and no `INPUT` scope.

## Rejected Alternatives

**Persist a complete `GuestInput` per lane**
: Repeats large data, couples cache identity to verifier and proof type, and preserves stale host
  configuration.

**Cache one `GuestInput` with zero carry data**
: Makes the proof-independent boundary implicit and easy to violate as fields evolve.

**Key only by proposal ID**
: Collides across network pairs, request boundaries, rule changes, and L1 reorganizations.

**Include every chain-spec field or package version in the key**
: Invalidates reusable preflight data for verifier, endpoint, name, and unrelated release changes.

**Delete every GCS object on startup**
: Is slow, conflates proof and preflight validity, and duplicates lifecycle management.

**Add a distributed cache-build lease**
: Conflicts with the repository's single-active-process namespace model and adds failure modes without
  a supported deployment need.
