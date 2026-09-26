# ZKGas Higher-Layer Fixed-Cost Calibration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure SP1 `proposal_startup`, `block_base`, `tx_base`, and
`native_value_transfer` costs without refitting opcode coefficients, then determine whether the
coarse state/trie model is accurate enough on frozen controlled holdouts.

**Architecture:** Reuse the production `sp1-shasta-proposal` guest and the existing controlled
overhead target/control generator, but resolve every operation delta against the exact Task 4
operation manifest and the sealed Osaka core. Fit only the four fixed/base terms. After those terms
are sealed, run paired witness-topology and dirty-account-topology holdouts that cannot alter the
fit; accept the coarse state/trie model only if all predeclared holdouts pass.

**Tech Stack:** Python 3 from `~/.venv`, Rust, REVM Osaka execution, SP1 gas-estimator execution,
canonical JSON/JSONL, exact `Decimal` arithmetic, `unittest`, and Cargo tests.

**Spec:** `docs/plans/2026-09-26-zkgas-calibration-design.md`

## Global Constraints

- Version identity remains Taiko Unzen, `UNZEN_ZK_GAS_SCHEDULE`, Fusaka vocabulary, REVM Osaka,
  SP1, and `proverGas`.
- Consume `experiments/opcode-gas/manifests/operation-coverage-v1.json` with artifact SHA256
  `fbb4817d50b04147d0d9c86a25c82324d5e36ce9d6acbf49c51dd165cf1905a3`.
- Consume the sealed Osaka core at
  `experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/core-opcode-submodel.json` with
  artifact SHA256 `b66d7951bfa416810f93319f99c30ce91969026ee9fa3a402a74cbc2214f7e8b`.
- Do not run formal opcode relations, anchor probes, joint opcode/block calibration, real proposal
  corpus rows, RISC0, Boundless, or proof generation.
- Do not change a production ZKGas table, intrinsic charge, spawn estimate, block limit, runtime
  configuration, or deployment configuration.
- Use the production `sp1-shasta-proposal` ELF with `--mode execute --sp1-execution-engine
  gas-estimator`; bind the exact ELF, launcher, source revision, manifest, and input hashes.
- Every guest execution has three byte-identical repeats. Any repeat instability rejects the row.
- Canonical numeric work uses 80-digit `Decimal`; binary `float` never enters a persisted result.
- Operation rows from unsupported, structured-without-context, direct-precompile-unmeasured, or
  wrapper-unmeasured classifications reject a calibration row. They are never treated as zero.
- Final network proposal validation remains closed after this plan.

## Frozen Mathematical Contract

For controlled execution row `r`:

```text
operation_cost(r) =
    sum(event_count[k] * common_dispatch
      + raw_gas[k] * body_scale * body_per_raw_gas[k])

prediction(r) = operation_cost(r)
              + proposal_startup
              + block_count(r) * block_base
              + started_non_anchor_tx_count(r) * tx_base
              + native_value_transfer_count(r) * native_value_transfer

APE(r) = abs(prediction(r) - observed_prover_gas(r)) / observed_prover_gas(r)
```

Only `static_raw_gas` operation models are allowed in this controlled campaign. The fixed campaign
does not approximate a structured or unsupported operation merely to obtain a row.

Required overhead families use the existing adaptive prefixes and checkpoints:

```text
round 8:   fit [0, 1, 2, 4], checkpoint 8
round 32:  fit [0, 1, 2, 4, 8, 16], checkpoint 32
round 128: fit [0, 1, 2, 4, 8, 16, 32, 64], checkpoint 128
```

Stop at the first passing round. Do not run 512 or 2048 for higher-layer costs. Required slope
gates are exact repeat stability, positive slope, signal at least `max(1000, 1% of baseline)`,
`R2 >= 0.995`, relative slope standard error `<= 0.05`, maximum residual/signal `<= 0.02`, and
checkpoint APE `<= 0.10`. The two `tx_base` scenarios must agree within 5%. Every fitted fixed cost
must be positive.

The state/trie rows are holdout-only target/control pairs with identical `Q_formula` features and
identical operation costs. For each pair:

```text
predicted_delta = delta(operation_cost) + delta(Q_formula * fixed_costs)
effect_ratio = abs((target_prover_gas - control_prover_gas) - predicted_delta)
             / control_prover_gas
```

