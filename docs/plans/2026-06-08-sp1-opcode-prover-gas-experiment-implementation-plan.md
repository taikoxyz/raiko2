# SP1 Opcode Prover Gas Experiment Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a repeatable SP1 opcode/precompile prover-gas experiment suite that generates synthetic cases, runs cached local execute, fits marginal coefficients, and compares them with the alethia-reth Uzen table.

**Architecture:** Add an experiment-only Python CLI under `experiments/opcode-gas/`. It generates synthetic case manifests and fixtures, runs `target/release/guest-launcher` directly with `--proof-type sp1 --mode execute --sp1-prover local`, then fits per-case marginal slopes and exports reports. V1 keeps the output compatible with the current one-dimensional Uzen table and records warm/cold or argument-dependent dimensions as TODO metadata.

**Tech Stack:** Python 3 standard library, TOML via Python 3.11 `tomllib`, JSON/JSONL reports, existing `target/release/guest-launcher`, existing SP1 local execute report fields.

---

### Task 1: Experiment Directory And Manifest Parser

**Files:**
- Create: `experiments/opcode-gas/README.md`
- Create: `experiments/opcode-gas/opcode_gas.py`
- Create: `experiments/opcode-gas/manifests/sp1-smoke.toml`
- Create: `experiments/opcode-gas/tests/test_manifest.py`

**Step 1: Write the failing test**

Create `experiments/opcode-gas/tests/test_manifest.py`:

```python
import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


class ManifestTests(unittest.TestCase):
    def test_load_manifest_parses_smoke_cases(self):
        manifest = opcode_gas.load_manifest(
            ROOT / "experiments" / "opcode-gas" / "manifests" / "sp1-smoke.toml"
        )

        self.assertEqual(manifest.name, "sp1-smoke")
        self.assertEqual(manifest.backend, "sp1")
        self.assertEqual(manifest.variants, [0, 1, 2, 4])
        self.assertGreaterEqual(
            {case.name for case in manifest.cases},
            {"add", "mul", "keccak256"},
        )
```

**Step 2: Run test to verify it fails**

Run:

```bash
python3 -m unittest discover -s experiments/opcode-gas/tests
```

Expected: FAIL because `opcode_gas.py` does not exist.

**Step 3: Add minimal implementation**

Implement:

- `Manifest` dataclass with `name`, `backend`, `variants`, `cases`
- `CaseSpec` dataclass with `name`, `opcode`, `scenario`, `template`, `target_raw_gas`
- `load_manifest(path)` using `tomllib`
- CLI skeleton with subcommands `generate`, `run`, `fit`

Create `sp1-smoke.toml`:

```toml
name = "sp1-smoke"
backend = "sp1"
variants = [0, 1, 2, 4]

[[cases]]
name = "add"
opcode = "0x01"
scenario = "arithmetic"
template = "stack_binary"
target_raw_gas = 3

[[cases]]
name = "mul"
opcode = "0x02"
scenario = "arithmetic"
template = "stack_binary"
target_raw_gas = 5

[[cases]]
name = "keccak256"
opcode = "0x20"
scenario = "memory"
template = "keccak_32"
target_raw_gas = 36
```

**Step 4: Run test to verify it passes**

Run:

```bash
python3 -m unittest discover -s experiments/opcode-gas/tests
```

Expected: PASS.

**Step 5: Commit**

```bash
git add experiments/opcode-gas
git commit -m "feat(experiments): add opcode gas manifest parser"
```

### Task 2: Synthetic Bytecode Generator

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Create: `experiments/opcode-gas/tests/test_generate.py`

**Step 1: Write failing tests**

Test that `build_bytecode(case, count)` keeps helper structure deterministic and only varies the
target opcode count:

