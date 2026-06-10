# ZKGas Full Coverage Inventory Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a complete Uzen opcode/precompile coverage inventory and start expanding pure opcode measurement safely.

**Architecture:** Extend `experiments/opcode-gas/opcode_gas.py` with full Uzen table metadata and an
`inventory` subcommand. Keep measurement generation manifest-driven, and only add templates after
tests prove they generate runnable lab inputs.

**Tech Stack:** Python 3.11 standard library, `unittest`, existing SP1 opcode/precompile lab guests.

---

### Task 1: Inventory Model

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Create: `experiments/opcode-gas/tests/test_inventory.py`

**Step 1: Write failing tests**

Add tests that assert:

- the Uzen opcode table includes representative pure, stateful, call, and halting opcodes
- the Uzen precompile table includes all current Uzen precompiles
- `build_inventory(...)` marks existing manifest cases as `measured`
- unmeasured rows are not left unclassified

**Step 2: Run tests**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest experiments/opcode-gas/tests/test_inventory.py
```

Expected: FAIL because the inventory API does not exist.

**Step 3: Implement minimal inventory API**

Add:

- `UzenEntry`
- `InventoryRow`
- full Uzen opcode/precompile metadata
- `build_inventory(manifest)`

**Step 4: Run tests**

Run the same command and expect PASS.

### Task 2: Inventory CLI

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_runner.py`
- Modify: `experiments/opcode-gas/README.md`

**Step 1: Write failing parser test**

Assert the parser accepts:

```bash
inventory --manifest experiments/opcode-gas/manifests/sp1-smoke.toml --out /tmp/inventory
```

**Step 2: Implement CLI**

Write:

- `inventory.json`
- `inventory.md`

**Step 3: Run tests**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover -s experiments/opcode-gas/tests
```

Expected: PASS.

### Task 3: Pure Opcode Template Expansion

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `guests/sp1/src/opcode_lab_impl.rs`
- Modify: `experiments/opcode-gas/manifests/sp1-smoke.toml`
- Modify: `experiments/opcode-gas/tests/test_generate.py`

**Step 1: Add failing generator tests**

Add tests for the next template families:

- unary stack operation
- ternary stack operation
- shift operation
- stack-only `PUSH0`, `DUP1`, `SWAP1`, and `POP`

**Step 2: Implement minimal templates**

Add templates only after tests fail:

- `stack_unary`
- `stack_ternary`
- `stack_shift`
- `stack_only`

**Step 3: Add guest opcode support**

Extend the guest interpreter for the matching opcodes.

**Step 4: Verify**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover -s experiments/opcode-gas/tests
cargo test --manifest-path guests/sp1/Cargo.toml --lib opcode_lab_impl
```

Expected: PASS.

### Task 4: First Full Inventory Run

**Files:**
- Output only under `/tmp/raiko2-opcode-gas/full-inventory`

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python experiments/opcode-gas/opcode_gas.py inventory \
  --manifest experiments/opcode-gas/manifests/sp1-smoke.toml \
  --out /tmp/raiko2-opcode-gas/full-inventory
```

Expected: JSON and Markdown inventory files are written.

### Task 5: Verification And Commit

Run:

```bash
cargo fmt --all
PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover -s experiments/opcode-gas/tests
git diff --check
```

Commit:

```bash
git add docs/plans/2026-06-08-zkgas-full-coverage-inventory-design.md \
  docs/plans/2026-06-08-zkgas-full-coverage-inventory-implementation-plan.md \
  experiments/opcode-gas
git commit -m "feat: add zkgas full coverage inventory"
```
