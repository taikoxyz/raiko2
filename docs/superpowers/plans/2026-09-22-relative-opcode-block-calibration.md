# Relative Opcode And Controlled Block Calibration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce and seal a review-only SP1 proving-cost mapping model whose opcode multipliers are
derived from formal relative measurements, a synthetic four-anchor body-cost prior, and six
parameters calibrated on controlled production-guest blocks: body scale, common per-opcode
interpreter overhead, and four fixed/base costs.

**Architecture:** Keep experiment orchestration, manifests, CLI commands, provenance, and sealing in
`opcode_gas.py`; put exact relation algebra and Decimal block fitting in a new dependency-free
`calibration_model.py`. Extend `guest-launcher` only with host-side controlled block fixture
construction and trace validation. Final proposal rows remain validation-only and cannot enter any
fit.

**Tech Stack:** Python 3.11 standard library (`dataclasses`, `decimal`, `fractions`, `unittest`),
Rust, REVM/Alethia host tracing, SP1 `ExecutionReport::gas()`, TOML/JSON artifacts, Git.

**Spec:** `docs/superpowers/specs/2026-09-22-relative-opcode-block-calibration-design.md`

## Global Constraints

- The primary candidate unit is normalized SP1 `proverGas / actual raw EVM gas`.
- The natural opcode anchors are exactly `POP`, `PUSH0`, `DUP1`, and `SWAP1` in that order.
- The fixed/base vector is exactly `proposal_startup`, `block_base`, `tx_base`, and
  `native_value_transfer` in that order.
- The canonical 102-key raw-gas-weighted relation matrix must have rank 98; removing the four anchor
  columns must leave a full-rank 98-column system.
- `EXP`, `KECCAK256`, `MLOAD`, `MSTORE`, `MSTORE8`, and `MCOPY` retain frozen non-fitting scalar
  holdouts for the candidate boundary and separately frozen fit/holdout rows for their structured
  non-candidate diagnostics.
- Use `Fraction` for relation/rank/basis algebra and `Decimal` with precision 80 for observations,
  fitting, prediction, APE, and serialization. Never convert a candidate value through `float`.
- Candidate fitting consumes controlled fixtures only. `integration_smoke` and `final_validation`
  rows are rejected by every calibration entrypoint.
- Do not update the production multiplier table, block limit, Boundless configuration, production
  guest ELFs, or alethia-reth revision in this plan. Task 3A may rebuild only the dedicated
  diagnostic opcode-lab ELF/VK; proposal, aggregation, precompile, and REVM artifacts must remain
  byte-identical.
- Preserve old calibration directories unchanged. Any implementation, manifest, or artifact change
  requires a new content-addressed calibration identity.
- Use `~/.venv/bin/python` for Python commands. Generated run artifacts remain ignored and
  uncommitted while the experiment is running.

---

### Task 1: Consolidate The Approved Model Into Authoritative Documentation

**Files:**
- Modify: `docs/plans/2026-09-06-zkgas-multiplier-recalibration-experiment-design.md`
- Modify: `docs/plans/2026-06-08-sp1-opcode-prover-gas-experiment-implementation-plan.md`
- Modify: `experiments/opcode-gas/README.md`

**Interfaces:**
- Consumes: approved design spec and the existing candidate/proposal invariants.
- Produces: one authoritative description of relative relation fitting, four-anchor block fitting,
  dynamic raw-gas holdouts, and candidate sealing.

- [ ] **Step 1: Replace the absolute-opcode-first formulas in the authoritative design**

Use the exact model below and remove statements requiring an isolated positive
`g_p(k) / operation` for every pure opcode:

```text
A * mu = d
theta_i = (body_scale * synthetic_body_cost_i + common_opcode_overhead) / raw_gas_i
mu = mu_zero + B * theta

family_slope_j = slope_count(x_j * mu_zero) +
  slope_count(x_j * B * C) * [body_scale, common_opcode_overhead]
mu = mu_zero + B * theta
p_hat_j = x_j * mu + q_j * beta
beta = [proposal_startup, block_base, tx_base, native_value_transfer]
```

State that `A_i` and `x_j` use per-key actual raw-gas totals, not execution counts.

- [ ] **Step 2: Replace the sequential overhead residualization task in the June plan**

Document the new artifact sequence exactly:

```text
opcode-relations.json
block-calibration-rows.jsonl
block-calibration.json
controlled-fit.json                 # precompile and fixed-event results only
candidate/candidate-manifest.json
candidate/candidate.sha256
```

Retain the existing proposal barrier, bridge isolation, provenance, coverage, and ownership rules.

- [ ] **Step 3: Update the operator README commands**

Add the new commands in their required order:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate-relations \
  --manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --calibration-run "$CALIBRATION_RUN" \
  --out "$CALIBRATION_RUN/generated/formal-relations"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-relations \
  --fixtures "$CALIBRATION_RUN/generated/formal-relations" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_revm_opcode_lab.elf \
  --calibration-run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/raw/formal-relations.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-relations \
  --runs "$CALIBRATION_RUN/raw/formal-relations.jsonl" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/opcode-relations.json"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-block-calibration \
  --guest-launcher target/release/guest-launcher \
  --calibration-run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --out "$CALIBRATION_RUN/block-calibration-rows.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-block-calibration \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --runs "$CALIBRATION_RUN/block-calibration-rows.jsonl" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/block-calibration.json"
```

State that `CALIBRATION_RUN` must be the exact directory emitted by `prepare-calibration`; the
README must show the machine-readable `--run-path-file` flow from Task 8 rather than asking the
operator to discover the newest run directory.

Label the older matched-control reports as exploratory predecessors and the old early-STOP opcode
fit as non-candidate.

- [ ] **Step 4: Verify documentation consistency**

Run:

```bash
git diff --check
rg -n "A \* mu|mu_zero|run-block-calibration|fit-block-calibration" \
  docs/plans experiments/opcode-gas/README.md
rg -n "isolation-proven opcode case|p_target\(x\).*opcode" \
  docs/plans/2026-09-06-zkgas-multiplier-recalibration-experiment-design.md \
  docs/plans/2026-06-08-sp1-opcode-prover-gas-experiment-implementation-plan.md
```

Expected: the first two commands succeed; the last command returns no obsolete candidate formula.

- [ ] **Step 5: Commit**

```bash
git add docs/plans/2026-09-06-zkgas-multiplier-recalibration-experiment-design.md \
  docs/plans/2026-06-08-sp1-opcode-prover-gas-experiment-implementation-plan.md \
  experiments/opcode-gas/README.md