The coarse state/trie model is `coarse_model_accepted` only when every state holdout has
`effect_ratio <= 0.10` and both target and control total APE are `<= 0.10`. A valid observation that
exceeds either gate produces `needs_state_split`; missing, unstable, or coverage-invalid evidence
produces `inconclusive`. Neither result automatically fits state features.

## File Structure

- Modify `bin/guest-launcher/src/controlled_workload.rs`: preserve per-operation event counts and
  build the two controlled state-topology holdout families.
- Modify `bin/guest-launcher/tests/controlled_workload.rs`: prove workload identity, exact tracing,
  and holdout isolation.
- Modify `experiments/opcode-gas/opcode_gas.py`: parse the higher-layer manifest, resolve frozen
  operation costs, orchestrate bounded execution, fit fixed costs, evaluate holdouts, and seal/replay
  artifacts.
- Modify `experiments/opcode-gas/tests/test_sp1_candidate_report.py`: regression-test the reused
  overhead evaluator with event-count-aware frozen operation costs.
- Create `experiments/opcode-gas/tests/test_higher_layer_calibration.py`: test manifest identity,
  model fitting, fail-closed coverage, holdout verdicts, resume, and replay.
- Create `experiments/opcode-gas/manifests/sp1-higher-layer-v1.json`: content-addressed campaign
  contract and frozen fixture matrix.
- Modify `experiments/opcode-gas/README.md`: document only the supported command sequence and the
  coarse-model decision boundary.
- Modify `docs/plans/2026-09-26-zkgas-calibration-progress.md`: record actual IDs, measurements,
  verdicts, review, and next gate after execution.
- Create the content-addressed directory emitted through `$DERIVATION_PATH` only after all replay
  and quality checks pass. The directory contains exactly `identity.json`, `overhead-evidence.json`,
  `state-holdout-evidence.json`, and `higher-layer-calibration.json`.

---

### Task 1: Freeze The Higher-Layer Campaign Contract

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Test: `experiments/opcode-gas/tests/test_higher_layer_calibration.py`
- Create: `experiments/opcode-gas/manifests/sp1-higher-layer-v1.json`

**Interfaces:**
- Consumes: Task 4 operation coverage manifest and the f945 Osaka core.
- Produces: `load_higher_layer_manifest(path: pathlib.Path) -> HigherLayerManifest` and
  `validate_higher_layer_manifest(...) -> None`.

- [ ] **Step 1: Write failing schema and source-binding tests**

Add tests that require this exact semantic contract:

```python
self.assertEqual(manifest.q_formula, (
    "proposal_startup", "block_base", "tx_base", "native_value_transfer",
))
self.assertEqual(manifest.generator_rounds, (8, 32, 128))
self.assertEqual(manifest.repeats, 3)
self.assertEqual(
    tuple(pair.pair_id for pair in manifest.state_holdouts),
    (
        "witness_topology_1", "witness_topology_8", "witness_topology_32",
        "dirty_accounts_2", "dirty_accounts_8", "dirty_accounts_32",
    ),
)
```

Also test rejection of a changed operation artifact hash, changed core artifact hash, extra family,
round 512, missing checkpoint gate, noncanonical pair ID, duplicate state holdout, state holdout in
the fit set, and a resealed manifest that changes one source path.

- [ ] **Step 2: Run the new tests and confirm RED**

Run:

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_higher_layer_calibration.py
```

Expected: FAIL because `load_higher_layer_manifest` and the manifest do not exist.

- [ ] **Step 3: Implement strict dataclasses and parsing**

Add frozen dataclasses with these interfaces:

```python
@dataclass(frozen=True)
class StateHoldoutSpec:
    pair_id: str
    kind: str
    scale: int
    control: Mapping[str, Any]
    target: Mapping[str, Any]

@dataclass(frozen=True)
class HigherLayerManifest:
    schema_version: int
    purpose: str
    artifact_sha256: str
    version_identity: Mapping[str, str]
    operation_coverage_ref: Mapping[str, str]
    augmented_core_ref: Mapping[str, str]
    q_formula: tuple[str, ...]
    generator_rounds: tuple[int, ...]
    repeats: int
    overhead_case_ids: tuple[str, ...]
    state_holdouts: tuple[StateHoldoutSpec, ...]
    gates: Mapping[str, str | int]
