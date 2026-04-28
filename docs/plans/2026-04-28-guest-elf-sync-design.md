# Guest ELF Sync Workflow Design

## Goal

Provide a manual GitHub Actions workflow that rebuilds guest ELFs and writes any updated
`crates/guests/elf/*.elf` files back to the selected branch, so operators do not need to download
artifacts and sync ELF files by hand.

## Context

The main CI workflow currently validates Rust checks plus guest buildability. Guest build jobs can
now upload generated ELF artifacts, but artifact download is only a temporary inspection path:

- artifacts are retained for 7 days only
- artifacts are detached from the repository source of truth
- syncing ELFs by hand creates friction and increases mismatch risk

At the same time, automatically committing regenerated ELFs back to a branch can create a second CI
run. That second run is wasteful because an ELF-only commit does not exercise new source code.

## Recommended Approach

Add a dedicated `workflow_dispatch` workflow named `sync-guest-elf` with two operator inputs:

- `backend`: `risc0`, `sp1`, or `all`
- `ref`: branch name to update; defaults to the branch the workflow was dispatched from

The workflow should:

1. check out the selected branch
2. run `just build-guest <backend>`
3. upload the generated ELF files as artifacts for audit/debugging
4. check whether `crates/guests/elf` changed
5. if changed, commit and push the ELF update back to the selected branch
6. if unchanged, exit successfully without creating a commit

To avoid redundant follow-up CI runs, the main `ci.yml` change filters should exclude
`crates/guests/elf/**` from the heavy `rust` and `zk` filters. That keeps the sync workflow as the
single expensive build for an ELF refresh, while preserving normal CI behavior for real source
changes.

## Why This Approach

This keeps the operator workflow simple:

- trigger one workflow
- wait for build
- branch is updated automatically if ELFs changed

It is also safer than relying on artifacts as a long-term source of truth because the synchronized
ELFs remain in git history and flow through normal PR review.

## Alternatives Considered

### 1. Artifact-only sync

Keep the current upload-only behavior and require humans to download artifacts and copy them into
`crates/guests/elf`.

Rejected because it keeps manual steps in the critical path and makes it easy to forget the final
commit.

### 2. Open a separate bot PR

Have CI create a second PR containing ELF updates.

Rejected because it adds noise and splits one logical change into two review units when the desired
behavior is “refresh the current branch in place.”

### 3. Keep strict ELF equality in main CI

Require every zk build job to fail if generated ELFs differ from checked-in ELFs.

Rejected for now because current goals are to stabilize and speed up CI, and strict equality is
better handled by an explicit sync workflow until deterministic ELF policy is settled.

## Workflow Semantics

- Manual only: no automatic trigger on push or PR.
- Branch-scoped: intended for branches in the main repository, not fork sync.
- Idempotent: if no ELF bytes change, the workflow produces no commit.
- Auditable: artifacts are still uploaded with short retention so a run can be inspected without
  checking out the pushed branch immediately.

## Open Risks

- If guest builds are not deterministic, repeated syncs may keep producing different ELFs. The new
  workflow will make that visible faster, but it does not solve determinism by itself.
- Pushing from Actions depends on repository permissions and branch rules. Protected branches should
  continue to reject direct bot pushes if configured that way; the expected target is a feature
  branch.

## Success Criteria

- A maintainer can manually dispatch `sync-guest-elf` on a feature branch.
- The workflow rebuilds the selected backend ELFs.
- Changed ELFs are committed back to that branch automatically.
- The resulting ELF-only sync commit does not trigger the heavy `ci.yml` Rust/zk lanes again.
