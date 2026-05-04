# Guest ELF Consistency Check Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a stable `guest-elf-consistency` CI job that always rebuilds checked-in guest ELF
artifacts on the self-hosted `raiko2` runner and fails when the repository copy drifts from a clean
rebuild.

**Architecture:** Extend `.github/workflows/ci.yml` with one always-on self-hosted job that
rebuilds both guest backends and checks `crates/guests/elf` with `git diff`. Keep
`.github/workflows/sync-guest-elf.yml` as the manual remediation path referenced by the failure
message.

**Tech Stack:** GitHub Actions, Cargo, `just`, self-hosted Actions runner labels, git diff

---

### Task 1: Document the always-on ELF consistency gate

**Files:**
- Create: `docs/plans/2026-05-04-guest-elf-consistency-design.md`
- Create: `docs/plans/2026-05-04-guest-elf-consistency-implementation-plan.md`

**Step 1: Write the design doc**

Describe the gap left by artifact-only guest builds and explain why the repository needs one stable
required check that always verifies checked-in ELF freshness.

**Step 2: Write the implementation plan**

Capture the exact workflow change, the self-hosted runner target, and the failure guidance that
points authors to `sync-guest-elf`.

### Task 2: Add the stable CI job

**Files:**
- Modify: `.github/workflows/ci.yml`

**Step 1: Add the job skeleton**

Add a `guest-elf-consistency` job that runs on:

```yaml
runs-on: [self-hosted, linux, x64, raiko2]
```

**Step 2: Rebuild both guest stacks**

Use:

```bash
just build-guest risc0
just build-guest sp1
```

**Step 3: Add the drift check**

Use shell logic that fails if `crates/guests/elf` changes and emits a GitHub Actions error message
that tells authors to run `sync-guest-elf`.

**Step 4: Keep the job always present**

Do not hide the job behind paths filters. It should exist on every PR so branch protection can rely
on a single stable check name.

### Task 3: Verify workflow integrity

**Files:**
- Test: `.github/workflows/ci.yml`

**Step 1: Parse workflow YAML**

Run:

```bash
python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/ci.yml"))'
```

Expected: no output and exit code `0`.

**Step 2: Check repository diff formatting**

Run:

```bash
git diff --check
```

Expected: no output and exit code `0`.

**Step 3: Commit**

```bash
git add .github/workflows/ci.yml docs/plans/2026-05-04-guest-elf-consistency-design.md docs/plans/2026-05-04-guest-elf-consistency-implementation-plan.md
git commit -m "ci: add guest elf consistency gate"
```
