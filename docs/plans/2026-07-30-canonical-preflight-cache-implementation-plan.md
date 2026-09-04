# Canonical Preflight Cache Implementation Plan

> **Superseded artifact invalidation guidance:** The canonical-cache architecture remains historical
> context, but `POST /v4/prover/invalidate-artifacts` and its range/prefix workflow were removed by
> `docs/plans/2026-08-23-tombstone-free-artifact-lifecycle-design.md`. Current operations use automatic
> retention or an explicitly broad, one-shot startup cleanup with the documented runtime-state blast
> radius.

**Goal:** Share one validated Shasta preflight result across all proof lanes and replace full startup namespace deletion with exact `proof` and `preflight` cleanup scopes.

**Architecture:** Add a versioned, lane-independent canonical preflight core and deterministic key
to the host-only Shasta pipeline; split Shasta preflight into locator, core construction, canonical
validation, and request-specific materialization; provide memory and GCS manifest/content stores
through the runtime; inject one local single-flight coordinator per network pair into every
`ShastaSpec`; and migrate startup cleanup from a boolean full reset to an exact scope list. Cache
types must stay outside the guest dependency closure so this host optimization cannot change guest
ELF provenance.

**Tech Stack:** Rust, Tokio, async traits, serde/bincode, SHA-256, Google Cloud Storage generation preconditions, Prometheus, TOML configuration, and existing Shasta stateless validation.

---

## Preconditions

- Start from current `origin/main` in an isolated worktree.
- Read the design first:
  `docs/plans/2026-07-30-canonical-preflight-cache-design.md`.
- Preserve the existing `GuestInput`, proof, public-input, and HTTP invalidation formats.
- Do not hand-edit `crates/guests/elf`.
- Keep internal manifest override paths cache-bypassed.
- Use Conventional Commits and commit after each completed task.

## Task 1: Define the Canonical Core and Deterministic Key

**Files:**

- Create: `crates/pipeline/src/forks/shasta/preflight_cache/types.rs`
- Modify: `crates/pipeline/src/forks/shasta/preflight_cache.rs`
- Modify: `crates/pipeline/Cargo.toml`
- Test: `crates/pipeline/src/forks/shasta/preflight_cache/types.rs`

### Step 1: Write failing key-boundary tests

Add unit tests for:

- deterministic key bytes and SHA-256 digest,
- verifier map changes preserving the rules fingerprint,
- RPC URL and chain display-name changes preserving the fingerprint,
- fork schedule, L1 contract fork, chain ID, genesis/timing, checkpoint, and last-anchor changes
  changing the key,
- full-key inequality being detected even if a test supplies the same external hash string, and
- canonical core serialization round trips without a `ChainSpec` or `ProofCarryData`.

Run:

```bash
cargo test -p raiko2-pipeline preflight_cache
```

Expected: fail because the canonical types and functions do not exist.

### Step 2: Add the versioned types

Add explicit types shaped along these lines:

```rust
pub const CANONICAL_PREFLIGHT_SCHEMA_V1: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalPreflightKeyV1 {
    pub schema: u16,
    pub l1_chain_id: u64,
    pub l2_chain_id: u64,
    pub proposal_id: u64,
    pub l2_block_range: L2BlockRange,
    pub l1_inclusion_block_number: u64,
    pub last_anchor_block_number: u64,
    pub checkpoint: Option<ShastaCheckpoint>,
    pub l1_inclusion_hash: B256,
    pub proposal_event_digest: B256,
    pub chain_rules_fingerprint: B256,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CanonicalStatelessInputV1 {
    pub block: Block,
    pub witness: ExecutionWitness,
    pub accounts: AddressMap<TrieAccount>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CanonicalShastaPreflightV1 {
    // Use explicit canonical manifest fields. Do not embed request-specific
    // TaikoProverData, ManifestChainSpec.name, ChainSpec, or ProofCarryData.
}
```

