---
title: Runtime and Guest Build Performance Optimization
date: 2026-07-05
category: performance-issues
module: build tooling
problem_type: performance_issue
component: tooling
symptoms:
  - Runtime image builds spent minutes on release-profile work that was not needed for the host image path
  - Docker and guest builds missed reusable cache layers for Cargo, sccache, and zkVM native dependencies
  - Guest refreshes repeated work instead of hitting a fast fingerprint skip when inputs were unchanged
root_cause: missing_tooling
resolution_type: tooling_addition
severity: high
related_components:
  - development_workflow
tags:
  - runtime-image
  - guest-builds
  - docker-buildkit
  - cargo-cache
  - sccache
  - mold-linker
  - cargo-chef
  - lto
---

# Runtime and Guest Build Performance Optimization

## Problem

Runtime image builds and guest refreshes were paying for work that should have been cached, skipped, or avoided entirely. The slow paths mixed several independent costs: release-profile host helper builds, cold Docker dependency compilation, native zkVM C/C++ compilation, repeated guest rebuilds, and oversized runtime artifacts from release debug info.

The solution had to preserve release-image and guest ELF semantics while making the optimized path measurable. PR #140 used a benchmark matrix rather than a single wall-clock number so cold, warm, source-change, and fingerprint-hit paths could be compared separately.

## Symptoms

- The host-side guest launcher previously spent about `6m12s` before reaching guest work. With the launcher profile reduced, the first rebuild dropped to about `2m11s`, and a subsequent launcher hit took about `0.49s`.
- A cold runtime image warm-fill still took `1346.78s`, with `cargo chef cook` accounting for `20m39s` and the final workspace build `54.85s`.
- A source-change runtime Docker image rebuild with `cargo-chef` took `50.92s` total, with the final Cargo build at `29.92s`. The measured optimized release-image-style path is `51.41s` total: `0.490s` fingerprint-hit guest refresh plus `50.92s` cached Docker build.
- The original end-to-end image build time, including pre-optimization guest refresh plus Docker image build, was not captured. The old Docker-only LTO baseline was already `909.56s`, so the optimized `51.41s` end-to-end path is at least `17.7x` faster than the old Docker portion alone; the exact original end-to-end improvement is higher but unmeasured.
- Enabling host LTO was not acceptable for the Docker image-build portion: the pre-PR Docker run using the removed workspace LTO profile took `909.56s` total, with a `10m03s` final Cargo build.
- The old no-LTO/debug release profile produced very large artifacts: `793.9MB` for `/usr/local/bin/raiko2` and `942.4MB` for the image.
- A RISC0 forced guest rebuild with Rust and C/C++ both routed through `sccache` took `56.94s`; narrowing guest `sccache` to C/C++ only reduced that clean-target path to `48.22s` while preserving `15/15` C/C++ cache hits.
- Disabling guest C/C++ `sccache` on the same clean RISC0 rebuild took `47.43s`; adding the current cached Docker image build gives `98.35s` for a current forced guest rebuild plus Docker build diagnostic path. This is not an original pre-optimization end-to-end baseline.
- A fingerprint-hit `just build-guest risc0` took `0.490s`; forced rebuilds and fingerprint hits are different regimes and should not be averaged together.
- Clean RISC0 guest target rebuilds were not final-link-bound: `cargo +risc0 build --timings` reported `507` dirty units over `57.30s`, with the longest units in `protobuf`, `ark-ff`, and `zerocopy`; touching the final guest binary rebuilt in only about `2.32s`.

### Benchmark Setup

The benchmark matrix intentionally compared different cache states instead of reducing the work to one number:

- cold or profile-invalidated runtime image builds measure dependency and profile-boundary cost;
- source-change release-image-style builds should add guest refresh time to Docker image-build time;
- source-change Docker image rebuilds measure whether the dependency boundary stays cached;
- guest `sccache` measurements should distinguish Rust wrapper overhead from native C/C++ compiler cache hits;
- no-chef transition runs measure the cost of removing `cargo-chef`, not only the steady-state result after a new target cache is warm;
- forced guest rebuilds measure actual guest compilation and export;
- fingerprint-hit guest runs measure the no-op path.

