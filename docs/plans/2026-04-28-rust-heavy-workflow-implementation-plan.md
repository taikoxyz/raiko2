# Manual Rust Heavy Workflow Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a manual GitHub Actions workflow that runs the heavy prover, engine, and server Rust
test lanes on a selected branch without reintroducing them into the required PR gate.

**Architecture:** Create a separate `workflow_dispatch` workflow that mirrors the current Rust CI
setup and exposes one input-controlled job per heavy lane. Keep the implementation isolated from the
default `ci.yml` so PR-required checks do not change.

**Tech Stack:** GitHub Actions, Cargo, `dtolnay/rust-toolchain`, `Swatinem/rust-cache`,
`mozilla-actions/sccache-action`, `rui314/setup-mold`

---

### Task 1: Add the design and workflow files

**Files:**
- Modify: `.github/workflows/`
- Create: `docs/plans/2026-04-28-rust-heavy-workflow-design.md`
- Create: `docs/plans/2026-04-28-rust-heavy-workflow-implementation-plan.md`
- Create: `.github/workflows/rust-heavy.yml`

**Step 1: Write the workflow skeleton**

Create a `workflow_dispatch` workflow with `lane` and `ref` inputs plus read-only repository
permissions.

**Step 2: Add target-ref resolution**

Resolve `inputs.ref` and fall back to `github.ref_name` so dispatching from a branch can target that
branch without requiring a second edit.

**Step 3: Add one job per heavy lane**

Implement `test-prover`, `test-engine`, and `test-server` jobs, each guarded by the selected `lane`
value and using the same Rust CI tuning already established in `ci.yml`.

**Step 4: Run the exact heavy test commands**

Use full `cargo test` commands:

```bash
cargo test -p raiko2-prover -p guest-launcher
cargo test -p raiko2-engine
cargo test -p raiko2
```

**Step 5: Commit**

```bash
git add .github/workflows/rust-heavy.yml docs/plans/2026-04-28-rust-heavy-workflow-design.md docs/plans/2026-04-28-rust-heavy-workflow-implementation-plan.md
git commit -m "ci: add manual rust-heavy workflow"
```

### Task 2: Verify workflow integrity

**Files:**
- Test: `.github/workflows/rust-heavy.yml`

**Step 1: Parse workflow YAML**

Run:

```bash
python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/rust-heavy.yml"))'
```

Expected: no output and exit code `0`.

**Step 2: Check repository diff formatting**

Run:

```bash
git diff --check
```

Expected: no output and exit code `0`.

**Step 3: Push**

```bash
git push origin feat/sync-guest-elf
```