Use one canonical serializer function for key hashing. Do not derive cache identity through Rust
`Hash`. Keep schema version independent of crate/package version.

Reuse the repository's canonical block serde adapters rather than relying on a backend crate's
default serde representation.

The canonical manifest representation must store the proposal event, L1 header and ancestors, data
sources, and other proof-independent manifest fields explicitly. It must not store placeholder
request fields in a normal `TaikoManifest`.

### Step 3: Implement the semantic chain-rules fingerprint

Add a single audited function that serializes only preflight-effective rule fields in a fixed order.
Document each included and excluded field beside the code. In particular:

- include chain IDs, fork schedule/effective max spec, derivation L1 contract forks, EIP-1559
  execution inputs, predeploy derivation inputs, genesis/slot timing, and `is_taiko`;
- exclude verifier maps, endpoint URLs, display names, backend settings, and guest/provider IDs.

Avoid serializing the complete `ChainSpec`, because doing so would reintroduce verifier-triggered
cache churn. Treat `(l1_chain_id, l2_chain_id)` as the canonical pair identity; do not put the
configured pair name into the persistent key.

### Step 4: Run focused verification

```bash
cargo fmt --all
cargo test -p raiko2-pipeline preflight_cache
cargo run -p xtask-build-guest --bin xtask-build-guest -- all --check
```

Expected: pass, with both guest backends reported current.

### Step 5: Commit

```bash
git add crates/pipeline
git commit -m "feat(preflight): define canonical cache identity"
```

## Task 2: Split Core Construction, Materialization, and Validation

**Files:**

- Modify: `crates/pipeline/src/forks/shasta/preflight_cache.rs`
- Modify: `crates/pipeline/src/forks/shasta/mod.rs`
- Modify: `crates/pipeline/src/forks/shasta/spec.rs`
- Modify: `crates/pipeline/src/forks/shasta/manifest.rs`
- Test: `crates/pipeline/src/forks/shasta/spec.rs`
- Test: `crates/pipeline/src/forks/shasta/preflight_cache.rs`

### Step 1: Write failing equivalence and trust-boundary tests

Add tests proving:

- a fresh canonical core materializes to the same encoded `GuestInput` as the current uncached path
  for the same lane and request,
- SGX and SGXGETH materialization use the current host-resolved verifier,
- changing verifier maps reuses the same core but changes `ProofCarryData.verifier`,
- changing prover or graffiti reuses the same core but changes the transient manifest,
- checkpoint and last anchor are injected exactly,
- the cached witness representation cannot supply a `ChainSpec`,
- `build_proof_carry_data_from_witness_spec` is not called by production materialization, and
- any internal `ShastaManifestBuilder` override activates cache bypass.

Run:

```bash
cargo test -p raiko2-pipeline shasta::preflight_cache
```

Expected: fail before the split exists.

### Step 2: Extract the locator phase

Move the lane-independent front of `ShastaSpec::preflight` into a function that returns a typed
locator:

```rust
pub struct CanonicalPreflightLocatorV1 {
    pub key: CanonicalPreflightKeyV1,
    pub block_numbers: Vec<u64>,
    pub proposal_event: ShastaEventData,
}
```

The locator must:

- require a valid explicit L2 block range,
- fetch the first L2 proposal block,
- fetch and validate the L1 proposal event and inclusion header,
- compute the normalized event digest,
- compute the chain-rules fingerprint from current host-resolved specs, and
- include checkpoint and last anchor in the key.

Do not include proof type, verifier, prover, graffiti, endpoint URLs, or the sole
`ProofOfEquivalence` blob value.

### Step 3: Extract canonical core construction

Refactor the current block, source, tx-list, witness, L1-header, and checkpoint hydration path to
return `CanonicalShastaPreflightV1`.

Keep one implementation of the provider and witness work. The existing uncached path should call
this same builder, not preserve a second copy.

### Step 4: Add transient materialization

Implement a function with an explicit trusted-spec argument:

