# Release Build Optimization — Design

- **Date:** 2026-06-18
- **Status:** Approved (pre-implementation)
- **Owner:** David
- **Scope:** Reduce raiko2 release/image build wall time (currently ~40 min) without
  changing shipped binary behavior or guest ELF artifacts.

## Problem

The raiko2 release build takes ~40 minutes. The `raiko2` binary links the entire prover
stack (risc0 + sp1 + reth/revm/alloy) into a single binary, and the release profile uses
**fat LTO** (`lto = true`) over a ~1482-crate dependency graph (reth 106, alloy 77, revm 13,
risc0 19, sp1 19). The shipped path is `just release-image` → `docker buildx build`
(`Dockerfile`) → `cargo chef cook --release` + `cargo build --release -p raiko2`.

Two build regimes exist because of `cargo-chef`:

- **Warm rebuild** (only raiko2 source changed — the common case): the chef "cook" dependency
  layer is cached, so wall time is almost entirely the final `cargo build -p raiko2`, dominated
  by **fat LTO over all dependency bitcode**. Fat LTO *is* the cost here.
- **Cold rebuild** (dependency bump / fresh runner): compiling the 1482-crate tree dominates,
  with LTO on top.

Notably, **CI lanes use `mold` + `sccache`, but the `Dockerfile` uses neither** — the actual
release image build links with the default linker and no compiler cache.

## Key insight (why the headline lever is low-risk)

raiko2's host binary is **not** where the heavy compute lives. Real proving runs in the guest
ELFs (built separately — unaffected by `[profile.release]`) or on remote networks
(Boundless / SP1 network). The host does orchestration + preflight. Fat LTO therefore buys the
host binary very little real-world speed, so dropping to thin LTO is effectively free.

## Goal & success criteria

- Cut the release/image build from ~40 min.
- Directional target (refine after Phase 0): **warm rebuild well under ~15 min**; cold rebuild
  materially reduced.
- Hard constraints:
  - No behavior change to the shipped binary.
  - Guest ELF artifacts untouched.
  - Every change reversible by reverting a single diff.

## Approach

Measure first, then apply staged changes (highest-leverage, lowest-risk first). Each phase is
independently shippable and measurable.

### Phase 0 — Measure (do first, ~one build)

Run on whichever build is the "40-min one":

- **Local proxy (fast iteration):** `cargo build --release -p raiko2 --timings` →
  open `target/cargo-timings/cargo-timing.html`. Read total time, slowest crates, and time
  spent in the final `raiko2` unit (= the LTO/link cost).
- **Cold vs warm split:** measure once from clean (cold = dep compile dominates), then `touch`
  one raiko2 source file and rebuild (warm ≈ pure final LTO/link cost).
- **Docker (shipped path):** `docker buildx build --progress=plain …` and read per-step seconds
  — `chef cook` (deps) vs final `cargo build`.

**Output:** a one-line verdict — *warm-link-bound* (→ Phase 1 is the whole win) vs
*cold-dep-bound* (→ Phase 3 also matters).

### Phase 1 — Profile tuning (headline lever, one file)

Edit root `Cargo.toml` → `[profile.release]`:

- **`lto = "thin"`** (from fat `true`) — single biggest warm-build win; effectively free here
  (compute is in guests/remote, not the host binary).
- **`debug = 0`** (from `1`) — *default decision: set to 0* for faster link + smaller binary.
  Tradeoff: loses backtrace line numbers in panics. Revert to `1` if readable prod panics are
  valued more than the marginal build time.
- Leave `codegen-units` unset — thin LTO + default 16 codegen units parallelize codegen.
- **Escape hatch:** `lto = false` only if Phase 0 shows the link *still* dominates after thin.

### Phase 2 — Dockerfile linker parity (Linux build)

- Install **mold** in the `builder` stage of `Dockerfile` and link through it (via a build-stage
  `.cargo/config.toml` or `RUSTFLAGS=-Clink-arg=-fuse-ld=mold`). CI already uses mold; the image
  build does not. Speeds the final link and compounds with thin LTO. Falls back to the default
  linker if it ever breaks.

### Phase 3 — Docker dependency cache (gated on Phase 0)

- **Only if** Phase 0 shows cold-dep-compile is a recurring cost: add BuildKit
  `--mount=type=cache` for the cargo registry/git and `target/`, and settle chef-vs-cache-mount
  coexistence at that point. Skipped entirely if warm builds dominate.

## Validation

- Image builds successfully; `raiko2 --help` and server startup work.
- `just release-image` guest-refresh step stays clean (no ELF drift).
- Record before/after wall time for both warm and cold builds.
- Rollback = revert the `Cargo.toml` / `Dockerfile` diff.

## Out of scope (deferred — Approach C)

- Backend-split images (risc0-only / sp1-only builds via the existing `release-image` backend
  arg) to shrink the per-image compile + LTO graph.
- sccache inside the Docker build with a persistent backend.

Revisit only if Phases 1–2 miss the target.

## Risks

| Change | Risk | Mitigation |
| --- | --- | --- |
| `lto = "thin"` | Negligible host-perf change | Compute is in guests/remote; reversible |
| `debug = 0` | Lose panic line numbers | Keep `debug = 1` if prod backtraces matter |
| mold in Docker | Link could break | CI already uses mold; fall back to default linker |
| BuildKit cache mounts | chef interaction complexity | Gated on Phase 0; skip if warm-bound |
