# Manual Rust Heavy Workflow Design

## Goal

Add a manual GitHub Actions workflow that runs the currently omitted heavy Rust test lanes for
`raiko2-prover`, `raiko2-engine`, and `raiko2`, so maintainers can compare GitHub-runner behavior
against the latest local codebase without making those lanes block every PR.

## Context

The default `ci.yml` intentionally keeps the prover/engine/server stack out of the required PR gate.
Those packages are slower, have a larger dependency graph, and previously hit an upstream
`boundless-market` failure on GitHub runners. Recent local verification on the latest codebase shows
`cargo test -p raiko2-engine --no-run` succeeds from a clean target, so the next step is to measure
the latest GitHub-runner behavior separately instead of reintroducing these lanes directly into the
default PR workflow.

## Requirements

- Trigger manually through `workflow_dispatch`.
- Allow selecting a target branch to test.
- Allow running only one heavy lane or all heavy lanes.
- Reuse the same Rust CI environment defaults that reduced compile cost in `ci.yml`.
- Stay outside required PR checks and avoid altering the current default PR gate.

## Approaches Considered

### 1. Add heavy lanes back into `ci.yml`

This restores coverage in the main PR workflow, but it also reintroduces the same instability and
cost that caused the heavy lanes to be removed. It is the wrong first step while the current GitHub
runner behavior is still uncertain.

### 2. Add a separate manual `rust-heavy.yml` workflow

This keeps the default PR signal stable while still giving maintainers a reproducible GitHub-runner
path for prover/engine/server coverage. It also lets us test the latest dependency state before
deciding whether any lane is safe to rejoin the required checks.

### 3. Add a scheduled nightly heavy workflow

This would provide continuous signal, but it introduces a second source of failure noise immediately
and does not help with ad hoc branch-specific validation as directly as a manual workflow.

## Decision

Use approach 2.

## Workflow Shape

- File: `.github/workflows/rust-heavy.yml`
- Trigger: `workflow_dispatch`
- Inputs:
  - `lane`: `prover`, `engine`, `server`, or `all`
  - `ref`: optional branch name; defaults to the branch the workflow was dispatched from
- Jobs:
  - `test-prover`
  - `test-engine`
  - `test-server`
- Each job runs only when selected by the `lane` input.

## Command Scope

The workflow runs full tests, not compile-only probes:

- `cargo test -p raiko2-prover -p guest-launcher`
- `cargo test -p raiko2-engine`
- `cargo test -p raiko2`

This keeps the workflow aligned with the coverage gap that exists after removing these lanes from the
default PR gate.

## Environment

Reuse the same Rust CI tuning already used in `ci.yml`:

- `CARGO_PROFILE_DEV_OPT_LEVEL=0`
- `CARGO_PROFILE_TEST_OPT_LEVEL=1`
- `RUSTFLAGS=-Cdebuginfo=0`
- `RUSTC_WRAPPER=sccache`
- `SCCACHE_GHA_ENABLED=true`
- `mold`, `rust-cache`, and `sccache-action`

## Non-Goals

- Do not add these jobs back to required PR checks yet.
- Do not change branch protection or PR rules.
- Do not build guest ELFs here; that remains covered by existing workflows.
- Do not add a nightly schedule yet.

## Verification

Local verification for this change is limited to workflow syntax and repository consistency:

- Parse the new workflow YAML.
- Run `git diff --check`.

GitHub is the source of truth for end-to-end verification because the workflow exists specifically to
measure GitHub-runner behavior for the heavy lanes.