```rust
pub fn materialize_guest_input(
    core: &CanonicalShastaPreflightV1,
    ctx: &ProofContext,
    trusted_chain_spec: &ChainSpec,
    proof_type: ProofType,
) -> RaikoResult<GuestInput>;
```

It must:

- clone canonical data into a transient `GuestInput`,
- insert `trusted_chain_spec` into each transient `StatelessInput`,
- build current `TaikoProverData`,
- rebuild carry only through `build_proof_carry_data_with_chain_spec`,
- compact data exactly once, and
- leave the canonical core unchanged.

### Step 5: Split canonical and lane validation

Refactor the existing Shasta validator into:

```rust
pub fn validate_canonical_preflight(...) -> RaikoResult<()>;
pub fn validate_materialized_carry(...) -> RaikoResult<()>;
```

Keep the public full validator as their composition. Canonical validation must include manifest
linkage, L1/L2 header authentication, stateless execution, witness completeness, anchor/checkpoint
rules, and transaction-root checks. Lane validation must cover carry, proof type, and verifier
consistency.

The split must not weaken guest-equivalent validation or create a second execution path.

### Step 6: Run focused verification

```bash
cargo fmt --all
cargo test -p raiko2-pipeline shasta
cargo test -p raiko2-primitives-shasta
```

Expected: pass.

### Step 7: Commit

```bash
git add crates/pipeline crates/primitives-shasta
git commit -m "refactor(preflight): split canonical core materialization"
```

## Task 3: Add the Store Contract and Memory Implementation

**Files:**

- Modify: `crates/pipeline/src/forks/shasta/preflight_cache.rs`
- Modify: `crates/runtime/src/artifact_store.rs`
- Modify: `crates/runtime/src/lib.rs`
- Test: `crates/runtime/src/artifact_store.rs`

### Step 1: Write failing store contract tests

Cover:

- put-if-absent creates one active manifest,
- repeated identical put returns the existing object,
- conflicting content is reported and never overwrites a valid manifest,
- get verifies full key and content hash,
- exact invalidation requires the observed generation,
- immutable content survives manifest invalidation,
- sibling environment/namespace isolation, and
- memory store cleanup scope parity placeholders for Task 5.

Run:

```bash
cargo test -p raiko2-runtime preflight
```

Expected: fail because no preflight store exists.

### Step 2: Define a pipeline-owned cache-store trait

The pipeline crate owns the abstraction so it does not depend on runtime:

```rust
#[async_trait]
pub trait CanonicalPreflightStore: Debug + Send + Sync {
    async fn get(
        &self,
        key: &CanonicalPreflightKeyV1,
    ) -> anyhow::Result<Option<CanonicalPreflightObject>>;

    async fn put_if_absent(
        &self,
        key: &CanonicalPreflightKeyV1,
        bytes: &[u8],
    ) -> anyhow::Result<CanonicalPreflightPutResult>;

    async fn invalidate_exact(
        &self,
        key: &CanonicalPreflightKeyV1,
        descriptor: &CanonicalPreflightDescriptor,
    ) -> anyhow::Result<CanonicalPreflightDeleteResult>;
}
```

Keep object descriptors generation-aware. Return enough information to validate full key, schema,
hash, and bytes.

### Step 3: Extend the runtime store

Implement the trait on `MemoryProofArtifactStore` and require it from `RuntimeStore`. Add a small
runtime adapter/handle that can be cloned into the pipeline without introducing a
pipeline-to-runtime dependency or relying on trait-object upcasting.

Use distinct memory maps for preflight manifests and immutable content. Do not reuse
`ProofArtifactKey`, because proof route and pipeline are intentionally absent from canonical
preflight identity.

### Step 4: Run focused verification

```bash
cargo fmt --all
cargo test -p raiko2-runtime preflight
```

Expected: pass.

### Step 5: Commit

```bash
git add crates/pipeline crates/runtime
git commit -m "feat(runtime): add canonical preflight store"
```

