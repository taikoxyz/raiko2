# Guest Provenance, Blob Timestamp Parity, and Launcher Cleanup Design

## Status

Approved in conversation on 2026-07-10.

## Goal

Make checked-in guest artifacts provably fresh for the source tree that produced them, align
Shasta blob lookup with `taiko-client-rs`, and reduce duplicate proving logic without changing
block derivation, hardfork schedules, or upstream dependency revisions.

## Scope

This change has three deliverables:

1. Replace the incomplete guest build fingerprint with one deep guest-artifact provenance Module.
   Its Interface must support both build-cache decisions and a non-building freshness check.
2. Resolve every Shasta blob sidecar slot from the derivation source's own
   `blobSlice.timestamp`, matching the driver.
3. Make `guest-launcher` a thin Adapter over the production SP1 prover Interface for standard
   proposal and aggregation runs, while retaining direct SDK execution for experiment-only lab
   stages. Remove the two one-line gaiko2 pass-through Modules.

The following work is explicitly excluded:

- Mainnet Unzen fork-time or fork-schedule changes.
- Alethia Reth, Reth, or `taiko-client-rs` revision changes.
- Trailing-RLP manifest decoding changes.
- A shared host/guest proposal-derivation Module.
- Anchor-transaction byte reconstruction.
- Runtime image publication, verifier registration, or cluster rollout.

## Design principles

- The checked-in provenance manifest is the single source of truth for guest freshness. A hidden
  target-directory fingerprint must not compete with it.
- The provenance Interface is the test surface. Callers should ask whether a backend is fresh or
  refresh it; they should not know which files participate in the fingerprint.
- Dependency discovery follows Cargo's resolved graph rather than a hand-maintained crate list.
  This keeps locality when guest dependencies are added or removed.
- Generated ELF and VK files are only changed by the existing guest build entrypoints. They are
  never hand-edited.
- Standard launcher proving crosses the same production Interface as the server. Experiment-only
  labs remain a separate Adapter because they intentionally accept arbitrary programs and expose
  low-level zkVM metrics.

## Guest artifact provenance Module

### Location and Interface

Create a focused Module under `xtask/build-guest/src/provenance.rs`. The existing build
orchestrator in `xtask/build-guest/src/lib.rs` will use it through two operations:

- compute and verify provenance for one concrete backend;
- write fresh provenance after that backend's artifacts have been exported.

`Backend::All` remains orchestration only and invokes the Interface once for RISC0 and once for
SP1. The Module will own dependency discovery, source hashing, artifact enumeration, manifest
serialization, and mismatch diagnostics.

### Resolved input closure

For each backend, run Cargo metadata against `guests/<backend>/Cargo.toml` with the backend's
effective feature set and locked dependency resolution. Starting from the guest package, collect
all local path/workspace packages reachable in the resolved graph. For each local package, hash:

- its `Cargo.toml`;
- source trees containing Cargo targets;
- its build script when present.

Also hash build inputs that Cargo's package graph does not represent:

- root `Cargo.toml` because local crates inherit workspace dependencies;
- the backend guest `Cargo.lock`;
- `rust-toolchain.toml`;
- the backend toolchain Dockerfile;
- the `xtask-build-guest` manifest and source;
- existing effective toolchain, compiler, rustflag, platform, and prover-mode inputs already
  included by the current fingerprint.

Paths are normalized relative to the repository root, sorted, and tagged before hashing. A local
package outside the repository root is rejected rather than silently omitted. Missing files and
Cargo metadata failures are hard errors with the backend and path in the message.

This closes the current false-negative case in which changes to `raiko2-primitives`,
`raiko2-primitives-shasta`, `raiko2-protocol`, `raiko2-protocol-shasta`, or
`raiko2-stateless` can leave the guest fingerprint unchanged.

### Artifact closure

Artifact enumeration comes from the guest manifest's binary targets:

- RISC0 requires one `.elf` per binary.
- SP1 requires both one `.elf` and its paired `.vk.bin` per binary.

The exact relative path and SHA-256 digest of every expected artifact is recorded. Missing,
unexpected, or byte-mismatched artifacts fail verification. Artifact bytes are not folded into
the source fingerprint; they are represented explicitly so diagnostics can distinguish source
drift from artifact corruption.

### Tracked manifest

Write one deterministic manifest per backend:

- `crates/guests/elf/risc0.provenance.json`
- `crates/guests/elf/sp1.provenance.json`

Each manifest contains:

- schema version;
- backend;
- `bench` feature state;
- source fingerprint;
- a path-sorted map of artifact SHA-256 digests.

The JSON contains no timestamp, absolute path, hostname, or commit SHA. This makes identical
inputs deterministic across worktrees and avoids making a commit hash part of its own provenance.

### Build and check behavior

Extend `BuildGuestArgs` with `--check`:

- Normal mode verifies the tracked manifest first. A complete match is the cache hit and skips the
  build. A mismatch triggers the existing backend build, then writes the tracked manifest.
- `--force` always rebuilds and rewrites the manifest.
- `--check` never invokes Docker or a guest compiler. It recomputes the current source fingerprint
  and artifact digests and fails on any mismatch.
- `--check` and `--force` are mutually exclusive.

Remove the target-directory `GuestBuildFingerprint` cache and its read/write helpers. The tracked
manifest now provides more leverage through one Interface and better locality for freshness bugs.

### CI and operator workflow

Add a provenance check to the existing `rust-test-xtask` job after its unit tests:

```text
cargo run -p xtask-build-guest --bin xtask-build-guest -- all --check
```

The job already runs for Rust changes, guest artifact changes, workflow changes, and manual
dispatches, so it will catch guest-facing source changes without rebuilding either zkVM guest.
The manual `sync-guest-elf` workflow continues to run `just build-guest`; that command will now
write the provenance manifests beside generated artifacts and include them in its existing ELF
artifact diff.