```

Reject unknown fields at every schema level. Recompute `artifact_sha256` over canonical JSON without
the hash field. Resolve each source through the same regular, non-symlink, repo-contained checks as
Task 4 and require exact canonical bytes.

- [ ] **Step 4: Create the canonical manifest**

Freeze these overhead cases:

```text
tx_base_no_code_no_value
tx_base_minimal_contract_call
native_transfer_positive_vs_zero
block_base_one_vs_two_minimal_blocks
startup_minimal_no_candidate_tx
startup_minimal_one_no_code_tx
```

Freeze three `witness_topology` target/control pairs at extra account counts `1`, `8`, and `32`, and
three `dirty_accounts` pairs at native-transfer counts `2`, `8`, and `32`. Dirty-account controls
send all transfers to one recipient; targets send the same values and transaction count to distinct
recipients. State holdouts use three repeats and are never part of a fit matrix.

- [ ] **Step 5: Run focused tests and verify canonical replay**

Run:

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_higher_layer_calibration.py
git diff --check
```

Expected: PASS; rebuilding the manifest produces byte-identical JSON and the same artifact hash.

- [ ] **Step 6: Commit Task 1**

```bash
git add experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/tests/test_higher_layer_calibration.py \
  experiments/opcode-gas/manifests/sp1-higher-layer-v1.json
git commit -m "test(zkgas): freeze higher-layer calibration contract"
```

### Task 2: Preserve Event Counts In Controlled Operation Deltas

**Files:**
- Modify: `bin/guest-launcher/src/controlled_workload.rs`
- Modify: `bin/guest-launcher/tests/controlled_workload.rs`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Test: `experiments/opcode-gas/tests/test_sp1_candidate_report.py`
- Test: `experiments/opcode-gas/tests/test_higher_layer_calibration.py`

**Interfaces:**
- Consumes: serialized `ControlledOperationUnits` from the production host trace.
- Produces: exact `{pricing_basis, units, event_count}` deltas and
  `resolve_static_operation_delta(core, coverage, key, delta) -> Decimal`.

- [ ] **Step 1: Write failing Rust aggregation tests**

For a controlled contract containing two `PUSH0` operations per transaction, require:

```rust
assert_eq!(push0.pricing_basis, PricingBasis::RawGasSlope);
assert_eq!(push0.units, 2 * event_count);
assert_eq!(push0.event_count, event_count);
```

Test target/control subtraction for positive, negative, and zero deltas. Test that one key cannot
change pricing basis and that event-count overflow fails.

- [ ] **Step 2: Run the Rust tests and confirm RED**

Run:

```bash
cargo test -p guest-launcher --test controlled_workload operation_units
```

Expected: FAIL because `ControlledOperationUnits` has no `event_count`.

- [ ] **Step 3: Add event-count-aware aggregation**

Extend the serialized type:

```rust
pub struct ControlledOperationUnits {
    pub pricing_basis: PricingBasis,
    pub units: i64,
    pub event_count: i64,
}
```

Increment `event_count` once for each emitted execution or confirmed wrapper row. Subtract both
fields in target/control deltas. Remove zero deltas only when both `units == 0` and
`event_count == 0`. Do not infer event count from raw gas.

- [ ] **Step 4: Write failing Python resolver tests**

For `opcode:0x5f`, require the exact formula:

```python
expected = (
    Decimal(delta["event_count"]) * Decimal(core["registry"]["common_dispatch"])
    + Decimal(delta["units"])
      * Decimal(core["body_scale"])
      * Decimal(core["registry"]["models"]["opcode:0x5f"]
                ["parameters"]["body_per_raw_gas"])
)
self.assertEqual(resolve_static_operation_delta(core, coverage, "opcode:0x5f", delta), expected)
```

Test fail-closed behavior for missing event count, wrong basis, negative impossible absolute counts,
structured models, unsupported keys, unmeasured precompiles, wrappers, and a key absent from the
frozen 168-row coverage ledger.

- [ ] **Step 5: Implement the exact frozen resolver**

The resolver must load model status from the operation coverage manifest, then retrieve parameters
from its exact pinned core. It may resolve only `classification == "static_raw_gas"` and
`model_status == "measured"`. It returns a `Decimal` and never reads the current production schedule
multiplier.