## Task 4: Add GCS Manifest/Content Persistence and Repair

**Files:**

- Modify: `crates/runtime/src/artifact_store/gcs.rs`
- Modify: `crates/runtime/src/artifact_store/gcs_tests.rs`
- Modify: `crates/runtime/src/artifact_store.rs`
- Test: `crates/runtime/src/artifact_store/gcs_tests.rs`

### Step 1: Write failing GCS tests

Using the existing fake transport, cover:

- object names under `preflights/v1/<key-hash>/`,
- immutable content create-if-absent,
- generation-protected manifest creation and deletion,
- valid concurrent-create winner loading,
- full-key mismatch rejection,
- content-hash mismatch rejection,
- malformed envelope rejection,
- CAS invalidation of a corrupt active manifest,
- content retention after invalidation, and
- pagination over more than one GCS page.

Run:

```bash
cargo test -p raiko2-runtime artifact_store::tests
```

Expected: new tests fail.

### Step 2: Implement the GCS envelope and paths

Use:

```text
<scope>/preflights/v1/<key-hash>/manifest.manifest.json
<scope>/preflights/v1/<key-hash>/content/<content-sha256>.bin
```

The manifest contains the full key, schema, content hash, and content object name. Verify all fields
before returning bytes. Keep immutable content creation separate from active manifest publication.

Do not add compression in this task.

### Step 3: Implement corrupt-entry repair

Expose exact manifest invalidation through the store. The caller must invalidate only the generation
it read. A concurrent replacement is preserved. Treat malformed or wrong-key data as unusable even
if deletion fails.

### Step 4: Run focused verification

```bash
cargo fmt --all
cargo test -p raiko2-runtime artifact_store::tests
```

Expected: pass.

### Step 5: Commit

```bash
git add crates/runtime
git commit -m "feat(runtime): persist canonical preflight cache"
```

## Task 5: Replace Full Startup Reset with Exact Cleanup Scopes

**Files:**

- Modify: `bin/raiko2/src/config/runtime.rs`
- Modify: `bin/raiko2/src/config/mod.rs`
- Modify: `bin/raiko2/src/server/state/mod.rs`
- Modify: `crates/runtime/src/artifact_store.rs`
- Modify: `crates/runtime/src/artifact_store/gcs.rs`
- Modify: `crates/runtime/src/artifact_store/gcs_tests.rs`
- Modify: `crates/runtime/src/lib.rs`
- Test: `bin/raiko2/src/config/runtime.rs`
- Test: `bin/raiko2/src/server/state/mod.rs`
- Test: `crates/runtime/src/artifact_store.rs`
- Test: `crates/runtime/src/artifact_store/gcs_tests.rs`

### Step 1: Write failing config tests

Cover:

- missing `startup_cleanup` means no cleanup,
- `["proof"]`, `["preflight"]`, and both parse,
- duplicates are normalized or rejected consistently,
- unknown scope and removed `reset_namespace_on_start` are rejected, and
- serialization uses lowercase external names.

Run:

```bash
cargo test -p raiko2 config::runtime
```

Expected: fail before config migration.

### Step 2: Add external scopes and internal mask

Use a typed enum for serde and a bitmask for runtime operations:

```rust
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StartupCleanupScope {
    Proof,
    Preflight,
}
```

`RuntimeConfig` owns `startup_cleanup: Vec<StartupCleanupScope>`. Remove
`reset_namespace_on_start`; do not keep a compatibility alias that could silently trigger broader
deletion.

### Step 3: Write failing cleanup parity tests

For both memory and fake GCS stores, create runtime state, proposal/aggregate proof manifests,
preflight manifests, immutable content, and sibling namespace objects. Assert:

- `PROOF` resets runtime state first and removes all active proof manifests only,
- `PREFLIGHT` removes active preflight manifests only,
- `ALL` does both,
- immutable content remains,
- sibling scopes remain,
- a partial failure aborts and retry is idempotent, and
- GCS pagination and bounded deletion concurrency work.