git commit -m "docs(zkgas): consolidate relative block calibration"
```

---

### Task 2: Add Exact Relation Algebra As A Focused Python Module

**Files:**
- Create: `experiments/opcode-gas/calibration_model.py`
- Create: `experiments/opcode-gas/tests/test_calibration_model.py`

**Interfaces:**
- Consumes: ordered opcode keys, signed raw-gas coefficient rows, signed Decimal relation slopes,
  and the four ordered anchor keys.
- Produces: `RelationEquation`, `AffineOpcodeModel`, `exact_rank()`,
  `derive_affine_opcode_model()`, and `reconstruct_multipliers()`.

- [ ] **Step 1: Write failing algebra tests**

Create these concrete tests:

```python
import unittest
from decimal import Decimal
from fractions import Fraction

from calibration_model import (
    RelationEquation,
    derive_affine_opcode_model,
    exact_rank,
)


class CalibrationModelTests(unittest.TestCase):
    def test_exact_rank_and_affine_reconstruction(self):
        equations = (
            RelationEquation(
                "add-pop", {"ADD": Fraction(3), "POP": Fraction(-2)}, Decimal("50")
            ),
            RelationEquation(
                "mul-pop", {"MUL": Fraction(5), "POP": Fraction(-2)}, Decimal("170")
            ),
        )
        model = derive_affine_opcode_model(
            opcode_keys=("ADD", "MUL", "POP"),
            equations=equations,
            anchor_keys=("POP",),
        )
        self.assertEqual(model.rank, 2)
        self.assertEqual(model.nullity, 1)
        self.assertEqual(
            model.reconstruct_multipliers({"POP": Decimal("20")}),
            {"ADD": Decimal("30"), "MUL": Decimal("42"), "POP": Decimal("20")},
        )

    def test_rejects_wrong_anchor_basis(self):
        equations = (
            RelationEquation("a-b", {"A": Fraction(1), "B": Fraction(-1)}, Decimal("1")),
        )
        with self.assertRaisesRegex(
            ValueError, "anchor columns do not leave a full-rank system"
        ):
            derive_affine_opcode_model(("A", "B"), equations, ())
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_calibration_model.py -v
```

Expected: import failure because `calibration_model.py` does not exist.

- [ ] **Step 3: Implement exact matrix primitives**

Use these public types and signatures:

```python
@dataclass(frozen=True)
class RelationEquation:
    relation_id: str
    coefficients: Mapping[str, Fraction]
    slope: Decimal


@dataclass(frozen=True)
class AffineOpcodeModel:
    opcode_keys: tuple[str, ...]
    anchor_keys: tuple[str, ...]
    rank: int
    nullity: int
    mu_zero: Mapping[str, Decimal]
    anchor_basis: Mapping[str, Mapping[str, Fraction]]

    def reconstruct_multipliers(
        self, anchor_values: Mapping[str, Decimal]
    ) -> dict[str, Decimal]:
        values = dict(self.mu_zero)
        for opcode_key in self.opcode_keys:
            for anchor_key in self.anchor_keys:
                coefficient = self.anchor_basis[opcode_key][anchor_key]
                values[opcode_key] += (
                    Decimal(coefficient.numerator)
                    / Decimal(coefficient.denominator)
                    * anchor_values[anchor_key]
                )
        return values
```

Implement Gauss-Jordan elimination over `Fraction` for rank, inverse, and `B`; apply the same
rational row operations to Decimal `d` to derive `mu_zero`. Reject duplicate keys, missing
coefficients, non-finite slopes, wrong rank, wrong anchor count, and non-square non-anchor systems.

- [ ] **Step 4: Add live 102-key structural coverage**

In `test_calibration_model.py`, import `opcode_gas`, derive each coefficient row from the current
matched-control target/reference raw-gas totals, and assert:

```python
assert len(opcode_keys) == 102
assert len(nonzero_equations) == 98
assert exact_rank(coefficient_matrix) == 98
assert model.anchor_keys == (
    "opcode:0x50",
    "opcode:0x5f",
    "opcode:0x80",
    "opcode:0x90",
)
```

- [ ] **Step 5: Run focused tests and commit**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_calibration_model.py -v
```

Expected: all tests pass.

```bash
git add experiments/opcode-gas/calibration_model.py \
  experiments/opcode-gas/tests/test_calibration_model.py
git commit -m "feat(zkgas): add exact opcode relation algebra"
```

---

### Task 3: Promote Matched Controls Into A Formal Relation Artifact

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/manifests/sp1-calibration-v1.toml`
- Modify: `experiments/opcode-gas/tests/test_manifest.py`
- Modify: `experiments/opcode-gas/tests/test_fixture_emit.py`
- Modify: `experiments/opcode-gas/tests/test_runner.py`
- Modify: `experiments/opcode-gas/tests/test_sp1_candidate_report.py`

**Interfaces:**
- Consumes: exact matched target/control programs, three repeats, actual per-key raw-gas totals,
  frozen positive-count prefixes, count-zero activation diagnostics, a last-slot count-one
  holdout, and checkpoint mapping. Relation `count` is the active target-microprogram repeat count
  inside the fixed footprint, not transaction count, block count, gas limit, or runtime gas.
- Produces: `opcode-relations.json` with 98 accepted canonical equations, four self-control checks,
  non-fitting dynamic relation observations, exact rank/basis metadata, signed quality gates, and
  provenance.

- [ ] **Step 1: Write failing manifest and CLI tests**

Require these top-level manifest fields:

```toml
opcode_relation_anchors = [
  "opcode:0x50",
  "opcode:0x5f",
  "opcode:0x80",
  "opcode:0x90",
]
dynamic_raw_gas_keys = [
  "opcode:0x0a",
  "opcode:0x20",
  "opcode:0x51",
  "opcode:0x52",
  "opcode:0x53",
  "opcode:0x5e",
]
```

Add tests that reject reordered/missing anchors, an unmarked dynamic template, duplicate relation
IDs, and a relation whose target/reference raw-gas totals do not match its fixture contract. Add
parser tests for `generate-relations`, `run-relations`, `fit-relations`, and the
`prepare-calibration --run-path-file` machine-readable run handoff.

Freeze the complete dynamic model matrix in the formal relation manifest. `EXP`, `KECCAK256`, and
`MCOPY` each have five fit scenarios and two untouched model holdouts. `MLOAD`, `MSTORE`, and
`MSTORE8` each have five fit scenarios, two reused zero-growth warmed holdouts, two retained
previously passing expansion holdouts, and one fresh untouched `0x4000` expansion holdout. The
viewed `0x2000` row is fit/diagnostic evidence. Every key retains exactly one `canonical` relation
for `A`; the other 45 relations remain outside `A`. Bind `model_split` and the exact structured
`relation_scenario` through the manifest, emitted fixtures, raw rows, adaptive decisions, and final
relation artifact.

Run `25db29d91bd3311441fe8317` showed that the linear page-count hypothesis fit at 0.259%
production MAPE and 1.103% maximum APE and passed the `0x0800` and `0x0fe0` holdouts, but
overpredicted `0x2000` by about 42% for all three memory opcodes. Replacing the count post hoc with a
binary boundary event reduced those viewed-row errors to 5.72%, 5.79%, and 5.96%. This observation
motivates the new feature, but it is not validation evidence: `0x2000` moves to fit/diagnostic and a
fresh run must evaluate `0x4000` under the new controlled-manifest identity.

For memory-sensitive scenarios, make `initial_memory_words` executable: the warmup must allocate
the declared memory before the target operation. Tests must show that changing this field changes
the bytecode and raw-gas expectation. A metadata-only initial-memory declaration is invalid because
it cannot identify the memory-growth term.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_manifest.py \
  experiments/opcode-gas/tests/test_fixture_emit.py \
  experiments/opcode-gas/tests/test_runner.py -v
```