```python
import unittest


class GenerateTests(unittest.TestCase):
    def test_stack_binary_variant_increases_only_target_opcode_count(self):
        case = opcode_gas.CaseSpec(
            name="add",
            opcode=0x01,
            scenario="arithmetic",
            template="stack_binary",
            target_raw_gas=3,
        )

        zero = opcode_gas.build_bytecode(case, 0)
        four = opcode_gas.build_bytecode(case, 4)

        self.assertEqual(zero.opcode_counts.get(0x01, 0), 0)
        self.assertEqual(four.opcode_counts[0x01], 4)
        self.assertTrue(zero.bytes_hex.endswith("00"))
        self.assertTrue(four.bytes_hex.endswith("00"))
```

**Step 2: Run test to verify it fails**

Run:

```bash
python3 -m unittest discover -s experiments/opcode-gas/tests
```

Expected: FAIL because generator helpers do not exist.

**Step 3: Implement minimal generator**

Add:

- `GeneratedBytecode(bytes_hex, opcode_counts)`
- template `stack_binary` for arithmetic and comparison opcodes
- template `keccak_32` for `KECCAK256` with fixed 32-byte memory input
- opcode count extraction

Use simple raw EVM bytecode. Keep the first version deliberately small.

**Step 4: Run tests**

Run:

```bash
python3 -m unittest discover -s experiments/opcode-gas/tests
```

Expected: PASS.

**Step 5: Commit**

```bash
git add experiments/opcode-gas
git commit -m "feat(experiments): generate opcode gas bytecode variants"
```

### Task 3: Fixture Emitter

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Create: `experiments/opcode-gas/tests/test_fixture_emit.py`

**Step 1: Write failing test**

Test that `generate` emits one case metadata JSON per manifest case and variant:

```python
import pathlib
import tempfile
import unittest


class FixtureEmitTests(unittest.TestCase):
    def test_generate_writes_case_metadata(self):
        manifest = opcode_gas.load_manifest(
            ROOT / "experiments" / "opcode-gas" / "manifests" / "sp1-smoke.toml"
        )

        with tempfile.TemporaryDirectory() as tmp:
            out_dir = pathlib.Path(tmp)
            opcode_gas.generate_cases(manifest, out_dir)
            cases = sorted(out_dir.glob("**/case.json"))

        self.assertEqual(len(cases), len(manifest.cases) * len(manifest.variants))
```

**Step 2: Run test to verify it fails**

Run:

```bash
python3 -m unittest discover -s experiments/opcode-gas/tests
```

Expected: FAIL because `generate_cases` does not exist.

**Step 3: Implement fixture emitter**

Emit:

- `case.json` with metadata, bytecode, opcode counts, and target count
- placeholder `guest-input.json` only if a minimal lab GuestInput builder is available

If a full raiko2 `GuestInput` cannot be generated without additional Rust support, stop at metadata
and record `guest_input_status = "pending_lab_guest_builder"` in `case.json`.

**Step 4: Run tests**

Run:

```bash
python3 -m unittest discover -s experiments/opcode-gas/tests
```

Expected: PASS.

**Step 5: Commit**

```bash
git add experiments/opcode-gas
git commit -m "feat(experiments): emit opcode gas case metadata"
```

### Task 4: Direct Guest Launcher Runner

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Create: `experiments/opcode-gas/tests/test_runner.py`

**Step 1: Write failing test**

Mock subprocess and assert the runner calls `target/release/guest-launcher` directly:

```python
import pathlib
import tempfile
import unittest
from unittest import mock


class RunnerTests(unittest.TestCase):
    def test_runner_uses_guest_launcher_directly(self):
        calls = []

        def fake_run(cmd, check):
            calls.append(cmd)

        with tempfile.TemporaryDirectory() as tmp:
            report_path = pathlib.Path(tmp) / "report.json"
            with mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                opcode_gas.run_guest_input(
                    guest_launcher=pathlib.Path("target/release/guest-launcher"),
                    input_path=pathlib.Path("/tmp/input.json"),
                    json_out=report_path,
                )

        self.assertEqual(calls[0][0], "target/release/guest-launcher")
        self.assertIn("--sp1-prover", calls[0])
        self.assertIn("local", calls[0])
        self.assertNotIn("cargo", calls[0][0])
```

