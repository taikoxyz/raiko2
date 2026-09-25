# Hierarchical zkGas Core Opcode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce a content-addressed, replayable SP1 core-opcode submodel with nonnegative static
opcode bodies, separated common dispatch, typed dynamic evaluation, and complete EXP input-domain
coverage.

**Architecture:** Extend the existing exact Decimal calibration math with an active-set
nonnegative least-squares fit for the 102-key relation system, then serialize the accepted static
and structured-dynamic evidence into a small typed registry. Keep the registry in a focused Python
module so fitting, evaluation, and seal-readiness checks do not further enlarge `opcode_gas.py`.
This phase deliberately emits a non-candidate `core_opcode_submodel`; the remaining named opcode
families and higher estimator layers require later plans.

**Tech Stack:** Python 3 from `~/.venv`, `Decimal`, `Fraction`, `dataclasses`, existing unittest
suite, existing SP1 gas-estimator launcher and controlled relation fixtures.

**Spec:** `docs/superpowers/specs/2026-09-25-hierarchical-zkgas-estimator-design.md`

## Global Constraints

- V1 measures SP1 `proverGas`; do not change Alethia's production schedule, runtime limit, integer
  zkGas scale, Boundless configuration, or proposal code.
- Proposal-purpose rows never participate in fitting, feature selection, thresholds, or repair.
- The static body fit and every artifact calculation use the existing isolated 80-digit Decimal
  context; no binary floating point enters persisted evidence.
- Fit production MAPE must be at most `0.05`; fit and holdout maximum production APE must each be at
  most `0.10`.
- A signed relative relation may have a negative observed slope. An exact-flat zero-slope relation
  is checked by `abs(predicted) / max(sum(abs(term)), 1) <= 1e-30`, not by APE.
- Common dispatch, body scales, shared-memory costs, and final event predictions are physically
  additive and nonnegative. Contrast or branch-adjustment coefficients may be signed, but every
  in-domain event prediction must be nonnegative.
- EXP exponent byte length zero is mandatory controlled coverage before dynamic evidence can become
  core-submodel eligible.
- The existing accepted 102-key artifacts are evidence only. This plan may emit
  `supported_core_submodel`, never a full opcode-layer or proposal candidate.
- All Python commands use `~/.venv/bin/python`.
- Every behavior change follows RED, GREEN, REFACTOR; the implementer records the observed failing
  assertion before production code is added.

---

### Task 1: Add Nonnegative Static Opcode Fitting

**Files:**
- Modify: `experiments/opcode-gas/calibration_model.py`
- Modify: `experiments/opcode-gas/tests/test_calibration_model.py`

**Interfaces:**
- Consumes: accepted `RelationEquation` values, the ordered opcode key set, and positive anchor
  body costs keyed by the four natural anchors.
- Produces: `NonnegativeOpcodeFitEvidence` and
  `fit_nonnegative_opcode_bodies(equations, opcode_keys, anchor_body_costs)`.

- [ ] **Step 1: Write failing active-set solver tests**

Add literal, independently calculated tests. The first catches an implementation that simply
clamps the unconstrained solution; the second catches an implementation that changes the fixed
anchors:

```python
def test_nonnegative_least_squares_refits_after_activating_zero_bound(self):
    result = nonnegative_decimal_least_squares(
        (
            (Fraction(1), Fraction(0)),
            (Fraction(0), Fraction(1)),
            (Fraction(1), Fraction(1)),
        ),
        (Decimal("1"), Decimal("-1"), Decimal("0")),
    )

    self.assertEqual(result.solution, (Decimal("0.5"), Decimal("0")))
    self.assertEqual(result.active_zero_indices, (1,))


def test_nonnegative_opcode_fit_preserves_fixed_anchor_body_cost(self):
    result = fit_nonnegative_opcode_bodies(
        equations=(
            RelationEquation(
                relation_id="target-minus-anchor",
                coefficients={
                    "opcode:0x01": Fraction(3),
                    "opcode:0x50": Fraction(-2),
                },
                slope=Decimal("9"),
            ),
        ),
        opcode_keys=("opcode:0x01", "opcode:0x50"),
        anchor_body_costs={"opcode:0x50": Decimal("6")},
    )

    self.assertEqual(result.lab_body_per_raw_gas["opcode:0x50"], Decimal("3"))
    self.assertEqual(result.lab_body_per_raw_gas["opcode:0x01"], Decimal("5"))
```

