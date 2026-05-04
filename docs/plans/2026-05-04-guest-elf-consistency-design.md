# Guest ELF Consistency Check Design

## Goal

Add a stable CI check that always rebuilds the checked-in guest ELF artifacts and fails when the
repository copy drifts from a clean rebuild.

## Context

The repository now has a manual `sync-guest-elf` workflow that can rebuild guest ELFs and push the
result back to a branch. That workflow is useful as a repair tool, but it does not by itself ensure
that every PR keeps `crates/guests/elf` in sync with the current guest sources and build logic.

The previous artifact-only guest build jobs were useful for compile coverage, but they did not
enforce checked-in ELF freshness and duplicated work once an always-on consistency gate existed. A
stable required check should therefore exist in the main CI workflow and always report one of two
states:

- the checked-in ELFs still match a clean rebuild
- drift was detected and the author must run `sync-guest-elf`

## Requirements

- The check must always exist on PRs so branch protection can rely on one stable name.
- The check must run on the `raiko2` self-hosted runner, not on GitHub-hosted runners.
- The check must rebuild both RISC0 and SP1 guest ELFs.
- The check must fail when `crates/guests/elf` differs after the rebuild.
- The failure message must explicitly tell authors to run `sync-guest-elf`.

## Approaches Considered

### 1. Only run ELF equality checks when `crates/guests/elf/**` changes

This saves runner time, but it misses the important stale-ELF case where someone changes
`guests/**`, `xtask/**`, or other guest build inputs without updating the checked-in ELF outputs.

### 2. Always run a stable ELF consistency check

This costs one self-hosted rebuild per PR, but it closes the stale-ELF gap and gives branch
protection a single dependable check name. It also works cleanly with the manual `sync-guest-elf`
workflow: rebuild drift is detected automatically, and the manual workflow is the explicit repair
path.

### 3. Remove checked-in ELFs entirely

That is a larger architectural change and is out of scope for this branch.

## Decision

Use approach 2.

## Workflow Shape

- Keep `sync-guest-elf.yml` as the manual remediation workflow.
- Remove the standalone `build-guest-risc0` and `build-guest-sp1` jobs from `ci.yml`.
- Add one `guest-elf-consistency` job to `ci.yml` as the only guest build gate.
- Run the new job on `runs-on: [self-hosted, linux, x64, raiko2]`.
- Rebuild both guest stacks with:
  - `just build-guest risc0`
  - `just build-guest sp1`
- Fail if `git diff --quiet -- crates/guests/elf` is not clean.

## Failure UX

The job should emit a clear GitHub Actions error message:

`Guest ELF drift detected. Run the sync-guest-elf workflow for this branch and commit the resulting ELF update.`

That keeps the repair path obvious and avoids overloading the default CI workflow with auto-push
behavior.

## Non-Goals

- Do not make the main CI workflow auto-commit ELF updates.
- Do not remove the existing manual `sync-guest-elf` workflow.
- Do not keep redundant build-only guest jobs in the main CI workflow once the consistency gate is in place.