Expected: failures for missing manifest fields and commands.

- [ ] **Step 3: Add the formal relation purpose without weakening diagnostics**

Add:

```python
FORMAL_RELATION_PURPOSE = "formal_opcode_relation"
FORMAL_RELATION_SIGNAL_KIND = "signed_raw_gas_relation"
OPCODE_RELATION_ANCHORS = (
    "opcode:0x50",
    "opcode:0x5f",
    "opcode:0x80",
    "opcode:0x90",
)
```

Parameterize fixture generation and pair validation by purpose. Diagnostic commands keep
`matched_control_diagnostic/contextual_relative`; formal commands require three repeats, the formal
purpose/signal kind, the frozen gas-estimator cadence, and the calibration identity.
`prepare-calibration --run-path-file PATH` must atomically write the exact created run directory to
`PATH` after provenance is durable; it must reject a pre-existing non-empty file.

- [ ] **Step 4: Bind actual raw-gas coefficient rows**

For every formal pair, serialize:

```python
{
    "relation_id": pair_id,
    "target_raw_gas_by_key": {"opcode:0x01": "3"},
    "control_raw_gas_by_key": {"opcode:0x50": "2"},
    "signed_raw_gas_by_key": {"opcode:0x01": "3", "opcode:0x50": "-2"},
}
```

Reconstruct these maps from the exact executed program and host trace. Never trust only
`target_raw_gas` metadata. Reject any lane where the trace and reconstructed totals differ.
Before opening SP1 output, require the actual trace to show three distinct positive target raw-gas
totals per dynamic key, with the middle value at least 2x canonical and the largest at least 4x
canonical. A frozen scenario that misses this gate is a manifest-design failure and cannot be
replaced after SP1 output is opened.

- [ ] **Step 5: Implement signed relation fitting**

Create:

```python
def fit_opcode_relations(
    manifest: Manifest,
    rows: Iterable[Mapping[str, Any]],
) -> dict[str, Any]:
```

Apply the frozen cumulative-prefix search, three-repeat determinism, signal, R2,
`se(slope)/abs(slope)`, residual/signal, and out-of-fit signed checkpoint gates. For every non-self
relation, fit only counts `>= 1`; count zero cannot enter slope, intercept quality, R2, standard
error, residual, signal, or checkpoint baselining. Serialize `positive_fit_intercept_p`,
`zero_delta_p`, and signed `activation_gap_p = zero_delta_p - positive_fit_intercept_p`. Serialize
`abs(activation_gap_p) / signal_p` when signal is positive, with explicit finite/zero-signal status
instead of Infinity or NaN.

Generate and execute, before reading results, one `active_tail` sample for every selected relation
round. It executes the same target microprogram exactly once in the last fixed-footprint slot; the
ordinary count-one row executes it in the first slot. Bind placement and sample ID into pair
identity, raw order, completeness, resume, and hashes. When activation-gap ratio is strictly above
`0.02`, require `tail_observed_marginal_p = delta_tail - delta_zero` to have the same sign as
`tail_predicted_marginal_p = slope_p` and APE `<= 0.10`. Both zero passes; one zero or opposite
signs fails. Zero signal plus zero gap does not trigger, while zero signal plus nonzero gap must
trigger with an explicit unavailable-ratio status. Always retain the tail diagnostic.

Keep the guest-visible case stable per lane across every count and placement, leaving count and
placement only in host metadata. Require fixed-bound controls to serialize identically and
prefix-one/tail-one target inputs to differ only in bytecode slot order. Before fitting or replay,
reconstruct the complete canonical case/guest declaration from manifest relation plus bound,
count, placement, lane, and generation formulas; do not take scenario, relation scenario, gas
limit, opcode/count, fixed length, template, operands, maps, or opcode counts from persisted rows.
Recompute fixture, pair, workload, execution-row, and controlled-trace identities. Permit identity
reuse only where exact canonical inputs recur.
Name the static EVM bytecode-count map `evm_opcode_counts` and reserve report `opcode_counts` for the
SP1 RISC-V execution profile. Preserve both in raw rows and reject formal fixture/report key
collisions before merging.

Treat the sealed formal decision ledger as the terminal source of truth. Downstream relation fit,
block-calibration run/fit, candidate build, and candidate/bridge replay must verify its seal, replay
a complete accepted state, derive canonical accepted rows, and require exact byte/order equality
with the final formal JSONL. Bind `formal_relation_decisions_sha256` into candidate provenance.

Canonical non-self relations must produce 98 accepted equations. Non-self exact-flat semantics use
the positive fit counts plus checkpoint, while self-controls retain all-count exact-flat semantics
and do not enter `A`. Dynamic holdouts pass the same signed slope-quality gates and are serialized
as non-fitting observations, but cannot add rows to `A` or affect `mu_zero`/`B`. Call
`derive_affine_opcode_model()` and serialize `mu_zero`, `B`, rank, nullity, and hashes using
canonical Decimal/Fraction strings.

- [ ] **Step 6: Prove failure cases before accepting the artifact**