Add a third test with coefficients `{-3 * DUP1, +3 * DUP2}` and observed slope zero. It must pass
with equal fitted bodies and report zero `flat_relation_max_normalized_error`. Add an inconsistent
fixed-anchor zero relation using unequal positive POP and PUSH0 anchor multipliers; it must return
`not_supported` for the exact-flat gate rather than divide by zero.

- [ ] **Step 2: Run the new tests and verify RED**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_calibration_model.py
```

Expected: import or attribute failure for `nonnegative_decimal_least_squares` and
`fit_nonnegative_opcode_bodies`; existing tests remain runnable.

- [ ] **Step 3: Implement the Decimal active-set solver and evidence type**

Add these public shapes:

```python
@dataclass(frozen=True)
class NonnegativeLeastSquaresResult:
    solution: tuple[Decimal, ...]
    active_zero_indices: tuple[int, ...]
    residual: Decimal


@dataclass(frozen=True)
class NonnegativeOpcodeFitEvidence:
    status: str
    opcode_keys: tuple[str, ...]
    anchor_keys: tuple[str, ...]
    lab_body_per_raw_gas: Mapping[str, Decimal]
    active_zero_keys: tuple[str, ...]
    nonzero_relation_mape: Decimal
    nonzero_relation_max_ape: Decimal
    flat_relation_max_normalized_error: Decimal
    residual: Decimal
    predictions: Mapping[str, Mapping[str, Decimal | str]]
    quality_failures: tuple[str, ...]
```

Implement Lawson-Hanson active-set iteration using `_scaled_decimal_least_squares` for each passive
set. Start at zero, move the largest positive dual coordinate into the passive set, step back to the
nearest zero boundary whenever a passive solution becomes nonpositive, and terminate only when all
inactive dual coordinates are nonpositive within the isolated Decimal context. Reject empty,
ragged, non-finite, or rank-deficient inputs and cap iterations at `30 * column_count` with an
explicit error.

`fit_nonnegative_opcode_bodies` must:

1. convert each positive anchor body cost to lab cost per raw gas with `_ANCHOR_RAW_GAS`;
2. subtract fixed-anchor contributions from every relation target;
3. fit only non-anchor columns under the zero lower bound;
4. reconstruct each signed relation prediction, retaining negative relative slopes as valid data;
5. for nonzero observations calculate
   `APE = abs(predicted - observed) / abs(observed)`;
6. for a zero observation require `abs(predicted) / max(sum(abs(term)), 1) <= 1e-30` and record the
   normalized error separately; and
7. report `supported` only when nonzero-relation MAPE is at most `0.05`, nonzero-relation maximum
   APE is at most `0.10`, and the exact-flat gate passes.

- [ ] **Step 4: Run focused and full model tests and verify GREEN**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_calibration_model.py
~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
```

Expected: all tests pass; the new solver tests name zero-bound refitting and fixed-anchor behavior.

- [ ] **Step 5: Commit Task 1**

```bash
git add experiments/opcode-gas/calibration_model.py \
  experiments/opcode-gas/tests/test_calibration_model.py
git commit -m "feat(zkgas): fit nonnegative opcode bodies"
```

### Task 2: Add The Typed Core Opcode Registry And Evaluator

**Files:**
- Create: `experiments/opcode-gas/hierarchical_model.py`
- Create: `experiments/opcode-gas/tests/test_hierarchical_model.py`

**Interfaces:**
- Consumes: canonical Decimal parameters and one `OpcodeEvent` per executed opcode.
- Produces: `ModelKind`, `ModelSpec`, `OpcodeEvent`, `OpcodeRegistry`,
  `predict_opcode_event`, and `validate_core_registry`.

- [ ] **Step 1: Write failing behavior tests for one-owner evaluation**

Use literal parameters and expectations:

```python
def test_static_opcode_adds_dispatch_exactly_once(self):
    registry = registry_with(
        common_dispatch="7",
        opcode=0x01,
        kind="static_raw_gas",
        params={"body_per_raw_gas": "3"},
    )

    self.assertEqual(
        predict_opcode_event(registry, OpcodeEvent(opcode=0x01, raw_gas=5)),
        Decimal("22"),
    )


def test_keccak_signed_empty_adjustment_must_leave_nonnegative_prediction(self):
    registry = registry_with_keccak(
        common_dispatch="5",
        params={
            "constant": "20",
            "zero_length_event": "-10",
            "permutation": "30",
        },
    )

    self.assertEqual(
        predict_opcode_event(registry, OpcodeEvent(opcode=0x20, input_length=0)),
        Decimal("15"),
    )


def test_registry_rejects_negative_in_domain_event_prediction(self):
    registry = registry_with_exp(
        common_dispatch="5",
        params={"constant": "-10", "exponent_byte": "2", "exponent_byte_sq": "0"},
    )

    with self.assertRaisesRegex(ValueError, "negative opcode prediction"):
        predict_opcode_event(
            registry,
            OpcodeEvent(opcode=0x0A, exponent_byte_length=0),
        )
```

Also test shared memory is charged once, undefined byte values resolve to the single INVALID model,
and a named opcode without a model is reported as unsupported rather than silently using INVALID.

- [ ] **Step 2: Run the new test module and verify RED**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_hierarchical_model.py
```

Expected: module import failure because `hierarchical_model.py` does not exist.

- [ ] **Step 3: Implement the minimal typed registry**

Use frozen dataclasses and Decimal-only parameters:

```python
class ModelKind(str, Enum):
    STATIC_RAW_GAS = "static_raw_gas"
    EXP = "exp"
    KECCAK = "keccak"
    MEMORY_ACCESS = "memory_access"
    MEMORY_COPY = "memory_copy"
    INVALID = "invalid"


@dataclass(frozen=True)
class ModelSpec:
    kind: ModelKind
    parameters: Mapping[str, Decimal]


@dataclass(frozen=True)
class OpcodeEvent:
    opcode: int
    raw_gas: int | None = None
    exponent_byte_length: int | None = None
    input_length: int | None = None
    copy_words: int | None = None
    memory_growth_event: int = 0
    memory_evm_gas_delta: int = 0
    memory_4k_boundary_event: int = 0


@dataclass(frozen=True)
class OpcodeRegistry:
    common_dispatch: Decimal
    models: Mapping[str, ModelSpec]
    opcode_model_ids: tuple[str | None, ...]
    named_opcodes: frozenset[int]
    invalid_model_id: str
    shared_memory_parameters: Mapping[str, Decimal]
```

Require exactly 256 lookup slots. Undefined slots must reference `invalid_model_id`; named but not
yet modeled opcodes must be `None`. Validate every provided feature as a nonnegative integer, derive
KECCAK permutations as zero for empty input and `input_length // 136 + 1` otherwise, and reject
missing or irrelevant dynamic features. Add common dispatch after the model body exactly once, then
reject non-finite or negative final event predictions.

- [ ] **Step 4: Run focused registry tests and the Python suite**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_hierarchical_model.py
~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
```

Expected: all tests pass without mocks.

- [ ] **Step 5: Commit Task 2**

```bash
git add experiments/opcode-gas/hierarchical_model.py \
  experiments/opcode-gas/tests/test_hierarchical_model.py
git commit -m "feat(zkgas): evaluate typed opcode models"
```

### Task 3: Add Mandatory EXP Zero-Exponent Coverage

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_fixture_emit.py`
- Modify: `experiments/opcode-gas/tests/test_sp1_candidate_report.py`

**Interfaces:**
- Consumes: `exponent_byte_length = 0` in the frozen EXP scenario matrix.
- Produces: a valid EXP relation fixture with exponent value zero, target raw gas `10`, and feature
  vector `(constant=1, exponent_bytes=0, exponent_bytes_squared=0)`.

- [ ] **Step 1: Write failing zero-exponent fixture and feature tests**

Add tests that independently assert:

```python
scenario = {"exponent_byte_length": 0, "initial_memory_words": 0}
self.assertEqual(
    opcode_gas._dynamic_relation_target_raw_gas(exp_case, scenario),
    10,
)
self.assertEqual(
    opcode_gas._dynamic_opcode_features("opcode:0x0a", scenario),
    {
        "constant": Fraction(1),
        "exponent_bytes": Fraction(0),
        "exponent_bytes_squared": Fraction(0),
    },
)
```

Generate the relation fixture and assert its target operand is zero, its control/target program
length contract still matches, and exactly one scenario with byte length zero is present in the EXP
matrix with `model_split = "fit"`.

- [ ] **Step 2: Run the two focused modules and verify RED**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_fixture_emit.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py
```

Expected: the current positive-only validation rejects byte length zero.

- [ ] **Step 3: Extend the frozen EXP domain without adding a coefficient**

Add one EXP fit scenario with byte length zero. Accept `0` in
`_dynamic_relation_target_raw_gas`, construct exponent value `0` instead of shifting by a negative
count, and accept zero in `_dynamic_opcode_features`. Keep the current three features and all
existing nonzero holdouts unchanged. Do not introduce a zero-branch coefficient until fresh
evidence shows that the three-term fit fails.

- [ ] **Step 4: Run focused and full Python tests**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_fixture_emit.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py
~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
```

Expected: all tests pass and the frozen manifest identity changes because the scenario matrix
changed.

- [ ] **Step 5: Commit Task 3**

```bash
git add experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/tests/test_fixture_emit.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py
git commit -m "feat(zkgas): cover zero-exponent EXP"
```

### Task 4: Build And Replay The Core Opcode Submodel Artifact

**Files:**
- Modify: `experiments/opcode-gas/hierarchical_model.py`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_hierarchical_model.py`
- Modify: `experiments/opcode-gas/tests/test_sp1_candidate_report.py`
- Modify: `experiments/opcode-gas/README.md`

**Interfaces:**
- Consumes: a validated schema-3 dynamic artifact and its exact accepted relation artifact.
- Produces: `build_core_opcode_submodel_artifact`, `validate_core_opcode_submodel_artifact`, and the
  `build-core-opcode-submodel` CLI command.

- [ ] **Step 1: Write failing artifact construction and replay tests**

Build small literal relation/dynamic fixtures and assert:

```python
artifact = opcode_gas.build_core_opcode_submodel_artifact(
    manifest=manifest,
    relation_artifact=relation_artifact,
    dynamic_artifact=dynamic_artifact,
)
self.assertEqual(artifact["purpose"], "core_opcode_submodel")
self.assertFalse(artifact["candidate_eligible"])
self.assertEqual(artifact["modeled_named_opcode_count"], 102)
self.assertIn("opcode:0x1e", artifact["unsupported_named_opcode_keys"])
self.assertEqual(
    opcode_gas.validate_core_opcode_submodel_artifact(
        manifest, relation_artifact, dynamic_artifact, artifact
    ),
    artifact,
)
```

Add mutation cases for a changed relation hash, changed dynamic hash, negative static body,
different common dispatch, a named opcode mapped to INVALID, missing EXP zero-domain evidence, a
negative in-domain dynamic prediction, and an artifact hash mismatch. Add a CLI parser test for
the exact command name and arguments.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_hierarchical_model.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py
```

Expected: builder, validator, and CLI symbols are missing.

- [ ] **Step 3: Implement content-addressed construction and exact replay**

Construction must:

1. validate `relation_artifact.status == "accepted"`, `dynamic_artifact.schema_version == 3`,
   `dynamic_artifact.status == "supported"`, and exact source-hash agreement;
2. parse anchor body costs and transfer parameters from the dynamic artifact as Decimal;
3. run `fit_nonnegative_opcode_bodies` over all 102 relation keys;
4. multiply lab body coefficients by positive `body_scale`, while storing
   `common_opcode_overhead_per_operation` once as registry dispatch;
5. replace the six structured dynamic opcodes with body-space models from the dynamic artifact,
   sharing one memory model and not reusing their static scalar entries;
6. probe the frozen dynamic scenario domain, including EXP byte length zero, through
   `predict_opcode_event`;
