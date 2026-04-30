# Manual Rust Heavy Workflow Design

## Goal

Add a manual GitHub Actions workflow that runs compile-only smoke checks for the heavy prover,
engine, and server stacks, so maintainers can confirm those bin-oriented graphs still type-check on
GitHub runners without making full heavy test execution block every PR.

## Context

The default `ci.yml` intentionally keeps the prover/engine/server stack out of the required PR gate.
Those packages are slower, have a larger dependency graph, and pull in heavy bin-side dependencies
such as `sp1-sdk`, `reth`, and their transitive AWS/RocksDB stacks. Recent manual workflow runs show
that full `cargo test` on GitHub-hosted runners spends most of its time cold-compiling rather than
executing tests. The default PR workflow is already centered on logic-focused crate tests plus guest
ELF builds, so the separate heavy workflow should shift from full test execution to compile smoke.

## Requirements

- Trigger manually through `workflow_dispatch`.
- Allow selecting a target branch to test.
- Allow running only one heavy lane or all heavy lanes.
- Reuse the same Rust CI environment defaults that reduced compile cost in `ci.yml`.
- Cover test code and dev-dependencies for the selected package, not just the main library target.
- Stay outside required PR checks and avoid altering the current default PR gate.

## Approaches Considered

### 1. Keep full `cargo test` in the manual workflow

This preserves the strongest signal, but it is the least practical use of GitHub-hosted runners for
these packages right now. The runs are dominated by compile cost, so a full test workflow provides
little extra value per minute spent.

### 2. Convert the manual workflow to `cargo check --tests`

This keeps the default PR signal stable while still giving maintainers a reproducible GitHub-runner
path for prover/engine/server compile coverage. `--tests` still pulls in test targets and
dev-dependencies, so it covers the heavy bin-side graphs that plain `cargo check` would miss, while
avoiding the extra cost of actually running tests.

### 3. Drop the manual heavy workflow entirely

This would simplify maintenance, but it would also remove the only GitHub-hosted path for checking
the heavy bin stacks after they were removed from the default required checks.

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

The workflow runs compile-only smoke commands that still include tests/dev-dependencies:

- `cargo check -p raiko2-prover -p guest-launcher --tests`
- `cargo check -p raiko2-engine --tests`
- `cargo check -p raiko2 --tests`

This keeps the workflow aligned with the remaining coverage gap after removing these lanes from the
default PR gate, while avoiding long GitHub-hosted runs that mostly measure cold compilation cost.

## Environment

Reuse the same Rust CI tuning already used in `ci.yml`:

- `CARGO_PROFILE_DEV_OPT_LEVEL=0`
- `CARGO_PROFILE_TEST_OPT_LEVEL=1`
- `RUSTFLAGS=-Cdebuginfo=0`
- `mold`

Unlike the default PR workflow, this manual smoke workflow does not need to mirror every cache
integration. Its job is to provide a lightweight GitHub-hosted compile signal for the heavy stacks,
not to optimize full test throughput.

## Non-Goals

- Do not add these jobs back to required PR checks yet.
- Do not change branch protection or PR rules.
- Do not build guest ELFs here; that remains covered by existing workflows.
- Do not add a nightly schedule yet.
- Do not turn this workflow back into full `cargo test` while the main goal is compile smoke.

## Verification

Local verification for this change is limited to workflow syntax and repository consistency:

- Parse the new workflow YAML.
- Run `git diff --check`.

GitHub is the source of truth for end-to-end verification because the workflow exists specifically to
measure GitHub-runner behavior for the heavy lanes.
