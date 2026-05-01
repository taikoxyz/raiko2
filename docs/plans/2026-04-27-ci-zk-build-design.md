# CI ZK Build Design

## Goal

Extend the repository CI so it covers:

- the existing Rust validation and test suite
- guest zk build verification for both `risc0` and `sp1`
- a documented future hook for native fixture or regression execution

The immediate scope is GitHub Actions only. Native fixture or regression execution is not enabled in
this change.

## Requirements

1. Keep the current Rust CI path that runs formatting, clippy, and workspace tests.
2. Add zk build coverage for both guest backends.
3. Keep failure isolation clear so a broken `risc0` build does not hide a passing `sp1` build, and
   vice versa.
4. Leave a clear TODO for a future native fixture or regression job without wiring it up now.

## Recommended Approach

Use one Rust job plus two parallel zk build jobs:

- `rust`: `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`,
  `cargo nextest run --workspace`
- `build-guest-risc0`: `just build-guest risc0`
- `build-guest-sp1`: `just build-guest sp1`

This keeps the current test behavior intact while making zk build failures easy to localize.

## Triggering

Keep the existing `paths-filter` gate, but split it into two booleans:

- `rust`: Rust workspace, config, and CI changes
- `zk`: guest sources, guest build tooling, release image tooling, Docker build inputs, and CI
  changes

That prevents guest-only changes from skipping zk build verification.

## Native Follow-Up

Do not add a runnable native fixture or regression job yet.

Instead, leave a TODO comment block in the workflow near the build jobs that points to the expected
future area:

- native fixture smoke for a local fixture-backed path, or
- native regression harness execution when a deterministic CI-safe entrypoint exists

## Non-Goals

- No rollout, deployment, or registry publication changes
- No conversion of CI to a larger matrix job
- No native regression execution in this patch