### Step 4: Implement scoped cleanup

Replace the three-phase physical reset in the startup path with:

```rust
pub async fn cleanup_before_start(
    &self,
    scopes: StartupCleanupMask,
) -> Result<StartupCleanupReport>;
```

For `PROOF`, clear authoritative runtime state before proof manifests. For `PREFLIGHT`, delete only
preflight manifests. Use generation-protected deletes and an internal concurrency limit of 64.

Keep any physical full-namespace reset as an explicitly manual/internal method only; do not expose it
through normal startup config.

### Step 5: Run focused verification

```bash
cargo fmt --all
cargo test -p raiko2-runtime reset
cargo test -p raiko2-runtime cleanup
cargo test -p raiko2 config::runtime
```

Use `rg` to confirm no production or active operator-documentation references remain.

```bash
rg -n "reset_namespace_on_start" \
  bin crates README.md docs/API.md docs/operations.md config.example.toml
```

Expected: no matches.

### Step 6: Commit

```bash
git add bin/raiko2 crates/runtime
git commit -m "feat(runtime): add scoped startup cleanup"
```

## Task 6: Implement the Two-Level Single-Flight Coordinator

**Files:**

- Modify: `crates/pipeline/src/forks/shasta/preflight_cache.rs`
- Modify: `crates/pipeline/src/forks/shasta/spec.rs`
- Test: `crates/pipeline/src/forks/shasta/preflight_cache.rs`

### Step 1: Write failing concurrency tests

Use a counting fake provider/store and Tokio tasks to prove:

- concurrent equal locator keys execute locator discovery once,
- concurrent equal canonical keys execute blocks/tx-list/witness construction once,
- different checkpoint, last anchor, range, rules fingerprint, or L1 identity does not coalesce,
- each follower receives the same validated core,
- proof task identity and cancellation remain per request,
- leader cancellation removes the entry and a follower becomes leader,
- build errors remove the entry and are not negative-cached,
- completed entries are removed instead of retaining large cores, and
- transient read failure bypasses cache while corrupt data attempts CAS repair.

Run:

```bash
cargo test -p raiko2-pipeline preflight_coordinator
```

Expected: fail.

### Step 2: Implement coordinator ownership

Add a cloneable `PreflightCoordinator` holding:

- an `Arc<dyn CanonicalPreflightStore>`,
- a locator in-flight map,
- a canonical build in-flight map, and
- an observer interface for bounded telemetry.

The in-flight value must have explicit completion and drop/cancellation behavior. Do not use a
leader-owned oneshot whose drop leaves followers waiting forever. Remove map entries only if they
still refer to the same in-flight generation.

The coordinator shares only preflight work. It must not create, mutate, or complete runtime proof
tasks.

### Step 3: Implement read-through flow

For a canonical key:

1. try cache load,
2. verify envelope, full key, hash, decode, and canonical validation,
3. return hit when valid,
4. exact-invalidate and rebuild when corrupt,
5. build and canonical-validate on miss,
6. publish after validation,
7. tolerate cache write failure and return the validated in-memory core, and
8. materialize and lane-validate separately for each waiter.

Run the optional checkpoint L2 RPC verification after each request materializes its input.

### Step 4: Run focused verification

```bash
cargo fmt --all
cargo test -p raiko2-pipeline preflight_coordinator
cargo test -p raiko2-pipeline shasta
```

Expected: pass.

### Step 5: Commit

```bash
git add crates/pipeline
git commit -m "feat(preflight): coalesce canonical Shasta builds"
```

## Task 7: Inject One Coordinator into Every Lane

**Files:**

- Modify: `bin/raiko2/src/server/state/mod.rs`
- Modify: `bin/preflight/src/main.rs`
- Modify: `crates/pipeline/src/forks/shasta/spec.rs`
- Test: `bin/raiko2/src/server/state/mod.rs`
- Test: `crates/pipeline/src/forks/shasta/spec.rs`