- [ ] **Step 6: Run focused Rust and Python tests**

Run:

```bash
cargo test -p guest-launcher --test controlled_workload
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py \
  experiments/opcode-gas/tests/test_higher_layer_calibration.py
```

Expected: PASS. Existing controlled-overhead identities change only through the explicitly versioned
event-count schema; no historical sealed artifact is rewritten.

- [ ] **Step 7: Commit Task 2**

```bash
git add bin/guest-launcher/src/controlled_workload.rs \
  bin/guest-launcher/tests/controlled_workload.rs \
  experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py \
  experiments/opcode-gas/tests/test_higher_layer_calibration.py
git commit -m "feat(zkgas): preserve controlled operation event counts"
```

### Task 3: Build Isolated State/Trie Holdout Fixtures

**Files:**
- Modify: `bin/guest-launcher/src/controlled_workload.rs`
- Modify: `bin/guest-launcher/tests/controlled_workload.rs`
- Modify: `bin/guest-launcher/src/main.rs`

**Interfaces:**
- Consumes: `StateHoldoutSpec` serialized by the Python campaign runner.
- Produces: production-guest inputs for `witness_topology` and `dirty_accounts`, plus exact host
  observations with unchanged four-term feature counts.

- [ ] **Step 1: Write failing witness-topology tests**

Build control and target fixtures with one block, zero candidate transactions, and respectively
zero and `N` deterministic extra prestate accounts. Assert:

```rust
assert_eq!(target.actual_features, control.actual_features);
assert_eq!(target.actual_raw_gas_by_key, control.actual_raw_gas_by_key);
assert!(target.actual_diagnostics["witness_node_count"]
    > control.actual_diagnostics["witness_node_count"]);
assert!(target.actual_diagnostics["witness_byte_count"]
    > control.actual_diagnostics["witness_byte_count"]);
```

Run for `N = 1, 8, 32`. Require deterministic GuestInput hashes and final roots on rebuild.

- [ ] **Step 2: Write failing dirty-account-topology tests**

Build controls that send `N` native transfers to one recipient and targets that send the same value,
nonce sequence, and count to `N` deterministic distinct recipients. Assert for `N = 2, 8, 32`:

```rust
assert_eq!(target.actual_features, control.actual_features);
assert_eq!(target.actual_raw_gas_by_key, control.actual_raw_gas_by_key);
assert_eq!(target.actual_features["native_value_transfer"], N);
assert!(target.actual_diagnostics["touched_state_key_count"]
    > control.actual_diagnostics["touched_state_key_count"]);
```

Also require all transactions committed successfully and no precompile, wrapper, structured, or
unsupported operation entry appeared.

- [ ] **Step 3: Run the Rust tests and confirm RED**

Run:

```bash
cargo test -p guest-launcher --test controlled_workload state_holdout
```

Expected: FAIL because neither holdout fixture kind exists.

- [ ] **Step 4: Implement deterministic topology builders**

Add an input schema owned by the existing controlled-workload module:

```rust
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ControlledStateHoldout {
    WitnessTopology { extra_account_count: usize },
    DirtyAccounts { transaction_count: u64, value: u64 },
}
```

Derive extra accounts and unique recipients from fixed domain-separated hashes of the zero-based
index. Do not use random keys, wall-clock data, local paths, or network input. Extend the existing
prestate and candidate builders; do not create another proposal reconstruction path.

- [ ] **Step 5: Add the launcher surface**

Add `--stage controlled-state-holdout`. It accepts one JSON pair spec, constructs both lanes,
validates each through `trace_shasta_proposal`, and emits two typed observations. It must support
only local SP1 execute mode with the gas-estimator engine, matching the current controlled-block
guard.

- [ ] **Step 6: Run focused Rust tests and formatting**

Run:

```bash
cargo fmt --all
cargo test -p guest-launcher --test controlled_workload
cargo test -p guest-launcher parses_controlled_state_holdout
```

Expected: PASS.

- [ ] **Step 7: Commit Task 3**

```bash
git add bin/guest-launcher/src/controlled_workload.rs \
  bin/guest-launcher/tests/controlled_workload.rs \
  bin/guest-launcher/src/main.rs
git commit -m "feat(zkgas): add controlled state holdouts"
```

