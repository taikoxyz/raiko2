# CI ZK Build Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Update GitHub Actions CI so it runs the existing Rust checks plus separate `risc0` and `sp1` guest build verification, while leaving a documented TODO for future native fixture or regression coverage.

**Architecture:** Keep the current `paths-filter` front door, preserve the existing Rust test job, and add two independent guest build jobs keyed off a dedicated zk-related change filter. This keeps the workflow easy to read and makes guest backend failures immediately attributable.

**Tech Stack:** GitHub Actions, `dorny/paths-filter`, Rust workspace commands, `just`, existing guest build tooling in `xtask`

---

### Task 1: Expand CI change detection for zk-related paths

**Files:**
- Modify: `.github/workflows/ci.yml`
- Reference: `docs/plans/2026-04-27-ci-zk-build-design.md`

**Step 1: Add a dedicated zk output from the `changes` job**

Add a second filter output named `zk` alongside the existing `rust` output.

**Step 2: Include the right paths in the new zk filter**

Cover at least:

- `guests/**`
- `crates/guests/**`
- `xtask/**`
- `docker/**`
- `justfile`
- `.github/workflows/ci.yml`

**Step 3: Keep the Rust filter aligned with existing behavior**

Preserve the current Rust-related paths unless there is a clear reason to tighten or widen them.

**Step 4: Verify workflow syntax mentally against GitHub Actions structure**

Expected: `changes.outputs.zk` is available to downstream jobs.

**Step 5: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add zk build change detection"
```

### Task 2: Preserve the Rust CI job as the all-test lane

**Files:**
- Modify: `.github/workflows/ci.yml`

**Step 1: Keep the existing Rust job name and command sequence unless a rename is needed for clarity**

Retain:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace -- -D warnings`
- `cargo nextest run --workspace`

**Step 2: Keep the Rust job gated by Rust changes or manual dispatch**

Expected: no behavioral regression for ordinary Rust PRs.

**Step 3: Re-read the job after edits to ensure no accidental dependency on the new zk jobs**

Expected: Rust tests stay independently runnable.

**Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: preserve rust validation lane"
```

### Task 3: Add separate guest build jobs for `risc0` and `sp1`

**Files:**
- Modify: `.github/workflows/ci.yml`
- Reference: `justfile`

**Step 1: Add a `build-guest-risc0` job**

Run:

```bash
just build-guest risc0
```

**Step 2: Add a `build-guest-sp1` job**

Run:

```bash
just build-guest sp1
```

**Step 3: Gate both jobs on zk changes or manual dispatch**

Expected: guest-only changes trigger zk builds even when Rust tests are skipped.

**Step 4: Reuse the shared Rust toolchain setup where needed**

Install what the guest build commands need, but do not overfit the job with unrelated rollout or
deployment tooling.

**Step 5: Keep the jobs parallel and failure-localized**

Expected: one backend can fail without obscuring the other backend's result.

**Step 6: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add guest build lanes"
```

### Task 4: Leave a future native fixture or regression hook

**Files:**
- Modify: `.github/workflows/ci.yml`

**Step 1: Add a short TODO comment block in the workflow**

Document that a future CI job may run:

- a native fixture smoke path, or
- a deterministic regression harness path

**Step 2: Keep the TODO close to the guest build jobs**

Expected: future readers can see where native execution belongs.

**Step 3: Do not add a disabled or fake job**

Only leave documentation-level intent in this patch.

**Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "docs(ci): note future native smoke coverage"
```

### Task 5: Verify the workflow and report results

**Files:**
- Modify: `.github/workflows/ci.yml`

**Step 1: Run a focused workflow syntax and repository sanity pass**

Run:

```bash
cargo test -p xtask release_image
```

This confirms the repo still passes the already-touched targeted xtask tests after workflow edits.

**Step 2: Inspect the rendered workflow file**

Run:

```bash
sed -n '1,260p' .github/workflows/ci.yml
```

Expected: job structure is readable, triggers are correct, and no removed rollout concepts appear.

**Step 3: Report exactly what changed and what was verified**

Include:

- Rust lane behavior
- new `risc0` guest build lane
- new `sp1` guest build lane
- TODO for future native fixture or regression coverage

**Step 4: Final commit**

```bash
git add .github/workflows/ci.yml docs/plans/2026-04-27-ci-zk-build-design.md docs/plans/2026-04-27-ci-zk-build-implementation-plan.md
git commit -m "ci: expand workflow for guest build coverage"
```