### Step 1: Write failing server wiring tests

Cover:

- every enabled lane for one network pair receives the same coordinator identity,
- different network pairs receive different coordinators,
- SGX and SGXGETH do not create separate preflight caches,
- RISC0 local and Boundless share the same pair coordinator,
- disabled lanes do not affect coordinator construction, and
- standalone `preflight` uses an explicit no-persistence or memory coordinator rather than hidden
  global state.

Run:

```bash
cargo test -p raiko2 server::state
cargo test -p preflight
```

Expected: fail before injection.

### Step 2: Change `ShastaSpec` construction

Add a constructor argument or builder for `Arc<PreflightCoordinator>`. Do not let
`ShastaSpec::new` silently allocate its own coordinator in server code.

In `register_pair_pipelines`, create one coordinator before iterating proof lanes and pass clones to
`build_risc0_engine`, `build_sp1_engine`, `build_native_engine`, `build_boundless_engine`, and
`build_remote_sgx_engine`.

Use a runtime-provided preflight store adapter so the same configured memory/GCS backend owns proof
and preflight persistence without creating a dependency cycle.

### Step 3: Preserve non-server callers

Update `bin/preflight` and test constructors explicitly. Fixture/dev callers may use a disabled
store, but production server lanes must use the shared runtime-backed coordinator.

### Step 4: Add cross-lane integration test

Submit the same proposal context concurrently through SGX, SGXGETH, SP1, RISC0, and native specs
backed by a counting provider. Assert locator and witness build counts are one while proof backend
calls remain one per requested lane.

### Step 5: Run focused verification

```bash
cargo fmt --all
cargo test -p raiko2 server::state
cargo test -p preflight
cargo test -p raiko2-pipeline
```

Expected: pass.

### Step 6: Commit

```bash
git add bin/raiko2 bin/preflight crates/pipeline crates/runtime
git commit -m "feat(server): share preflight cache across lanes"
```

## Task 8: Add Bounded Telemetry

**Files:**

- Modify: `crates/pipeline/src/forks/shasta/preflight_cache.rs`
- Modify: `bin/raiko2/src/server/telemetry.rs`
- Modify: `bin/raiko2/src/server/state/mod.rs`
- Test: `bin/raiko2/src/server/telemetry.rs`

### Step 1: Write failing metric tests

Cover:

- hit, miss, bypass, and error counters,
- load, build, validation, and cleanup duration observations,
- serialized size observations,
- leader and waiter counters/gauge balance,
- cleanup matched/removed/failed counts by exact scope, and
- absence of proposal ID, block number, key hash, verifier, and address labels.

Run:

```bash
cargo test -p raiko2 telemetry
```

Expected: fail before metric registration.

### Step 2: Add a pipeline observer contract

Keep `raiko2-pipeline` independent of Prometheus by defining a small no-op-by-default observer trait
with typed bounded events. Implement the Prometheus observer in the server and inject it into each
pair coordinator.

Use these metric families:

- `raiko2_preflight_cache_requests_total{pair,result}`,
- load/build/canonical-validation duration histograms,
- canonical serialized-size histogram,
- single-flight leader/waiter counters and waiter gauge,
- startup cleanup object counters by scope/outcome, and
- startup cleanup duration by scope.

The startup path records cleanup metrics directly from `StartupCleanupReport`; it does not need a
pipeline coordinator to exist before runtime initialization.

### Step 3: Run focused verification

```bash
cargo fmt --all
cargo test -p raiko2 telemetry
cargo test -p raiko2-pipeline preflight_coordinator
```

Expected: pass.

### Step 4: Commit

```bash
git add bin/raiko2 crates/pipeline
git commit -m "feat(metrics): observe preflight cache and cleanup"
```

## Task 9: Update Configuration and Operator Documentation

**Files:**

- Modify: `config.example.toml`
- Modify: `README.md`
- Modify: `docs/API.md`
- Modify: `docs/operations.md`
- Test: `bin/raiko2/src/config/runtime.rs`