Absolute times depend on the Docker builder, cache temperature, Rust/toolchain versions, feature set, base image, and whether caches were evicted. Future comparisons should record those states next to the measured result.

## What Didn't Work

- Treating the imported buildx cache as proof of cache effectiveness was too weak (session history). A prior no-push runtime image cache gate imported the local cache manifest, but `cargo chef cook --release` still missed and began updating crates.io and git repositories. Step-level `CACHED` evidence matters.
- Treating the guest build delay as only a linker-choice problem was too narrow (session history). Earlier `just build-guest risc0` no-op validation timed out after `270.15s` before reaching Docker guest work because the host launcher was still compiling/linking in release mode with LTO.
- Keeping workspace release LTO enabled was too expensive for this host image path. It reduced size compared with the old debug-info release profile, but the final Cargo build still took `10m03s`.
- Keeping `[profile.release] debug = 1` made the no-LTO binary and image too large. Returning to Cargo defaults removed both full LTO and release debug info from the default path.
- Removing `cargo-chef` was not a clear win. A fully warmed no-chef source-change rebuild matched the cargo-chef path (`50.76s` vs `50.92s`), but the first transition into the no-chef path regressed to `154.77s`.
- Using `--skip-guest-refresh` as a broad shortcut is unsafe when source/config changes are guest-facing (session history). Prior release work showed host/preflight and guest ELF can diverge when compiled-in chain spec data changes but guest artifacts are not refreshed.
- A local benchmark wrapper completed Docker image build/load but failed during post-processing because it used zsh's read-only `status` variable. The artifact sizes were collected directly with `docker image inspect` and `docker run --entrypoint stat`.

## Solution

The final fix split the build system into separate runtime image, host launcher, and guest build decisions.

Runtime Docker builds now use BuildKit cache mounts for the expensive reusable state:

```dockerfile
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    --mount=type=cache,target=/var/cache/sccache,sharing=locked \
    cargo +1.94.0 build --release -p raiko2 ${BIN_FEATURES}
```

The runtime Dockerfile defaults Rust and native compilation through `sccache`, and uses `mold` for host linking:

```dockerfile
ARG RUSTFLAGS="-C link-arg=-fuse-ld=mold"
ARG RUSTC_WRAPPER=sccache
ARG CC="sccache cc"
ARG CXX="sccache c++"
```

`xtask release-image host` keeps the host-only feature set but no longer carries a separate LTO override:

```text
BIN_FEATURES=--no-default-features --features host
```

The workspace release profile was returned to Cargo defaults by deleting the override:

```toml
# Removed
[profile.release]
debug = 1
lto = true
```

That made the default release path match the host image requirement: no LTO and no debug info unless a caller explicitly overrides Cargo profile environment variables. The measured result after profile invalidation was `cargo chef cook` at `4m22s`, final Cargo build at `47.69s`, a `102.1MB` binary, and a `250.7MB` image.

The host-side `xtask-build-guest` launcher is still intentionally cheaper than normal release code:

```make
build-guest backend="all" *args:
    CARGO_PROFILE_RELEASE_OPT_LEVEL=1 \
        cargo run -r -p xtask-build-guest --bin xtask-build-guest -- {{backend}} {{args}}
```

That override applies only to the launcher that orchestrates guest builds. It is not a claim about guest proof runtime performance.

Guest build orchestration now makes cache and skip behavior visible:

- backend fingerprints skip unchanged guest ELF refreshes;
- forced rebuilds write fingerprints so the next normal run can skip;
- Docker Cargo and repo-managed-image `sccache` cache volumes are deterministic per backend;
- RISC0/SP1 toolchain images install pinned `sccache`;
- guest C/C++ compilers are wrapped through `sccache` when supported, while guest Rust compilation stays on Cargo's normal path because the measured clean-target Rust wrapper path had cache misses that outweighed the native C/C++ cache hits;
- rebuild logs print elapsed time and `sccache --show-stats`.

The fingerprint optimization has a correctness boundary: every guest-affecting source, environment, toolchain image tag, build flag, and expected output must be part of the fingerprint inputs. Otherwise a performance skip can become a stale-artifact bug.

## Why This Works