Add tests for negative accepted slopes, tiny-signal rejection, nonzero repeat noise, wrong sign at
checkpoint, count-zero activation contamination, tail placement/order/completeness, zero-signal
activation behavior, tail sign/APE failure, a missing canonical relation, wrong raw-gas units, rank
97, rounded/tampered basis coefficients, placement relabeling, wrong/missing/reused recomputed
identities, reversed final raw rows, missing/stale decision seals, and candidate formal-decision
digest mismatch.

Keep `experiments/opcode-gas/runs/1bee0a5941fddc9984b009b8` sealed and read-only. Its JUMPI
count-zero/first-slot/last-slot result and positive-count linear fit are failed-run design evidence,
not campaign input and not a candidate artifact.

- [ ] **Step 7: Run the relation test lane and commit**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_manifest.py \
  experiments/opcode-gas/tests/test_fixture_emit.py \
  experiments/opcode-gas/tests/test_runner.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py -v
```

Expected: all tests pass.

```bash
git add experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  experiments/opcode-gas/tests/test_manifest.py \
  experiments/opcode-gas/tests/test_fixture_emit.py \
  experiments/opcode-gas/tests/test_runner.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py
git commit -m "feat(zkgas): formalize opcode relation fitting"
```

---

### Task 3A: Add The Synthetic Four-Anchor Ratio Probe

**Files:**
- Modify: `guests/sp1/src/opcode_lab.rs`
- Create: `guests/sp1/src/opcode_anchor_probe.rs`
- Create: `guests/sp1/tests/opcode_anchor_probe.rs`
- Modify: `bin/guest-launcher/src/main.rs`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Test: `guests/sp1/tests/opcode_anchor_probe.rs`
- Test: `bin/guest-launcher/src/main.rs`
- Test: `experiments/opcode-gas/tests/test_fixture_emit.py`
- Test: `experiments/opcode-gas/tests/test_runner.py`
- Test: `experiments/opcode-gas/tests/test_sp1_candidate_report.py`

**Interfaces:**
- Consumes: the synthetic `sp1-opcode-lab` guest, canonical gas-estimator cadence, and the four
  ordered natural anchors.
- Produces: `generate-anchor-probe`, `run-anchor-probe`, and `fit-anchor-probe`; the final artifact
  is synthetic-prior-only and cannot be consumed as a candidate table.

- [ ] **Step 1: Write failing lane-decode and guest-loop tests**

Keep the historical `OpcodeLabInput` schema unchanged. In a module reachable only from the dedicated
`sp1-opcode-lab` binary, decode the exact case/opcode/envelope plus equal-length `anchor_target_` or
`anchor_control` scenario into a typed lane. Assert all other JSON/bincode follows the ordinary path,
unrelated guest ELFs remain byte-identical, and only opcodes `0x50`, `0x5f`, `0x80`, and `0x90` validate.
Assert the target/control loop returns deterministic common-mix accumulators, binds lane in the
fixed-size public digest, calls all four REVM stack primitives successfully, keeps bincode length
fixed, and rejects unsupported probes.

- [ ] **Step 2: Implement the minimal probe guest path**

Keep ordinary `execute_bytecode` unchanged. In probe mode, instantiate REVM's real
`revm::interpreter::Stack`, clear and seed the same two `U256` words in both lanes, and call only the
selected `pop`, `push(U256::ZERO)`, `dup(1)`, or `swap(1)` primitive in the target lane. Select an
`#[inline(never)]` target or no-op control function once before the loop so both lanes make one
function-pointer call per iteration without a per-iteration opcode match. Apply the same
`black_box(&stack)` compiler barrier and accumulator mix in both lanes without reading stack length
or contents. Keep every guest field except
the fixed-width `target_count` identical across counts and prepare the public digest from a
fixed-size byte array.

- [ ] **Step 3: Write failing launcher identity and exit tests**

Assert opcode-lab input reports contain the SHA-256 and length of their canonical bincode input in
single and batch assembly. Assert missing/nonzero SP1 exit code fails before single JSON or batch
JSONL publication for both `opcode-lab` and `revm-opcode-lab` gas-estimator paths.

- [ ] **Step 4: Implement fail-closed report finalization**

Use one report finalizer for single and batch opcode stages. It installs canonical input identity,
applies execution metadata, requires `exit_code == 0`, and only then permits serialization. A stale
ELF that exits 3 with empty public values must return an error and cannot create an accepted raw row.

- [ ] **Step 5: Write failing end-to-end Python tests**

Freeze fit counts `[0, 1024, 4096, 16384, 65536]`, checkpoint `131072`, and three repeats. Test
stable guest fields/bytecode/input length, target/control pair identities, exact raw ordering, ELF
hash and cadence binding, unsupported declarations, repeat nondeterminism, nonpositive/tiny signal,
R2/residual/stderr failure, count-zero intercept-residual failure, and checkpoint APE failure.

- [ ] **Step 6: Implement generate/run/fit commands and diagnostic sealing**

Fit `P_target(n) - P_control(n)` with a free Decimal intercept. Require positive slope,
`R2 >= 0.99`, relative stderr `<= 0.05`, residual/signal `<= 0.02`, count-zero
intercept-residual/signal `<= 0.02`, marginal checkpoint APE `<= 0.10`, and exact repeat
determinism. Compute checkpoint prediction as `slope * checkpoint_count` and observation as
`delta(checkpoint_count) - fitted_intercept`; never divide by total proverGas. Emit
per-operation and raw-gas-normalized slopes for POP/PUSH0/DUP1/SWAP1 plus canonical hashes and
explicit `synthetic_prior_only=true`, `candidate_eligible=false` declarations.

- [ ] **Step 7: Document boundaries and verify without committing**

Update the experiment README and calibration-pitfalls solution. Run focused Python/Rust tests, the
complete `experiments/opcode-gas/tests` suite, relevant primitives/guest-launcher tests,
`py_compile`, `cargo fmt --all -- --check`, and `git diff --check`. Do not build or run a real SP1
campaign and do not modify an existing run artifact.

---

### Task 4: Build Manifest-Frozen Production-Guest Block Fixtures