### Task 4: Prepare And Run The Bounded Higher-Layer Campaign

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_higher_layer_calibration.py`
- Modify: `experiments/opcode-gas/README.md`

**Interfaces:**
- Consumes: clean implementation revision, higher-layer manifest, operation coverage, Osaka core,
  launcher, and production SP1 proposal ELF.
- Produces: an ignored immutable run directory containing identity, raw overhead rounds, and round
  decisions. State holdout execution remains closed until Task 5 seals the fixed costs.

- [ ] **Step 1: Write failing prepare/run identity tests**

Require these CLI commands:

```text
prepare-higher-layer-calibration
run-higher-layer-calibration
```

Test rejection of dirty implementation files, wrong manifest/core/coverage hashes, wrong launcher,
wrong ELF, noncanonical output path, rerunning a completed round with different bytes, missing prior
round decisions, and any generator bound outside `8,32,128`.

- [ ] **Step 2: Run focused tests and confirm RED**

Run:

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_higher_layer_calibration.py
```

Expected: FAIL because the commands do not exist.

- [ ] **Step 3: Implement preparation and immutable identity**

`prepare-higher-layer-calibration` creates `$HIGHER_LAYER_RUN/identity.json` under the configured
run root and writes the exact run path to `--run-path-file`. The identity includes:

```text
implementation revision
higher-layer manifest artifact and file hashes
operation coverage artifact and file hashes
Osaka core artifact and file hashes
guest-launcher hash
sp1-shasta-proposal ELF and VK hashes
version identity
SP1 execution parameters
```

The calibration ID is the first 24 hex characters of the full canonical identity SHA256. Existing
directories are accepted only when every byte exactly matches the recomputed identity.

- [ ] **Step 4: Reuse the existing overhead round executor**

Refactor `run_controlled_overhead_round` only enough to bind the new identity and event-count schema.
Do not copy its target/control construction or SP1 invocation loop. Execute round 8 first. After
fitting that round, run 32 only for `checkpoint_generator_bound` or `exhausted_sweep`; run 128 under
the same rule. Any other failure terminates the campaign with preserved raw evidence.

- [ ] **Step 5: Enforce the fixed-cost gate**

Terminate `run-higher-layer-calibration` after the bounded overhead rounds and their immutable
decision ledger. Add a regression test proving this command cannot execute or create a state-holdout
raw file. State holdouts open only through the Task 5 command after an accepted fixed-cost artifact
exists.

- [ ] **Step 6: Document the supported command flow**

Add this exact sequence to the README:

```bash
RUN_PATH_FILE="$(mktemp)"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  prepare-higher-layer-calibration \
  --manifest experiments/opcode-gas/manifests/sp1-higher-layer-v1.json \
  --operation-coverage experiments/opcode-gas/manifests/operation-coverage-v1.json \
  --augmented-core experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/core-opcode-submodel.json \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_shasta_proposal.elf \
  --out experiments/opcode-gas/runs \
  --run-path-file "$RUN_PATH_FILE"
HIGHER_LAYER_RUN="$(<"$RUN_PATH_FILE")"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  run-higher-layer-calibration --run "$HIGHER_LAYER_RUN"
```

- [ ] **Step 7: Run focused tests and commit Task 4**

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_higher_layer_calibration.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py
git diff --check
git add experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/tests/test_higher_layer_calibration.py \
  experiments/opcode-gas/README.md
git commit -m "feat(zkgas): add bounded higher-layer campaign"
```

### Task 5: Fit Fixed Costs And Evaluate The Coarse State Model

**Files:**
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_higher_layer_calibration.py`
- Modify: `experiments/opcode-gas/README.md`

**Interfaces:**
- Consumes: immutable raw overhead evidence; after fixed-cost acceptance, creates and consumes the
  immutable state-holdout evidence.
- Produces: `fit_higher_layer_calibration(run: pathlib.Path) -> Mapping[str, Any]` with fixed costs,
  predictions, coverage, and one of `coarse_model_accepted`, `needs_state_split`, or `inconclusive`.

- [ ] **Step 1: Write failing exact-fit tests**

Use synthetic Decimal rows with known costs:

```text
proposal_startup = 1000
block_base = 2000
tx_base = 300
native_value_transfer = 40
```