`docs/development.md` will document local check, refresh, and failure-remediation commands.

## Blob timestamp parity

In `crates/provider/src/network/blobs.rs`, every derivation source will select its beacon slot from
`source.blobSlice.timestamp`. Proposal timestamp fallback is removed for normal sources.

The source timestamp is authenticated as part of the proposed derivation source and is the value
used by the current `taiko-client-rs` driver when requesting manifest sidecars. The existing
versioned-hash and KZG checks remain unchanged, so this change affects lookup parity and
availability, not guest trust assumptions.

Extract the timestamp selection into a small pure helper only if needed to make the behavior
directly testable. Tests must cover both normal and forced-inclusion sources and prove that neither
uses the proposal timestamp.

## Thin guest-launcher Adapter

### Standard proposal and aggregation paths

For `--stage proposal --proof-type sp1`, construct the existing production `Sp1Config`, load the
normal Shasta SP1 backend, instantiate `Sp1Prover`, and call the production `Prover` Interface.
For aggregation, read the input `Proof` files into `AggregationGuestInput` and call the production
aggregation Interface.

The returned production `Proof` is written directly. This deletes launcher-owned copies of:

- SP1 stdin construction for proposal and aggregation;
- local/mock/network setup and proof submission;
- compressed subproof loading and aggregation input construction;
- proof payload and quote encoding;
- verifier-key/image-id consistency checks;
- network mode URL and signer setup.

The production Module becomes the single implementation of those invariants. In particular,
network requests use the same retry, progress, verification, and output construction behavior as
the server.

Execute-mode benchmark fields are reconstructed from the public `Sp1ExecutionMetadata` already
stored under the production proof's `extra_data.sp1` entry. Existing JSON report field names stay
stable.

### Experimental paths

Opcode, REVM-opcode, and precompile lab stages continue to use direct SP1 SDK execution. These
stages are genuine alternate Adapters: they accept arbitrary lab ELFs and expose execution reports
that are not production proof requests.

The single `--elf` override remains supported for lab stages. Standard proposal and aggregation
paths require a coherent backend artifact set so ELF and VK bytes cannot be mixed. Operators who
need a custom standard guest set must use `RAIKO2_GUEST_ELF_DIR`, which is already the production
backend override Interface.

Standard SP1 proposal proofs remain compressed and aggregation proofs remain PLONK, matching the
production request contexts. Unsupported explicit combinations fail before any proving work.
Native and RISC0 launcher behavior is unchanged.

### Shallow Module deletion

Delete:

- `crates/prover/src/gaiko2/adapter.rs`
- `crates/prover/src/gaiko2/protocol.rs`

Update the one internal example caller to import the canonical `remote_prover` Module directly.
These files fail the deletion test: removing them eliminates an Interface without moving any
implementation complexity.

Do not delete the `protocol-shasta` upstream reexport Modules in this change. They currently form
the compatibility seam around the pinned `taiko-client-rs` protocol surface, and removing them
would create broad public-path churn unrelated to this PR's correctness goals.

## Error handling

- Provenance mismatches list the backend and distinguish source fingerprint, missing artifact,
  artifact set, and artifact digest failures.
- `--check` exits nonzero and tells the developer to run `just build-guest <backend>` or dispatch
  `sync-guest-elf`.
- Cargo metadata or path-normalization errors fail closed; no source path is silently skipped.
- Standard guest-launcher `--elf` misuse explains that lab stages accept a single ELF and standard
  stages use `RAIKO2_GUEST_ELF_DIR`.
- Existing provider RPC and preflight error categories remain unchanged.

## Test strategy

Implementation follows red-green-refactor cycles.

### Provenance tests

- A temporary local dependency graph proves that changing a transitive path crate changes the
  source fingerprint.
- The real guest graph includes the five shared crates previously omitted.
- SP1 artifact enumeration requires paired VK files.
- Verification rejects a changed source fingerprint, a missing artifact, an unexpected artifact,
  and modified artifact bytes.
- Deterministic serialization produces stable ordering and no machine-local paths.
- `--check`/`--force` conflict is rejected.

### Provider tests

- Normal source uses `blobSlice.timestamp` even when proposal timestamp differs.
- Forced-inclusion source uses the same source timestamp rule.
- Existing timestamp-to-slot validation remains green.

### Launcher and cleanup tests

- Standard SP1 CLI settings map to the production proposal and aggregation contexts.
- A single proposal `--elf` override is rejected with the backend-directory remediation.
- Lab stages still accept explicit ELFs.
- Production execute metadata maps to the existing benchmark JSON fields.
- `cargo check --all-targets` proves no caller relies on deleted gaiko2 reexport paths.

### Final verification

Run, at minimum:

- `cargo fmt --all -- --check`
- `RISC0_SKIP_BUILD_KERNELS=1 cargo test -p xtask-build-guest --features digests`
- `RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2-provider`
- `RISC0_SKIP_BUILD_KERNELS=1 cargo test -p guest-launcher`
- `RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2-prover`
- `cargo clippy --workspace -- -D warnings`
- `just build-guest all --force`
- `cargo run -p xtask-build-guest --bin xtask-build-guest -- all --check`
- `git diff --check origin/main...HEAD`

Both guest families must be regenerated because the initial tracked provenance cannot truthfully
certify pre-existing artifacts against the current `origin/main` source tree.

## Delivery

Implementation is based on live `origin/main` and delivered as a draft pull request. The PR body
will state the excluded Unzen and dependency-pin work, list exact validation commands, and call out
the regenerated ELF/VK and provenance artifacts for review.