**Step 2: Run test to verify it fails**

Run:

```bash
python3 -m unittest discover -s experiments/opcode-gas/tests
```

Expected: FAIL because runner does not exist.

**Step 3: Implement runner**

Add `run_guest_input()`:

```bash
target/release/guest-launcher \
  --proof-type sp1 \
  --mode execute \
  --sp1-prover local \
  --input <guest-input.json> \
  --json-out <report.json>
```

The runner must refuse to run if the executable is missing.

**Step 4: Run tests**

Run:

```bash
python3 -m unittest discover -s experiments/opcode-gas/tests
```

Expected: PASS.

**Step 5: Commit**

```bash
git add experiments/opcode-gas
git commit -m "feat(experiments): run opcode gas cases with direct guest launcher"
```

### Task 5: Fit And Report

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Create: `experiments/opcode-gas/tests/test_fit.py`

**Step 1: Write failing test**

Test ordinary least squares on synthetic raw-run data:

```python
import unittest


class FitTests(unittest.TestCase):
    def test_fit_linear_slope_from_raw_runs(self):
        runs = [
            {"case": "add", "target_count": 0, "target_raw_gas": 3, "prover_gas": 100},
            {"case": "add", "target_count": 1, "target_raw_gas": 3, "prover_gas": 130},
            {"case": "add", "target_count": 2, "target_raw_gas": 3, "prover_gas": 160},
        ]

        fit = opcode_gas.fit_case(runs)

        self.assertEqual(fit.slope_per_operation, 30)
        self.assertEqual(fit.slope_per_raw_gas, 10)
        self.assertEqual(fit.r2, 1)
```

**Step 2: Run test to verify it fails**

Run:

```bash
python3 -m unittest discover -s experiments/opcode-gas/tests
```

Expected: FAIL because fit helpers do not exist.

**Step 3: Implement fit**

Add:

- `fit_case(runs)`
- `fit_report(raw_runs_path, out_dir)`
- `fit.json`
- `coefficients.json`
- `uzen-vs-fit.md`

Use no external dependencies.

**Step 4: Run tests**

Run:

```bash
python3 -m unittest discover -s experiments/opcode-gas/tests
```

Expected: PASS.

**Step 5: Commit**

```bash
git add experiments/opcode-gas
git commit -m "feat(experiments): fit opcode prover gas coefficients"
```

### Task 6: Documentation And Smoke Validation

**Files:**
- Modify: `experiments/opcode-gas/README.md`
- Modify: `docs/development.md`

**Step 1: Document canonical V1 workflow**

Add commands for:

- generating smoke cases
- running direct guest-launcher local execute
- fitting reports
- comparing with alethia-reth Uzen table

Explicitly state:

- no network prover
- no live L1/L2 RPC during run/fit
- warm/cold and argument-dependent dimensions are TODOs
- current export remains compatible with the one-dimensional Uzen table

**Step 2: Run tests**

Run:

```bash
python3 -m unittest discover -s experiments/opcode-gas/tests
git diff --check
```

Expected: PASS and no whitespace errors.

**Step 3: Run a no-execute smoke**

Run:

```bash
python3 experiments/opcode-gas/opcode_gas.py generate \
  --manifest experiments/opcode-gas/manifests/sp1-smoke.toml \
  --out /tmp/raiko2-opcode-gas/fixtures

find /tmp/raiko2-opcode-gas/fixtures -name case.json | wc -l
```

Expected: count equals `len(cases) * len(variants)`.

**Step 4: Commit**

```bash
git add experiments/opcode-gas docs/development.md
git commit -m "docs(experiments): document opcode prover gas suite"
```

## Notes For Later Tasks

- Add a Rust lab GuestInput builder if Python metadata-only generation cannot create valid
  raiko2 `GuestInput` fixtures.
- Add mixed-op synthetic validation blocks after the single-op smoke suite is stable.
- Add real-proposal validation using cached preflight GuestInputs only after synthetic fit works.
- Add warm/cold and argument-dependent sweeps only when the export model can represent them.
