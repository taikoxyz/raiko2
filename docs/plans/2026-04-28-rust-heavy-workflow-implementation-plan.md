# Manual Rust Heavy Workflow Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Convert the manual GitHub Actions heavy workflow into compile-only smoke checks for the
prover, engine, and server stacks on a selected branch without reintroducing them into the required
PR gate.

**Architecture:** Keep the separate `workflow_dispatch` workflow and its input-controlled jobs, but
switch each lane from full `cargo test` to `cargo check --tests`. This preserves bin/test graph
coverage while aligning the workflow with its intended role as a lightweight smoke path outside the
default PR gate.

**Tech Stack:** GitHub Actions, Cargo, `dtolnay/rust-toolchain`, `rui314/setup-mold`

---

### Task 1: Update the workflow design and command scope

**Files:**
- Modify: `docs/plans/2026-04-28-rust-heavy-workflow-design.md`
- Modify: `docs/plans/2026-04-28-rust-heavy-workflow-implementation-plan.md`
- Modify: `.github/workflows/rust-heavy.yml`

**Step 1: Update the design docs**

Revise the existing design and implementation-plan docs to state that `rust-heavy` is now a
compile-only smoke workflow for heavy bin stacks.

**Step 2: Keep the workflow skeleton and inputs**

Retain the existing `workflow_dispatch` interface with `lane` and `ref`, along with the
`resolve-ref` job that defaults `ref` to the dispatching branch.

**Step 3: Keep one job per smoke lane**

Retain the `prover`, `engine`, and `server` lanes, each guarded by the selected `lane` value and
using the same low-optimization compile settings already established in `ci.yml`.

**Step 4: Replace full tests with compile smoke**

Use `cargo check --tests` so the workflow still covers dev-dependencies and test code:

```bash
cargo check -p raiko2-prover -p guest-launcher --tests
cargo check -p raiko2-engine --tests
cargo check -p raiko2 --tests
```

**Step 5: Commit**

```bash
git add .github/workflows/rust-heavy.yml docs/plans/2026-04-28-rust-heavy-workflow-design.md docs/plans/2026-04-28-rust-heavy-workflow-implementation-plan.md
git commit -m "ci: reduce rust-heavy workflow to smoke checks"
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