### Step 1: Update the canonical config example

Replace the old boolean with:

```toml
[runtime]
startup_cleanup = ["proof"]
```

Document:

- normal SGX/ZK guest or verifier upgrades use `["proof"]`,
- derivation, fork, or witness-generation rule changes use `["proof", "preflight"]`,
- no startup cleanup is the default,
- there is no `input` scope,
- cleanup invalidates active manifests and leaves immutable content to GCS TTL, and
- cleanup runs before runtime initialization and aborts startup on failure.

### Step 2: Update API and operations docs

Keep `/v4/prover/invalidate-artifacts` documented as targeted proof invalidation. Explain that it is
not the startup scope API.

Describe the canonical preflight flow, lane sharing, cache key exclusions, GCS lifecycle dependency,
and the one-process-per-namespace requirement. Do not add environment-specific endpoints, local
paths, credentials, or personal names.

### Step 3: Verify examples and removed names

```bash
cargo test -p raiko2 config::runtime
rg -n "reset_namespace_on_start|startup_cleanup" \
  README.md docs/API.md docs/operations.md config.example.toml bin/raiko2
git diff --check
```

Expected: the removed boolean has no live reference; `startup_cleanup` is consistently documented.

### Step 4: Commit

```bash
git add README.md docs/API.md docs/operations.md config.example.toml bin/raiko2/src/config
git commit -m "docs: document canonical preflight cache cleanup"
```

## Task 10: Full Verification and Regression

**Files:**

- Modify only files required by failures found during verification.

### Step 1: Run formatting and targeted CI lanes

```bash
cargo fmt --all --check
cargo test -p raiko2-primitives-shasta
cargo test -p raiko2-protocol-shasta
cargo test -p raiko2-pipeline
cargo test -p raiko2-runtime
cargo test -p preflight
cargo test -p raiko2
cargo clippy --workspace -- -D warnings
```

Expected: all pass.

### Step 2: Confirm guest artifacts are untouched

```bash
git diff origin/main -- crates/guests/elf guests
```

Expected: no diff. A guest build is not required because the public `GuestInput`, guest source,
proof format, and public input are unchanged.

### Step 3: Run non-production integration regression

Using configured test endpoints and credentials rather than hardcoded values:

1. submit one proposal concurrently to every configured proof lane,
2. verify one canonical preflight build and per-lane proof execution,
3. restart with `startup_cleanup = ["proof"]` and verify preflight cache reuse,
4. restart with `startup_cleanup = ["proof", "preflight"]` and verify a new preflight build,
5. verify proposal and aggregate proving still complete, and
6. inspect metrics for bounded labels, expected hit/miss counts, cleanup counts, and no stuck waiters.

Do not replace or stop an externally used service during this regression.

### Step 4: Review hygiene

```bash
git diff --check
git status --short
```

Inspect every added line for machine-specific paths or person-identifying examples. Expected: none.

### Step 5: Commit verification-only fixes

If verification required code changes:

```bash
git add <changed-files>
git commit -m "fix(preflight): address integration findings"
```

Do not create an empty commit.

## Completion Criteria

- All proof lanes for one pair share one coordinator.
- Equal concurrent requests wait on one locator entry and one canonical core-build entry.
- Runtime proof tasks remain independent per request.
- No persisted input artifact exists.
- Cache identity excludes proof/verifier/guest identity and includes every execution-boundary input.
- Cache hits and misses run equivalent canonical validation.
- Production carry construction uses only the current host-resolved chain spec.
- `startup_cleanup` supports exact `proof` and `preflight` scopes.
- GCS cleanup deletes active manifests and leaves immutable content to lifecycle TTL.
- Memory and GCS behavior match.
- Metrics are bounded and operationally useful.
- Documentation and sample config are synchronized.
- Targeted tests, workspace Clippy, and non-production regression pass.
- Guest ELF files and on-chain formats are unchanged.