Returning release builds to Cargo defaults removed one class of avoidable host-image cost without inventing a custom profile: final-build LTO and release debug symbols. Cargo already defaults release builds to optimized code without LTO or debug symbols, which is the right default for a host binary that is not the hot path being optimized.

BuildKit cache mounts, `sccache`, and `cargo-chef` address a different class of cost: repeated Rust and native dependency compilation inside Docker. Cache mounts keep Cargo registry, git checkout, target artifacts, and compiler cache state outside the image layer but available to subsequent builds. Those cache hits depend on stable mount targets, the same BuildKit builder, compatible profile/features/toolchain/base-image inputs, and cache state that has not been pruned.

`cargo-chef` remains useful as a stable Docker dependency boundary. The no-chef warm path can be competitive once its cache is already hot, but the first transition penalty makes removal a regression for normal branch and image-build workflows.

The guest path benefits most from fingerprinting and cache observability. Timing showed that clean guest rebuilds were dominated by many Rust units, not only final linking. Routing those Rust misses through `sccache` made the clean-target path slower, so the final guest cache wiring keeps `sccache` on native C/C++ compilers only and relies on Cargo target/cache reuse plus fingerprints for the Rust graph.

The benchmark matrix was essential because each row answered a different question: whether a no-op truly skips, whether native C/C++ cache hits, whether `cargo-chef` is still buying anything, whether LTO is justified, and whether binary-size work actually changed the artifact.

## Prevention

- Measure build performance by regime:
  - cold or profile-invalidated runtime Docker image build;
  - source-change release-image-style build, including guest refresh plus Docker image build;
  - source-change Docker image rebuild;
  - no-chef transition and warm no-chef rebuild before removing `cargo-chef`;
  - forced guest rebuild;
  - fingerprint-hit guest skip.
- Do not count mixed synthetic totals as original build times. If the original end-to-end image build was not measured, report the current end-to-end time and the measured component deltas separately.
- Treat release-profile changes as artifact decisions. Before enabling LTO or debug info on host images, record build time, binary size, image size, and a runtime benchmark that justifies the tradeoff.
- Keep host helper tuning separate from guest artifact tuning. `CARGO_PROFILE_RELEASE_OPT_LEVEL=1` is appropriate for `xtask-build-guest` because it is orchestration code; do not use that measurement as proof about zkVM guest runtime behavior.
- Require step-level cache evidence for Docker builds. A buildx cache import is not enough; check whether `cargo chef cook`, final `cargo build`, and native compiler work actually hit cache.
- Monitor cache validity, not just elapsed time:
  - `cargo chef cook` and final `cargo build` should show expected `CACHED` behavior for no-op Docker steps;
  - `sccache --show-stats` should show hit/miss counts and cache size;
  - forced guest rebuild followed by normal build should produce a stable skip;
  - unexpected ELF/VK diffs indicate the fingerprint boundary or build inputs need review.
- Do not use `--skip-guest-refresh` when source changes can affect guest-facing compiled data. Review generated ELF/VK diffs after guest builds and keep the worktree clean before formal `release-image` runs.
- Keep benchmark wrappers shell-portable. Avoid zsh reserved variables such as `status`; if wrapper post-processing fails after a successful build/load, collect artifact facts directly from Docker and record the wrapper failure separately.
- For changes in this area, use focused validation before publishing benchmark claims:

```sh
cargo fmt --all -- --check
cargo test -p xtask
cargo test -p xtask-build-guest
cargo clippy -p xtask -p xtask-build-guest -- -D warnings
git diff --check
docker buildx build --builder raiko2-local-cache --load --progress=plain \
  --build-arg 'BIN_FEATURES=--no-default-features --features host' \
  -t raiko2-bench:default-release-profile .
```

## Related Issues

- [PR #140](https://github.com/taikoxyz/raiko2/pull/140) — source PR and benchmark matrix for this optimization.
- [Issue #90](https://github.com/taikoxyz/raiko2/issues/90) — developer build latency, especially guest compilation.
- [Runtime image speed plan](../../plans/2026-07-05-001-perf-runtime-image-build-speed-plan.md) — implementation plan for Docker image cache, linker, and release-profile work.
- [Guest build speed plan](../../plans/2026-07-05-002-perf-guest-build-speed-plan.md) — implementation plan for guest fingerprinting, timing, and sccache support.
