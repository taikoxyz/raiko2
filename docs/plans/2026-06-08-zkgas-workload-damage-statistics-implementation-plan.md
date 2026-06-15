# ZKGas Workload Damage Statistics Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a small statistics report that turns `experiments/opcode-gas` fit results into an
eth-limit damage frontier and current-Uzen containment report.

**Architecture:** Extend `experiments/opcode-gas/opcode_gas.py` with a `damage` subcommand. The
subcommand reads fit JSON plus the manifest metadata, uses built-in current-Uzen smoke multipliers for
the measured cases, and writes JSON plus Markdown reports.

**Tech Stack:** Python 3.11 standard library, existing `unittest` test suite, existing SP1 smoke fit
outputs.

---

### Task 1: Add Damage Model Unit Tests

**Files:**
- Modify: `experiments/opcode-gas/tests/test_fit.py`

**Step 1: Write failing tests**

Add tests for:

- computing eth-only damage from `L_eth`, raw gas, and fitted workload
- computing candidate damage from `L_eth`, `L_zk`, raw gas, measured workload, and multiplier
- classifying the binding resource as `eth` or `zkgas`

**Step 2: Run tests to verify failure**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
  experiments.opcode-gas.tests.test_fit
```

Expected: import or attribute failure because the damage helper does not exist.

### Task 2: Implement Damage Helpers

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`

**Step 1: Add data structures**

Add a `DamageResult` dataclass with:

- `case`
- `kind`
- `eth_gas_per_unit`
- `measured_workload_per_unit`
- `damage_ratio`
- `eth_only_units`
- `eth_only_damage`
- `zkgas_multiplier`
- `zkgas_per_unit`
- `candidate_units`
- `candidate_damage`
- `attack_reduction`
- `binding_resource`

**Step 2: Add pure helper**

Add `compute_damage_result(...) -> DamageResult`.

**Step 3: Run tests**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
  experiments.opcode-gas.tests.test_fit
```

Expected: PASS.

### Task 3: Add Damage CLI

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_runner.py`

**Step 1: Write CLI tests**

Add a test that invokes parser construction and verifies the `damage` command accepts:

- `--fit`
- `--manifest`
- `--eth-gas-limit`
- `--zk-gas-limit`
- `--out`

**Step 2: Implement CLI**

The command should:

1. Load `fit.json`.
2. Load the manifest to recover `kind`, opcode/address, and `target_raw_gas`.
3. Map smoke cases to current-Uzen multipliers.
4. Write:
   - `damage.json`
   - `damage.md`

**Step 3: Run Python tests**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover -s experiments/opcode-gas/tests
```

Expected: all tests pass.

### Task 4: Run First Smoke Damage Report

**Files:**
- Output only under `/tmp/raiko2-opcode-gas/damage-smoke`

**Step 1: Ensure smoke fit exists**

Use the latest available fit, or regenerate with:

```bash
PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit \
  --runs /tmp/raiko2-opcode-gas/precompile-smoke-raw-runs.jsonl \
  --out /tmp/raiko2-opcode-gas/precompile-smoke-report
```

**Step 2: Generate damage report**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python experiments/opcode-gas/opcode_gas.py damage \
  --fit /tmp/raiko2-opcode-gas/precompile-smoke-report/fit.json \
  --manifest experiments/opcode-gas/manifests/sp1-smoke.toml \
  --eth-gas-limit 30000000 \
  --zk-gas-limit 100000000 \
  --out /tmp/raiko2-opcode-gas/damage-smoke
```

Expected: report files are written.

### Task 5: Verify and Commit

**Files:**
- Modified files from Tasks 1-3.

**Step 1: Run checks**

Run:

```bash
cargo fmt --all
PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover -s experiments/opcode-gas/tests
git diff --check
```

**Step 2: Commit**

Run:

```bash
git add experiments/opcode-gas/opcode_gas.py experiments/opcode-gas/tests/test_fit.py \
  experiments/opcode-gas/tests/test_runner.py \
  docs/plans/2026-06-08-zkgas-workload-damage-statistics-implementation-plan.md
git commit -m "feat: add zkgas damage statistics report"
```