**Files:**
- Modify: `bin/guest-launcher/src/controlled_workload.rs`
- Modify: `bin/guest-launcher/src/main.rs`
- Modify: `bin/guest-launcher/tests/controlled_workload.rs`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/manifests/sp1-calibration-v1.toml`
- Modify: `experiments/opcode-gas/tests/test_manifest.py`
- Modify: `experiments/opcode-gas/tests/test_sp1_candidate_report.py`

**Interfaces:**
- Consumes: formal affine opcode model and manifest-frozen block row specifications.
- Produces: host-trace-validated `block-calibration-rows.jsonl` containing absolute actual raw-gas
  vectors, fixed/base feature vectors, diagnostic feature counts, three SP1 repeats, workload
  family, and `fit|holdout` split.

- [ ] **Step 1: Write failing Rust fixture tests**

Add serializable row types:

```rust
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlledBlockSplit {
    Fit,
    Holdout,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ControlledProgram {
    Empty,
    NativeTransfer { value: u64 },
    OpcodeLoop {
        family: String,
        count: u64,
        scenario: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ControlledBlockRowSpec {
    pub row_id: String,
    pub workload_family: String,
    pub split: ControlledBlockSplit,
    pub block_count: usize,
    pub transaction_count: u64,
    pub program: ControlledProgram,
    pub expected_raw_gas_by_key: BTreeMap<String, i64>,
    pub expected_features: BTreeMap<String, i64>,
    pub expected_diagnostics: BTreeMap<String, i64>,
}
```

Tests must prove a row ID changes when any semantic field changes, post-Unzen timestamps are used,
system/Anchor operations are excluded, and actual trace raw gas/features/diagnostics equal the
specification before SP1 execution.

- [ ] **Step 2: Run Rust tests and verify RED**

Run:

```bash
cargo test -p guest-launcher --test controlled_workload -- --nocapture
```

Expected: compilation failure because the new row types/builders do not exist.

- [ ] **Step 3: Generalize the controlled GuestInput builder**

Replace the private fixed `CandidateKind::MinimalContractCall` branch with a controlled contract
variant that accepts generator-produced bytecode while keeping the address, account shape, code
length class, transaction envelope, and touched keys frozen. Bind each row's resulting final state
root exactly; do not require roots to match when the frozen count immediate changes the deployed
code hash. Reuse `build_overhead_guest_input()` for empty, transfer, block-count, and
transaction-count rows.

The `OpcodeLoop` generator may use fixed-length PUSH immediates and traced helper opcodes, but every
helper's actual raw gas must appear in `expected_raw_gas_by_key`. Do not add a parallel EVM
interpreter or copy pricing logic into Rust.

For count-bearing opcode rows, freeze `gas_limit = 100000`, empty calldata, and 256-byte code. Encode
the count only as a fixed-width `PUSH3` immediate. Reject a generated trace containing a runtime
count-source `GAS` or `CALLDATALOAD`; varying `gas_limit` with count is a confounded fixture even when
the resulting raw opcode counts are correct.

Add a non-fitting data-control family that changes only the `PUSH3` immediate, discards it, and then
executes one fixed modeled operation sequence. Run every control input with normal exact repeats.
Serialize the proverGas range across distinct control inputs as `cross_input_data_floor_p`; do not
average or subtract it, and do not treat it as repeat noise. Report each opcode-anchor family's
signal and fit residual relative to this floor. A fixture/guest revision change requires a newly
measured floor and fresh calibration identity.

- [ ] **Step 4: Materialize the fit and holdout inventory in the manifest**

Add exactly five `fit` rows and one `holdout` row for each of the eight frozen families
`pop_family`, `push_family`, `dup_family`, `swap_family`, `proposal_startup`, `block_base`,
`tx_base`, and `native_value_transfer`, for 40 fit rows and eight holdout rows. Use the frozen
controlled count sequence `1, 2, 4, 8, 16` in each count-bearing fit family. The five
`proposal_startup` rows are separately named manifest panels with different predeclared controlled
operation/base-feature mixes; startup remains exactly one in every valid guest execution. The
exact four-family `slope_count(xBC)` transfer matrix must have rank two, the 40-row `q` matrix must
have rank four, and the eight holdouts must cover all eight families.

Freeze these diagnostic fields on every row: `guest_input_bincode_length`, `witness_node_count`,
`witness_byte_count`, `blob_count`, `kzg_invocation_count`, `calldata_length`, `bytecode_length`,
and `touched_state_key_count`.

- [ ] **Step 5: Add pre-execution matrix validation in Python**

Create:

```python
def preflight_block_calibration_rows(
    manifest: Manifest,
    affine_model: AffineOpcodeModel,
) -> dict[str, Any]:
```

Build exact `Fraction` transfer-family slope rows from `x_j * B * C` and separate exact `q_j` rows
from manifest expectations before launching SP1. Require 40 fit rows, transfer rank two,
fixed/base rank four, eight holdouts covering all workload families, no precompile or spawned work,
and zero dynamic-key totals across fit and holdout rows. Reject rounded basis values.

- [ ] **Step 6: Run all host traces before each SP1 execution**

Add `run-block-calibration`. It first runs Python preflight, then invokes guest-launcher for the
frozen rows. Guest-launcher constructs each GuestInput, runs `trace_shasta_proposal`, compares
actual raw gas/features/diagnostics to the row spec, and only then calls the production SP1 proposal
guest three times through `--sp1-execution-engine gas-estimator`. Before the campaign, run one
frozen row once through the standard engine and once through the estimator and require exact
proverGas, instruction-count, syscall-count, and public-values parity. A trace or parity mismatch
emits a rejected result and no formal SP1 observations.

Treat the SP1 gas estimator only as an offline calibration/validation oracle. Do not wire it into
production online quoting or admission; those paths continue to use the host-native operation
ledger and sealed coefficient table. Do not record an elapsed-time benchmark without a captured
command, hardware context, and output.

- [ ] **Step 7: Run focused Python and Rust tests and commit**

Run:

```bash
cargo fmt --all -- --check
cargo test -p guest-launcher --test controlled_workload -- --nocapture
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_manifest.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py -v
```

Expected: all checks pass.

```bash
git add bin/guest-launcher/src/controlled_workload.rs \
  bin/guest-launcher/src/main.rs \
  bin/guest-launcher/tests/controlled_workload.rs \
  experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  experiments/opcode-gas/tests/test_manifest.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py
git commit -m "feat(zkgas): add controlled block calibration fixtures"
```

---

### Task 5: Fit Six Transfer/Block Parameters And Validate Dynamic Holdouts

**Files:**
- Modify: `experiments/opcode-gas/calibration_model.py`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_calibration_model.py`
- Modify: `experiments/opcode-gas/tests/test_sp1_candidate_report.py`

**Interfaces:**
- Consumes: accepted `opcode-relations.json`, accepted `anchor-probe-fit.json`, and validated
  controlled block rows.
- Produces: `BlockCalibrationResult`, two production transfer parameters, four reconstructed anchor
  multipliers, four fixed/base costs, all 102 reconstructed opcode multipliers, fit/holdout
  predictions, dynamic holdout results, opcode-family slope evidence, and leave-one-family-out
  omitted-slope predictions.

- [ ] **Step 1: Write failing known-recovery and gate tests**

Use these public types:

```python
@dataclass(frozen=True)
class BlockCalibrationRow:
    row_id: str
    workload_family: str
    split: str
    prover_gas: Decimal
    raw_gas_by_key: Mapping[str, int]
    feature_counts: Mapping[str, int]
    workload_count: int | None


@dataclass(frozen=True)
class BlockCalibrationResult:
    transfer_params: Mapping[str, Decimal]
    reconstructed_anchors: Mapping[str, Decimal]
    fixed_costs: Mapping[str, Decimal]
    opcode_multipliers: Mapping[str, Decimal]
    fit_mape: Decimal
    fit_max_ape: Decimal
    holdout_max_ape: Decimal
    status: str
```

Create a synthetic staged dataset with a rank-two opcode-family slope matrix and rank-four fixed
matrix. Assert recovery within
`Decimal("1e-60")` for both transfer parameters, every fixed/base parameter, and every reconstructed
multiplier. Add separate tests rejecting transfer rank one, fixed rank three, invalid transfer
parameters, a negative
reconstructed multiplier, fit MAPE above 5%, fit or holdout maximum APE above 10%, and
leave-one-family-out omitted production-slope APE above 10%.

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_calibration_model.py -v
```

Expected: import failures for the block fitting interfaces.

- [ ] **Step 3: Implement Decimal least squares without third-party dependencies**

Set `getcontext().prec = 80`. Column-scale the Decimal-converted exact-rational design matrix, then
solve least squares with modified Gram-Schmidt QR plus one reorthogonalization pass. Reject a zero
column norm or zero diagonal instead of regularizing it. Unscale the solution before prediction.
Preserve the original exact-rational matrix, its rank, the scale vector, and the Decimal solver
residual in the artifact. Tests must include an ill-scaled full-rank matrix that fails without
column scaling but recovers within `Decimal("1e-50")` through this path.

Implement:

```python
def fit_block_calibration(
    affine_model: AffineOpcodeModel,
    rows: Sequence[BlockCalibrationRow],
    feature_keys: tuple[str, ...],
    anchor_body_costs: Mapping[str, Decimal],
) -> BlockCalibrationResult:
```

Compute every row error as
`abs(predicted_prover_gas - actual_prover_gas) / actual_prover_gas`; reject a missing, non-finite,
or non-positive actual value. Within each opcode family, regress each transfer column,
`x * mu_zero`, and observed proverGas on the frozen workload count with a free intercept. Fit
`[body_scale, common_opcode_overhead]` only from the resulting four-by-two slope system, then freeze
the reconstructed opcode table. Build the two transfer columns from
`theta_i = (body_scale * anchor_body_costs[i] + common_opcode_overhead) / raw_gas_i` using the
frozen raw-gas divisors `(2, 2, 3, 3)`. Fit the four fixed/base coefficients in a separate
rank-four least-squares solve after subtracting `x * mu`. Run one transfer refit per opcode family,
require the reduced slope matrix to retain rank two, and gate the omitted family's complete
production-slope APE rather than coefficient drift. Require every fixed/base leave-one-family-out
matrix to retain rank four.

- [ ] **Step 4: Preserve scalar fail-closed validation and add structured diagnostics**

Create:

```python
@dataclass(frozen=True)
class DynamicRelationObservation:
    dynamic_key: str
    scenario_id: str
    split: str
    equation: RelationEquation


def validate_dynamic_holdouts(
    affine_model: AffineOpcodeModel,
    opcode_multipliers: Mapping[str, Decimal],
    dynamic_relations: Sequence[DynamicRelationObservation],
    dynamic_keys: tuple[str, ...],
) -> dict[str, Any]:
```

Reconstruct `mu_lab` from `anchor_body_cost / anchor_raw_gas`; do not compare opcode-lab slopes to
the production multipliers produced after applying `body_scale` and common overhead. For every
dynamic key, require exactly one `canonical` and at least one `dynamic_holdout`
observation. For every observation, require finite nonzero observed slope, predicted sign equality,
relation APE at most 10%, and a nonzero coefficient for the declared dynamic key. Isolate the
dynamic key from the signed equation to compute its implied multiplier and require it to be
positive. For each key require `max(implied_mu) / min(implied_mu) - 1 <= 0.05`. This function
remains the candidate boundary: a failed lab-body scalar hypothesis prevents candidate sealing and
never changes the multiplier. A future production representation must separately account for the
common per-operation overhead; a lab-body scalar pass is not sufficient evidence for a production
per-raw-gas scalar when raw gas varies.

Add a separate `fit_dynamic_opcode_models` diagnostic over these frozen feature orders:

```python
DYNAMIC_OPCODE_FEATURE_ORDERS = {
    "opcode:0x0a": ("constant", "exponent_bytes", "exponent_bytes_squared"),
    "opcode:0x20": ("constant", "input_words", *SHARED_MEMORY_FEATURES),
    "opcode:0x51": ("constant", *SHARED_MEMORY_FEATURES),
    "opcode:0x52": ("constant", *SHARED_MEMORY_FEATURES),
    "opcode:0x53": ("constant", *SHARED_MEMORY_FEATURES),
    "opcode:0x5e": ("constant", "copy_words", *SHARED_MEMORY_FEATURES),
}
```

Recover target body cost by removing the signed static-control contribution from each accepted
relation slope. Fit MLOAD, MSTORE, and MSTORE8 jointly with opcode-specific constants and shared
`memory_growth_event`, exact EVM memory-gas delta, and a binary `memory_4k_boundary_event`
coefficient. The boundary event is one when the operation crosses at least one additional 4-KiB
logical-memory boundary beyond the warmed state, regardless of how many boundaries it crosses.
Subtract the shared memory contribution before fitting KECCAK256 and MCOPY and add it back for their
predictions. Require exact rank six for the shared matrix and aggregate rank and parameter count 13.
Fit with `Decimal`, never binary `float`, and transform to production units with the staged block fit:

```text
production_constant = body_scale * body_constant + common_opcode_overhead
production_nonconstant = body_scale * body_nonconstant
```

Gate production-space fit MAPE at 5%, fit max APE at 10%, and holdout max APE at 10%.
Quality failures produce `status = not_supported` with complete evidence. Structural identity,
rank, provenance, or numeric failures still raise. Serialize the result as the content-addressed,
schema-3 non-candidate `dynamic-opcode-models.json`; do not pass it to `build-candidate`. Treat the
shared V1 `f_mem` as a hypothesis that future evidence may split by opcode family or zkVM without
changing the non-memory multiplier model.

- [ ] **Step 5: Add `fit-block-calibration` and canonical serialization**

The command verifies run provenance and row identities, calls the fitter and dynamic validator, and
writes `block-calibration.json` atomically. Include hashes of the relation artifact and raw block
rows, exact parameter order, formulas, gates, predictions, and status.

- [ ] **Step 6: Run focused tests and commit**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_calibration_model.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py -v
```

Expected: all tests pass.

```bash
git add experiments/opcode-gas/calibration_model.py \
  experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/tests/test_calibration_model.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py
git commit -m "feat(zkgas): fit opcode anchors from controlled blocks"
```

---

### Task 6: Make The Reconstructed Mapping The Only Candidate Path

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_run_manifest.py`
- Modify: `experiments/opcode-gas/tests/test_sp1_candidate_report.py`

**Interfaces:**
- Consumes: accepted formal relation artifact, accepted block-calibration artifact, existing
  precompile/fixed-event controlled fit, provenance, and schedule inventory.
- Produces: the canonical review-only candidate components and transitive candidate digest.

- [ ] **Step 1: Write failing candidate-boundary tests**

Test that candidate construction rejects:

```text
old target-only opcode slopes
matched-control diagnostic reports
missing or rejected formal relation artifact
missing or rejected block calibration
changed relation/block raw artifact hash
changed anchor order or formula
proposal-purpose calibration rows
non-positive opcode/fixed/base values
failed dynamic holdout evidence
```

Also test that changing bridge-only data leaves `candidate.sha256` unchanged.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_run_manifest.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py -v
```

Expected: candidate tests fail because the old controlled fit/overhead path is still authoritative.

- [ ] **Step 3: Replace the pure-opcode candidate source**

Change `build_candidate_components()` to consume the 102 accepted multipliers and four fixed/base
costs from `block-calibration.json`. Continue consuming precompile and fixed spawned-wrapper values
from `controlled-fit.json`. Remove pure-opcode values and required Q values from the old
`construct_measurement_values()`/`fit_controlled_overheads()` candidate path so there is one source
of truth. Replace the CLI's old `--fit` and `--overheads` inputs with required `--relations`,
`--block-calibration`, and `--controlled-fit` inputs; keep `--run`, `--controlled-manifest`, and
`--provenance`.

- [ ] **Step 4: Update terminal decisions and provenance**

Persist and verify:

```text
opcode_relations_sha256
block_calibration_rows_sha256
block_calibration_sha256
controlled_fit_sha256
controlled_decisions_sha256
```

Resume must revalidate every digest and status. A terminal complete decision requires all formal
relations, block fit/holdout gates, dynamic holdouts, precompile/fixed-event dependencies, and Q
values to be accepted.

- [ ] **Step 5: Bind the candidate root**

Include formulas, parameter order, rank evidence, relation gates, block gates, dynamic holdouts,
all component hashes, normalized `m_p`, raw `c_p`, fixed `f_p`, and fixed/base `o_p`. Preserve
`review_only = true` and `production_write = false`.

- [ ] **Step 6: Run candidate tests and commit**

Run:

```bash
~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_run_manifest.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py -v
```

Expected: all tests pass.

```bash
git add experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/tests/test_run_manifest.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py
git commit -m "feat(zkgas): seal relative block-calibrated candidate"
```

---

### Task 7: Run Full Verification And Independent Review

**Files:**
- Modify only files needed to fix confirmed review or verification findings from Tasks 1-6.

**Interfaces:**
- Consumes: complete implementation diff against `09605f61`.
- Produces: a review-approved, locally verified implementation revision ready for a fresh
  content-addressed calibration run.

- [ ] **Step 1: Run formatting and Python syntax checks**

```bash
cargo fmt --all -- --check
~/.venv/bin/python -m py_compile \
  experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/calibration_model.py
git diff --check
```

- [ ] **Step 2: Run the complete Python experiment suite**

```bash
~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
```

Expected: all tests pass, with only the existing explicitly skipped external-data test.

- [ ] **Step 3: Run focused Rust tests**

```bash
cargo test -p guest-launcher --test controlled_workload -- --nocapture
cargo test -p guest-launcher controlled_overhead -- --nocapture
```

Expected: all tests pass.

- [ ] **Step 4: Run one host-only fixture preflight smoke**

Use the new preflight command on the checked-in manifest and formal relation fixture. It must report:

```text
opcode_keys=102
canonical_relations=98
relation_rank=98
relation_nullity=4
block_fit_rows>=40
block_fit_rank=8
block_holdout_rows>=8
dynamic_keys=6
```

No SP1 proposal or final-validation input is opened in this step.

- [ ] **Step 5: Request independent adversarial review and behavioral verification**

The reviewer receives the original user goal, approved spec, this plan, and the complete diff. The
tester independently reruns the exact-rank synthetic recovery, dynamic holdout failure cases,
controlled block host traces, and candidate tamper/rejection tests. Fix or rebut every material
finding, then have its author recheck the result.

- [ ] **Step 6: Commit verified review fixes**

```bash
git add docs/plans/2026-09-06-zkgas-multiplier-recalibration-experiment-design.md \
  docs/plans/2026-06-08-sp1-opcode-prover-gas-experiment-implementation-plan.md \
  experiments/opcode-gas/README.md \
  experiments/opcode-gas/calibration_model.py \
  experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  experiments/opcode-gas/tests/test_calibration_model.py \
  experiments/opcode-gas/tests/test_fixture_emit.py \
  experiments/opcode-gas/tests/test_manifest.py \
  experiments/opcode-gas/tests/test_run_manifest.py \
  experiments/opcode-gas/tests/test_runner.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py \
  bin/guest-launcher/src/controlled_workload.rs \
  bin/guest-launcher/src/main.rs \
  bin/guest-launcher/tests/controlled_workload.rs
git commit -m "fix(zkgas): address calibration review findings"
```

Skip this commit when review produces no confirmed finding.

---

### Task 8: Run A Fresh Controlled Calibration And Seal The Mapping

**Files:**
- Generate a new content-addressed directory under ignored path: `experiments/opcode-gas/runs/`
- Do not modify tracked source, manifest, dependency, or guest artifact files after preparation.

**Interfaces:**
- Consumes: clean verified implementation revision, frozen manifest, current pinned Alethia revision,
  and existing SP1 guest artifacts.
- Produces: formal relation, controlled block, dynamic holdout, precompile/fixed-event, candidate,
  and bridge artifacts under one new calibration identity.

- [ ] **Step 1: Verify the checkout and create a new calibration identity**

```bash
git status --short
cargo build -r -p guest-launcher
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-calibration \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --guest-launcher target/release/guest-launcher \
  --out experiments/opcode-gas \
  --run-path-file /tmp/raiko2-zkgas-calibration-run-path
read -r CALIBRATION_RUN < /tmp/raiko2-zkgas-calibration-run-path
test -d "$CALIBRATION_RUN"
```

Expected: tracked worktree is clean and a new run ID names the current implementation revision.

- [ ] **Step 2: Run and fit the frozen synthetic probe and formal opcode relation cohort**

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate-anchor-probe \
  --calibration-run "$CALIBRATION_RUN" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_opcode_lab.elf \
  --out "$CALIBRATION_RUN/generated/anchor-probe"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-anchor-probe \
  --calibration-run "$CALIBRATION_RUN" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_opcode_lab.elf \
  --fixtures "$CALIBRATION_RUN/generated/anchor-probe" \
  --out "$CALIBRATION_RUN/raw/anchor-probe.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-anchor-probe \
  --calibration-run "$CALIBRATION_RUN" \
  --runs "$CALIBRATION_RUN/raw/anchor-probe.jsonl" \
  --out "$CALIBRATION_RUN/anchor-probe-fit.json"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate-relations \
  --manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --calibration-run "$CALIBRATION_RUN" \
  --out "$CALIBRATION_RUN/generated/formal-relations"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-relations \
  --fixtures "$CALIBRATION_RUN/generated/formal-relations" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_revm_opcode_lab.elf \
  --calibration-run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/raw/formal-relations.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-relations \
  --runs "$CALIBRATION_RUN/raw/formal-relations.jsonl" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/opcode-relations.json"
```

The runner enforces three repeats and the adaptive frozen prefixes. Stop on a failed required
relation; do not change a threshold or scenario after seeing output.

- [ ] **Step 3: Preflight and execute controlled blocks**

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-block-calibration \
  --guest-launcher target/release/guest-launcher \
  --calibration-run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --anchor-probe "$CALIBRATION_RUN/anchor-probe-fit.json" \
  --out "$CALIBRATION_RUN/block-calibration-rows.jsonl"
```

Confirm preflight reports exact relation rank 98, transfer rank two, and fixed/base rank four
before the first production-guest SP1 execution. Preserve every raw row if a later gate fails.

- [ ] **Step 4: Fit structured diagnostics, then test the lab-body scalar boundary**

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-dynamic-opcode-models \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --anchor-probe "$CALIBRATION_RUN/anchor-probe-fit.json" \
  --runs "$CALIBRATION_RUN/block-calibration-rows.jsonl" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/dynamic-opcode-models.json"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-block-calibration \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --anchor-probe "$CALIBRATION_RUN/anchor-probe-fit.json" \
  --runs "$CALIBRATION_RUN/block-calibration-rows.jsonl" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/block-calibration.json"
```

Require `dynamic-opcode-models.json` to preserve the explicit shared-memory fit, the three
operation-specific fits, exact ranks, production-space errors, zero-growth holdouts, retained
expansion holdouts, and the fresh untouched expansion holdout even when a quality gate reports
`not_supported`. The structured artifact is diagnostic only.
`fit-block-calibration` retains the lab-body scalar dynamic gate; if
it rejects
the already falsified one-multiplier hypothesis, preserve that failure and do not proceed to
candidate sealing. A future promotion task must choose and review the production representation
before the remaining candidate steps become eligible.

- [ ] **Step 5: Fit remaining precompile/fixed-event components and seal**

Run the existing controlled precompile/fixed-event path against the reconstructed opcode table,
then seal the candidate and bridge:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-controlled \
  --fixtures "$CALIBRATION_RUN/generated/controlled" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_revm_opcode_lab.elf \
  --precompile-elf crates/guests/elf/sp1_precompile_lab.elf \
  --calibration-run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/controlled-runs.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py build-candidate \
  --run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --anchor-probe "$CALIBRATION_RUN/anchor-probe-fit.json" \
  --block-calibration "$CALIBRATION_RUN/block-calibration.json" \
  --controlled-fit "$CALIBRATION_RUN/controlled-fit.json" \
  --provenance "$CALIBRATION_RUN/provenance.json"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py build-sp1-bridge \
  --run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --samples "$CALIBRATION_RUN/samples/controlled-cycle-cost-samples.json"
```

Verify `candidate.sha256` before and after bridge construction is identical.

- [ ] **Step 6: Produce the controlled calibration summary**

Report the calibration ID, implementation revision, artifact hashes, every rejected/unmeasured key,
both transfer parameters, all four reconstructed anchors, all four fixed/base costs, fit MAPE/max
APE, holdout max APE, dynamic-key results,
and the candidate digest. Do not claim validation against real proposals yet.

---

### Task 9: Validate The Sealed Mapping On The Frozen Proposal Corpus

**Files:**
- Generate under ignored paths: the existing immutable validation/corpus directories.
- Do not change calibration artifacts or candidate bytes.

**Interfaces:**
- Consumes: sealed candidate and bridge digests plus the preselected 60-row Mainnet/Hoodi corpus.
- Produces: proposal predictions, per-network and combined errors, coverage, and the final candidate
  classification.

- [ ] **Step 1: Verify the validation barrier**

Require `candidate.sha256` and `bridge.sha256`, verify their complete transitive manifests, and bind
the unchanged corpus with `prepare-validation`.

- [ ] **Step 2: Execute the frozen 60-row corpus once**

Use only `purpose=final_validation`. Reject any duplicate, changed GuestInput hash, wrong network,
wrong proposal ID, or row opened before the candidate seal.

- [ ] **Step 3: Apply the sealed model without refitting**

Compute per-proposal APE, MAPE, maximum APE, and measured coverage from the frozen mapping. Do not
change an offset, table entry, feature, threshold, or workload split.

- [ ] **Step 4: Classify and report**

Emit `candidate_table_validated_at_reported_coverage` only when all frozen validation gates pass;
otherwise emit `candidate_table_not_validated` with exact failures. Preserve the candidate and raw
observations in either outcome. Production table promotion remains a separate reviewed task.