Require exact recovery, rank four, first-passing adaptive round, checkpoint exclusion from fitting,
and byte-stable serialization. Test rejection of negative cost, rank loss, repeat noise, low signal,
low R2, excessive slope error, excessive residual, failed checkpoint, and >5% disagreement between
the two tx-base cases.

- [ ] **Step 2: Write failing holdout-verdict tests**

Test all three terminal statuses:

```python
self.assertEqual(result["coarse_state_trie"]["status"], "coarse_model_accepted")
self.assertEqual(failed["coarse_state_trie"]["status"], "needs_state_split")
self.assertEqual(missing["coarse_state_trie"]["status"], "inconclusive")
```

Prove that changing a holdout cannot change any fixed cost or the fixed-cost digest. Require Decimal
APE and `effect_ratio`, both using the observed value denominators defined above.

- [ ] **Step 3: Run focused tests and confirm RED**

Run:

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_higher_layer_calibration.py
```

Expected: FAIL because the fitter and verdict do not exist.

- [ ] **Step 4: Implement fixed-cost fitting**

Reuse `evaluate_controlled_sweep` for non-startup overheads. Replace its operation-cost input with
the exact frozen resolver from Task 2. Residualize `native_value_transfer` by `tx_base`, and
`proposal_startup` by the accepted lower-level terms. Average the two startup residual cases only
after their maximum/minimum spread is `<= 5%`. Expose this through
`fit-higher-layer-fixed-costs --run RUN`; the command writes only the canonical fixed-cost artifact
inside that run and rejects a differing pre-existing result.

- [ ] **Step 5: Implement gated state holdout execution**

Add `run-higher-layer-state-holdouts --run RUN`. It refuses to start unless the canonical fixed-cost
artifact is accepted and its source hashes match the run identity. Execute all six pairs through
`controlled-state-holdout`, three repeats per lane. Persist exact workload spec, GuestInput hash,
execution row ID, proverGas, public output, final state root, operation ledger, four-term feature
counts, and diagnostics. Do not fit or select a state feature.

- [ ] **Step 6: Implement state holdout evaluation**

Validate target/control identity before opening proverGas. Require identical `Q_formula` counts and
operation costs for each state pair. Compute total predictions, total APE, predicted delta, observed
delta, and effect ratio with `Decimal`. Holdout results may set only the coarse-state status; they
cannot rewrite the fixed costs, selected rounds, thresholds, or source identities. Expose this as
`finalize-higher-layer-calibration --run RUN`, which requires all six complete pairs.

- [ ] **Step 7: Implement exact artifact replay**

Add `verify-higher-layer-calibration --run-path-file PATH`. It reconstructs fixtures without SP1
execution, checks every GuestInput hash, replays fitting from raw observations, and requires exact
equality with the persisted result. Reject copied results, missing raw rows, duplicate execution
IDs, edited diagnostics, changed source files, symlinks, and any absolute/user-specific path.

- [ ] **Step 8: Implement create-only sealing**

Add `seal-higher-layer-calibration --run RUN --out-root ROOT
--derivation-path-file PATH`. It must invoke the same verifier, derive the 24-hex derivation ID from
the canonical sealed identity, reject an existing destination, and atomically publish exactly the
four files declared in this plan. Add tests for partial-write cleanup, existing-destination
rejection, changed raw evidence, and byte-identical replay from the four-file package.

- [ ] **Step 9: Document the gated command order**

Extend the README sequence with `fit-higher-layer-fixed-costs`, then
`run-higher-layer-state-holdouts`, then `finalize-higher-layer-calibration`, followed by verification.
State clearly that changing this order is rejected and that holdouts never modify the fixed artifact.

- [ ] **Step 10: Run focused and complete Python tests**

Run:

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
  experiments/opcode-gas/tests/test_higher_layer_calibration.py \
  experiments/opcode-gas/tests/test_sp1_candidate_report.py
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
```

Expected: PASS with only the existing explicitly opt-in skip.

- [ ] **Step 11: Commit Task 5**

```bash
git add experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/tests/test_higher_layer_calibration.py \
  experiments/opcode-gas/README.md
git commit -m "feat(zkgas): fit higher-layer fixed costs"
```

