# Guest-ELF-Consistency CI Build Optimization — Design

- **Date:** 2026-06-18
- **Status:** Approved (pre-implementation)
- **Owner:** David
- **Scope:** Reduce the wall time of the `guest-elf-consistency` job in `.github/workflows/ci.yml`
  (the self-hosted lane that rebuilds checked-in guest ELFs and verifies they don't drift) without
  changing any guest ELF bytes or the published digest summary.

## Problem

The `guest-elf-consistency` CI job takes ~28 minutes **and has been failing**. The slowness and the
failure share a root cause. This is a different build path from
`2026-06-18-release-build-optimization-design.md` (which targets `just release-image`); this job is
the guest-build / consistency lane.

### Diagnosis (from a real failing run)

Reference run: `ci` workflow on `main`, job `guest-elf-consistency`
(`actions/runs/27534690448/job/81380985210`), self-hosted runner `[self-hosted, linux, x64, raiko2]`,
~28 min, conclusion **failure**. Two steps dominate:

| Step | Time | Detail |
| --- | --- | --- |
| `rebuild guest elfs` | ~15.5 min | **~11m10s of it is the host compile of `xtask-build-guest` (release).** The actual in-Docker guest ELF compiles were only ~2 min each — those are well-cached via a per-repo Docker named volume (`<repo>-cargo-{risc0,sp1}`). |
| `export guest digests` | ~12.5 min → **`No space left on device`** | `cargo run` here omits `-r`, so it recompiles the *same heavy crate in the debug profile*, duplicating ~12 min of work **and** building a second `target/` tree until the runner disk fills. |

### Root causes (in order of leverage)

1. **Duplicate compile + disk blowout.** The digest step
   (`cargo run -p xtask-build-guest --bin guest-digests -- --output target/guest-digests/summary.json`)
   has no `-r`, so it builds a full second copy of the dependency tree in the **debug** profile on top
   of the release tree from the previous step. This is both ~12 wasted minutes and the cause of the
   `No space left on device` failure.
2. **Inflated builder compile.** `guest_digests` lives in the *same crate* as the builder
   (`xtask/build-guest/src/lib.rs` → `pub mod guest_digests;`). That module is the only code pulling in
   `risc0-zkvm` + `sp1-sdk` + `raiko2-guests` + `alloy-primitives`. Because the library always compiles
   that module, `just build-guest` pays an ~11-min prover-SDK compile it never uses — the builder itself
   only orchestrates Docker and hashes files (`risc0-binfmt`, `risc0-zkos-v1compat`, `sha2`).
3. **Cache never warms.** Because the job fails on disk, the `Post Swatinem/rust-cache` save step is
   skipped, so the `shared-key: ci-zk-build` cache never persists the SDK dependency artifacts → every
   run starts cold.

### Import analysis (confirms the dependency partition)

- `xtask/build-guest/src/lib.rs` (the builder) imports only: `risc0-binfmt`, `risc0-zkos-v1compat`,
  `sha2`, `anyhow`, `clap`, `serde`, `serde_json`, `toml`, `std`.
- `xtask/build-guest/src/guest_digests.rs` (the digest tool) is the sole user of the heavy deps:
  `risc0-zkvm` (`compute_image_id`), `sp1-sdk` (`ProverClient::setup`, `HashableKey`), `raiko2-guests`
  (ELF loaders), `alloy-primitives`.
- The main `xtask` crate already isolates these behind a default `guest-tools` feature and routes its
  `GuestDigests` subcommand through `xtask_build_guest::guest_digests::run`
  (`xtask/src/main.rs`). The CI job calls `xtask-build-guest` directly
  (`cargo run -p xtask-build-guest --bin …`), not through the `xtask` umbrella crate.

## Goal & success criteria

- The `guest-elf-consistency` job passes and is materially faster.
- Directional targets: **warm run ~6–8 min**, **cold run ~14–16 min** (one inherent prover-SDK compile
  in the digest tool), versus ~28 min and failing today.
- Hard constraints:
  - No change to any guest ELF bytes (`crates/guests/elf` stays drift-free).
  - The published guest digest summary (image IDs / VKs) is unchanged in content.
  - Each workstream is independently revertible by reverting a single diff.

## Approach

Three independent, independently-shippable workstreams, highest-leverage first.

### WS1 — Kill the duplicate compile (headline fix + failure fix)

Edit the digest step in `.github/workflows/ci.yml`:

```
cargo run -r -p xtask-build-guest --bin guest-digests --features digests \
  -- --output target/guest-digests/summary.json
```

- `-r` reuses the release artifacts already built by the `rebuild guest elfs` step instead of building
  a second debug copy. Removes ~12 min and the duplicate `target/` tree (fixing the disk OOM).
- `--features digests` is required because WS2 puts the bin behind that feature (see below). If WS1 ever
  lands before WS2, the `--features digests` flag is simply a no-op until the feature exists; land them
  together to avoid an intermediate broken state.
- Running in release also speeds the runtime VK/image-id derivation (debug `ProverClient::setup` is
  much slower). Output content is identical.

### WS2 — Feature-gate the prover SDKs out of the builder

Isolate the heavy deps so the builder no longer compiles them. Chosen mechanism: **feature-gate in
place** (no new crate).

- `xtask/build-guest/Cargo.toml`:
  - Mark `risc0-zkvm`, `sp1-sdk`, `raiko2-guests`, `alloy-primitives` as `optional = true`.
  - Add a feature: `digests = ["dep:risc0-zkvm", "dep:sp1-sdk", "dep:raiko2-guests", "dep:alloy-primitives"]`
    (not in `default`).
  - Declare the digest binary explicitly with a feature guard:
    `[[bin]] name = "guest-digests"`, `path = "src/bin/guest-digests.rs"`,
    `required-features = ["digests"]`.
  - Keep the builder's light deps non-optional: `risc0-binfmt`, `risc0-zkos-v1compat`, `sha2`, `clap`,
    `anyhow`, `serde`, `serde_json`, `toml`.
- `xtask/build-guest/src/lib.rs`: gate the module — `#[cfg(feature = "digests")] pub mod guest_digests;`.
- `xtask/Cargo.toml`: add `"xtask-build-guest/digests"` to the `guest-tools` feature so the `xtask`
  `GuestDigests` subcommand still compiles. (The `xtask` crate is not on the CI hot path; trimming its
  own direct heavy deps is out of scope — see below.)

Result: `cargo run -p xtask-build-guest --bin xtask-build-guest` (the builder, used by `just build-guest`
and the `rebuild guest elfs` CI step) builds without the `digests` feature, so `sp1-sdk`/`risc0-zkvm`
are never compiled for it. The prover-SDK compile lives only in the digest tool, built once and
cacheable.

Implementation note: confirm the dependency partition with `cargo tree -p xtask-build-guest` (no
`digests`) — it must not contain `risc0-zkvm`, `sp1-sdk`, `raiko2-guests`, or `alloy-primitives`. If
`risc0-binfmt` / `risc0-zkos-v1compat` unexpectedly drag in `risc0-zkvm`, fall back to also gating them.

### WS3 — Cache & disk reliability (self-hosted runner)

- **Cache warms automatically** once WS1 makes the job pass: the `Post Swatinem/rust-cache` save then
  runs and persists the SDK dependency artifacts under `shared-key: ci-zk-build`, so warm runs skip the
  ~11-min SDK compile. (Cache invalidates correctly on `Cargo.lock` / toolchain changes.)
- **Guarded low-disk prune** in the `free self-hosted runner disk` step: keep the `df -h` reporting, and
  *only when free space is below a threshold* (suggested default: < 20 GB available on the workspace
  filesystem, tunable) prune the `<repo>-cargo-{risc0,sp1}` Docker volumes as a last resort. These
  volumes are never pruned today (they keep guest compiles at ~2 min); preserve them normally and
  sacrifice them only to self-heal a near-full disk instead of OOMing. The existing
  container/image/builder prunes and diag-log cleanup stay.
- **Honest floor:** a truly cold runner still pays one prover-SDK compile (~11 min) in the digest tool —
  inherent to deriving VKs/image-ids in Rust. The split + cache make that rare, not gone.

## Validation

- Re-run the `ci` workflow on a branch and confirm the `guest-elf-consistency` job **passes**.
- `git status --porcelain --untracked-files=all -- crates/guests/elf` is empty (no ELF drift) — the
  job's existing consistency gate.
- The `guest-digests` step still writes `target/guest-digests/summary.json` and the step summary table
  is unchanged in content.
- Record before/after wall time for the job; expect warm runs well under the previous ~28 min.
- Rollback = revert the relevant workstream diff.

## Out of scope (deferred)

- Trimming the `xtask` umbrella crate's own direct heavy deps (`risc0-zkvm` / `sp1-sdk` /
  `raiko2-guests`) — `xtask` is not on the CI hot path; revisit only if it becomes one.
- Dropping or restructuring the digest computation itself (e.g. extracting VKs without the SDKs).
- The `just release-image` path — covered by `2026-06-18-release-build-optimization-design.md`.
- Splitting `guest_digests` into a standalone crate (the alternative to feature-gating, not chosen).

## Risks

| Change | Risk | Mitigation |
| --- | --- | --- |
| `-r` on the digest step | None expected; same output | Reuses already-built release artifacts; revert one line |
| Feature-gating heavy deps | A builder code path unexpectedly needs a gated dep → compile error | Caught immediately by CI; import analysis shows only `guest_digests` uses them; verify with `cargo tree` |
| `xtask` feature wiring | `GuestDigests` subcommand fails to compile | Add `xtask-build-guest/digests` to `guest-tools`; covered by building `xtask` with default features |
| Guarded volume prune | Occasionally cold-starts guest builds (~slower) | Only fires below a disk threshold; preserves the cache volumes in the common case |
| WS1 landing before WS2 | `--features digests` references a non-existent feature | Land WS1 + WS2 together |