7. map every undefined byte to the one INVALID model and leave the 48 currently unsupported named
   opcodes as `None`;
8. persist ordered registry data, all source hashes, fit evidence, modeled and unsupported coverage,
   `candidate_eligible=false`, and a canonical SHA256.

Validation must independently rebuild the artifact from its sources and require canonical JSON
equality. It must never trust the persisted registry or its digest in isolation.

The CLI accepts:

```text
build-core-opcode-submodel
  --calibration-run <run>
  --controlled-manifest <manifest>
  --relations <run/opcode-relations.json>
  --dynamic-models <run/dynamic-opcode-models.json>
  --out <run/core-opcode-submodel.json>
```

It reuses the existing frozen-run path, identity, and canonical-artifact checks before writing.
After the atomic write, the command must reopen the file, call
`validate_core_opcode_submodel_artifact` with the same validated sources, and fail unless the
replayed canonical payload and digest match exactly.

- [ ] **Step 4: Document and verify the command**

Add the command after `fit-dynamic-opcode-models` in the README and state that the output covers 102
of 150 named Unzen opcodes and cannot open proposal validation.

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_hierarchical_model.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py
~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
git diff --check
```

Expected: all tests and whitespace checks pass.

- [ ] **Step 5: Commit Task 4**

```bash
git add experiments/opcode-gas/hierarchical_model.py \
  experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/tests/test_hierarchical_model.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py \
  experiments/opcode-gas/README.md
git commit -m "feat(zkgas): seal core opcode submodel"
```

### Task 5: Run A Fresh Zero-Exponent Calibration And Record The Result

**Files:**
- Modify: `experiments/opcode-gas/README.md`
- Generated, ignored evidence: `experiments/opcode-gas/runs/<calibration-id>/`

**Interfaces:**
- Consumes: the committed Tasks 1-4 implementation, existing SP1 lab ELFs, and release
  `guest-launcher` with the gas estimator.
- Produces: a fresh schema-3 dynamic artifact and replayable core-opcode submodel bound to one clean
  implementation revision.

- [ ] **Step 1: Verify the committed tree and rebuild the launcher**

Run:

```bash
git status --short
cargo build -r -p guest-launcher --features sp1-sdk/profiling
```

Expected: worktree clean before preparation and launcher build succeeds.

- [ ] **Step 2: Prepare one exact calibration run**

Run the documented `prepare-calibration`, `generate-anchor-probe`, `run-anchor-probe`, and
`fit-anchor-probe` commands using one `RUN_PATH_FILE`. Never discover or substitute the newest run
directory.

Expected: one new immutable calibration identity whose manifest includes the zero-exponent EXP row.

- [ ] **Step 3: Run and fit controlled relation evidence**

Run the documented `generate-relations`, `run-relations`, `fit-relations`,
`run-block-calibration`, and `fit-dynamic-opcode-models` commands for that same directory.

Expected: all terminal relation rows are accepted. A dynamic result of `not_supported` is a valid
experimental outcome and must be preserved without threshold or feature changes.

- [ ] **Step 4: Build and replay the core submodel**

Run:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  build-core-opcode-submodel \
  --calibration-run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --dynamic-models "$CALIBRATION_RUN/dynamic-opcode-models.json" \
  --out "$CALIBRATION_RUN/core-opcode-submodel.json"
```

Then invoke the validator through the same command's readback path and compare its recomputed digest
with the file's digest. Do not build a full candidate or open proposal results.

- [ ] **Step 5: Record observed evidence and commit**

Append the calibration ID, implementation revision, relation acceptance count, static active-zero
keys, static relation MAPE/max APE, EXP zero prediction/APE, dynamic aggregate status, core-submodel
status, and artifact SHA256 to the README. If any gate fails, record its exact failure and stop this
subproject at `not_supported`; do not tune the model in the same run.

Run:

```bash
~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
cargo test -p raiko2-zkgas-trace
cargo test -p guest-launcher --test controlled_workload
git diff --check
```

Commit:

```bash
git add experiments/opcode-gas/README.md
git commit -m "docs(zkgas): record core opcode calibration"
```