### Task 6: Execute, Seal, And Independently Verify The Campaign

**Files:**
- Create: `$DERIVATION_PATH/identity.json`
- Create: `$DERIVATION_PATH/overhead-evidence.json`
- Create: `$DERIVATION_PATH/state-holdout-evidence.json`
- Create: `$DERIVATION_PATH/higher-layer-calibration.json`
- Modify: `docs/plans/2026-09-26-zkgas-calibration-progress.md`

**Interfaces:**
- Consumes: the reviewed implementation and production SP1 guest artifacts.
- Produces: one portable create-only derivation and the next documented gate.

- [ ] **Step 1: Build and identify the execution artifacts**

Run:

```bash
cargo build -r -p guest-launcher --features sp1-sdk/profiling
sha256sum target/release/guest-launcher crates/guests/elf/sp1_shasta_proposal.elf
git status --short
```

Expected: build PASS and a clean tracked worktree before preparation.

- [ ] **Step 2: Prepare and run the campaign**

Run the README preparation and overhead command sequence. Preserve the exact run path emitted by
`prepare-higher-layer-calibration`; do not select the newest directory. Let the bounded controller
stop at the first passing overhead round. Confirm no state-holdout raw file exists yet.

- [ ] **Step 3: Fit and inspect before sealing**

Run:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  fit-higher-layer-fixed-costs --run "$HIGHER_LAYER_RUN"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  run-higher-layer-state-holdouts --run "$HIGHER_LAYER_RUN"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  finalize-higher-layer-calibration --run "$HIGHER_LAYER_RUN"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  verify-higher-layer-calibration --run-path-file "$RUN_PATH_FILE"
```

Inspect all four costs, chosen prefixes/checkpoints, family errors, every holdout effect ratio and
APE, operation coverage, hashes, and the terminal coarse-state verdict. Do not reinterpret a failed
holdout as an accepted coarse model.

- [ ] **Step 4: Seal a create-only portable derivation**

Run:

```bash
DERIVATION_PATH_FILE="$(mktemp)"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  seal-higher-layer-calibration \
  --run "$HIGHER_LAYER_RUN" \
  --out-root experiments/opcode-gas/derivations \
  --derivation-path-file "$DERIVATION_PATH_FILE"
DERIVATION_PATH="$(<"$DERIVATION_PATH_FILE")"
```

The four output files contain canonical compact raw repeats, workload identities, input hashes,
fixed results, holdout results, and full source identity. Sealing fails if the destination exists or
if verification does not replay exactly.

- [ ] **Step 5: Perform independent adversarial review**

The reviewer independently checks:

```text
no opcode coefficient was refit
all operation deltas resolve through the frozen Task 4 ledger
event count and raw gas are not conflated
startup/block/tx/native ownership is exact and non-overlapping
state holdouts did not participate in fitting
coarse-state verdict follows the frozen gates
no proposal, production table, or runtime configuration changed
```

Fix every confirmed finding and ask the same reviewer to re-check the updated complete diff.

- [ ] **Step 6: Perform independent behavioral verification**

The tester copies the four-file derivation to a clean temporary clone at its frozen revision, runs
the verifier, corrupts one source hash, one proverGas repeat, one event count, one state diagnostic,
and one verdict, and confirms every corruption is rejected. The tester also scans every string and
changed path for machine-specific absolute paths or usernames.

- [ ] **Step 7: Update progress and run final checks**

Record actual IDs, exact costs, selected round, errors, state verdict, commands, review, and next
gate. Then run:

```bash
cargo fmt --all
cargo test -p guest-launcher --test controlled_workload
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
git diff --check
```

- [ ] **Step 8: Commit the sealed evidence**

```bash
git add "$DERIVATION_PATH" \
  docs/plans/2026-09-26-zkgas-calibration-progress.md
git commit -m "chore(zkgas): seal higher-layer calibration"
```

## Plan Exit Boundary

This plan ends after the four fixed/base costs and coarse state/trie verdict are sealed. If the
verdict is `needs_state_split`, the next plan may introduce predeclared witness, access, dirty-node,
or hash/update features using the same frozen holdouts; it must not select features from network
proposal residuals. If the verdict is `coarse_model_accepted`, the next plan may build a complete
lower-layer predictor and only then prepare the still-closed final proposal validation corpus.
