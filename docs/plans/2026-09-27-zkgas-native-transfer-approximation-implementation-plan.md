# zkGas Native-Transfer Approximation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Admit the frozen `5017 proverGas` native EOA-transfer approximation under its `0.002`
whole-guest materiality budget, then finish and seal the existing SP1 higher-layer calibration
without opening proposal validation or changing production zkGas behavior.

**Architecture:** Preserve the rejected v1 campaign and manifest as historical evidence. Add a v2
content-addressed manifest that freezes the approximation and its source, evaluate that policy from
fresh production-guest rows without refitting it, and allow only nonzero unresolved dependencies to
block ordinary block/startup fits. Propagate the distinction between ordinary measurements and the
declared approximation through the fixed-cost, state-holdout, replay, and portable sealing
artifacts.

**Tech Stack:** Python 3 via `~/.venv`, `unittest`, exact `Decimal` arithmetic, canonical JSON and
SHA256 content addressing, Rust `guest-launcher`, SP1 `gas-estimator`, Git.

**Spec:** `docs/plans/2026-09-26-zkgas-calibration-design.md`

## Global Constraints

- Keep `experiments/opcode-gas/manifests/sp1-higher-layer-v1.json` and run
  `999b91b91fd693899d09fa53` immutable rejected evidence; never relabel or resume them.
- The only native-transfer coefficient is exactly `5017 proverGas` per committed native EOA
  transfer. It is selected from prior evidence and must not be recomputed from the new campaign.
- The native whole-guest materiality budget is exactly `0.002`, evaluated with `Decimal` as
  `abs(5017 * count - (target - control)) / target` for every count in
  `[1, 2, 4, 8, 16, 32, 64, 128]`.
- Preserve exact identity, inventory, successful-execution, repeat-stability, ownership,
  target/control lane, and positive-signal checks. Only the ordinary relation residual and
  checkpoint gates are replaced for `native_value_transfer`.
- Keep native EOA transfer independent from EVM call wrappers, opcodes, and precompiles.
- A dependency blocks a case only when the case declares a nonzero delta for that dependency.
- Ordinary `tx_base`, `block_base`, and `proposal_startup` retain their existing fit gates and the
  status `accepted`. Native transfer uses only `declared_approximation`.
- The complete fixed model uses `accepted_with_declared_approximation`; retain controller decision
  `accepted` so the persisted round state machine remains `accepted / expand_next_round /
  terminal_failure`.
- State holdouts remain closed until all three ordinary terms pass and the native materiality gate
  passes. Holdouts may validate or reject the coarse model but may not tune `5017` or `0.002`.
- Do not run proposal workloads, alter production tables/configuration, generate proofs, publish
  artifacts externally, push the branch, or open a PR under this plan.
- Use the production SP1 proposal ELF with `--mode execute --sp1-execution-engine gas-estimator`.
- Preserve user changes and use Conventional Commits. Before every implementation commit, inspect
  the complete relevant diff and run the focused tests named in that task.
- This is a non-trivial experiment-control change. Every coherent implementation task requires an
  independent adversarial review; the final live campaign also requires independent behavioral
  verification before its result is described as sealed.

## File Structure

- Create `experiments/opcode-gas/manifests/sp1-higher-layer-v2.json`: the new immutable campaign
  contract, including coefficient, budget, required counts, and source hashes.
- Modify `experiments/opcode-gas/opcode_gas.py`: v2 manifest parsing, approximation evaluation,
  nonzero-dependency resolution, artifact status propagation, replay, and sealing validation.
- Modify `experiments/opcode-gas/tests/test_higher_layer_calibration.py`: manifest, evaluator,
  artifact, state, replay, and tamper regression coverage.
- Modify `experiments/opcode-gas/README.md`: v2 operator path and the declared-approximation gate.
- Modify `docs/plans/2026-09-26-zkgas-calibration-progress.md`: only after the fresh campaign, with
  exact hashes, decisions, state verdict, and derivation identity or terminal failure.
- Create one content-addressed directory under `experiments/opcode-gas/derivations/` only if the
  fresh campaign passes the fixed and state gates. Generated run directories remain ignored data.

---

### Task 1: Freeze and validate the v2 campaign contract

**Files:**

- Create: `experiments/opcode-gas/manifests/sp1-higher-layer-v2.json`
- Modify: `experiments/opcode-gas/opcode_gas.py:5988-6265`
- Modify: `experiments/opcode-gas/opcode_gas.py:21027-21042`
- Modify: `experiments/opcode-gas/tests/test_higher_layer_calibration.py:1-180`

**Interfaces:**

- Consumes: the v1 manifest fields, immutable source run identity and round hashes recorded in the
  spec, and the existing canonical JSON content-addressing helpers.
- Produces: `NativeTransferApproximationSpec`,
  `HigherLayerManifest.native_transfer_approximation`, and a canonical v2 manifest accepted only at
  `experiments/opcode-gas/manifests/sp1-higher-layer-v2.json`.

- [ ] **Step 1: Write failing manifest-contract tests**

  Change `MANIFEST_PATH` to `sp1-higher-layer-v2.json`, retain an explicit `V1_MANIFEST_PATH`, and
  add assertions equivalent to:

  ```python
  approximation = manifest.native_transfer_approximation
  self.assertEqual(approximation.overhead_key_id, "native_value_transfer")
  self.assertEqual(approximation.method, "frozen_max_observed_per_transfer")
  self.assertEqual(approximation.coefficient_prover_gas, "5017")
  self.assertEqual(approximation.materiality_budget, "0.002")
  self.assertEqual(approximation.required_counts, (1, 2, 4, 8, 16, 32, 64, 128))
  self.assertEqual(
      approximation.source,
      {
          "calibration_id": "999b91b91fd693899d09fa53",
          "identity_sha256": (
              "999b91b91fd693899d09fa53c0502c19"
              "e6c43d6f9a0ddcf4c38bd15ef9c0fccd"
          ),
          "raw_rows_sha256": (
              "297d88799b069765481f9793645da504"
              "c507b14193677109cf760803d491140e"
          ),
          "fit_sha256": (
              "fc92696741c1b74503b4863f156936a6"
              "b3ad00164222058d8db26f14f637b3e5"
          ),
      },
  )
  self.assertEqual(json.loads(V1_MANIFEST_PATH.read_text())["artifact_sha256"],
                   "f531201bd93f98ba20e51474ae4d64ae1f444bdaad027ea88362a47fb45e9809")
  ```

  Add resealed-manifest rejection cases for a changed coefficient, budget, method, required count,
  source calibration ID, source identity hash, raw hash, fit hash, unknown field, noncanonical
  decimal, and schema version.

- [ ] **Step 2: Run the focused manifest tests and confirm the expected failure**

  Run:

  ```bash
  env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
    experiments.opcode-gas.tests.test_higher_layer_calibration.HigherLayerManifestTests
  ```

  Expected: FAIL because the v2 manifest and `native_transfer_approximation` interface do not yet
  exist.

- [ ] **Step 3: Add the typed policy and exact validator**

  Add the following frozen shape near `HigherLayerManifest`:

  ```python
  _HIGHER_LAYER_SCHEMA_VERSION = 2
  _HIGHER_LAYER_NATIVE_TRANSFER_APPROXIMATION = {
      "overhead_key_id": "native_value_transfer",
      "method": "frozen_max_observed_per_transfer",
      "coefficient_prover_gas": "5017",
      "materiality_budget": "0.002",
      "required_counts": [1, 2, 4, 8, 16, 32, 64, 128],
      "source": {
          "calibration_id": "999b91b91fd693899d09fa53",
          "identity_sha256": (
              "999b91b91fd693899d09fa53c0502c19"
              "e6c43d6f9a0ddcf4c38bd15ef9c0fccd"
          ),
          "raw_rows_sha256": (
              "297d88799b069765481f9793645da504"
              "c507b14193677109cf760803d491140e"
          ),
          "fit_sha256": (
              "fc92696741c1b74503b4863f156936a6"
              "b3ad00164222058d8db26f14f637b3e5"
          ),
      },
  }

  @dataclass(frozen=True)
  class NativeTransferApproximationSpec:
      overhead_key_id: str
      method: str
      coefficient_prover_gas: str
      materiality_budget: str
      required_counts: tuple
      source: Mapping[str, str]
  ```

  Add `native_transfer_approximation: NativeTransferApproximationSpec` to
  `HigherLayerManifest`. Extend `validate_higher_layer_manifest` with the field and require exact
  canonical equality to `_HIGHER_LAYER_NATIVE_TRANSFER_APPROXIMATION`. Parse both decimals with
  `_decimal`, require a positive coefficient and `0 < budget < 1`, require positive strictly
  increasing unique counts, validate each source hash with `_is_sha256`, and construct immutable
  tuple/mapping values in `load_higher_layer_manifest`.

- [ ] **Step 4: Create the canonical v2 manifest without changing v1**

  Copy the v1 logical content into `sp1-higher-layer-v2.json`, set `schema_version` to `2`, add the
  exact `native_transfer_approximation` object above, and recompute `artifact_sha256` over the
  canonical object with that field omitted. Write the final file with
  `_canonical_json_file_bytes`; do not edit `sp1-higher-layer-v1.json`.

  Update `_HIGHER_LAYER_MANIFEST_PATH` to the v2 path. Keep operation coverage, augmented core,
  rounds, cases, state holdouts, and all existing gates byte-for-byte equivalent at the logical JSON
  value level.

- [ ] **Step 5: Run the manifest tests and inspect the exact diff**

  Run:

  ```bash
  env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
    experiments.opcode-gas.tests.test_higher_layer_calibration.HigherLayerManifestTests
  git diff --check
  git diff -- experiments/opcode-gas/manifests/sp1-higher-layer-v1.json \
    experiments/opcode-gas/manifests/sp1-higher-layer-v2.json \
    experiments/opcode-gas/opcode_gas.py \
    experiments/opcode-gas/tests/test_higher_layer_calibration.py
  ```

  Expected: tests PASS, `git diff --check` is silent, and v1 has no diff.

- [ ] **Step 6: Obtain independent adversarial review, fix confirmed findings, and commit**

  The reviewer must inspect exact schema rejection, v1 preservation, source-hash provenance,
  canonical decimals, and the committed manifest hash. After re-running Step 5, commit:

  ```bash
  git add experiments/opcode-gas/manifests/sp1-higher-layer-v2.json \
    experiments/opcode-gas/opcode_gas.py \
    experiments/opcode-gas/tests/test_higher_layer_calibration.py
  git commit -m "feat(zkgas): freeze native approximation policy"
  ```

---

### Task 2: Evaluate the approximation and resolve only nonzero dependencies

**Files:**

- Modify: `experiments/opcode-gas/opcode_gas.py:8898-9541`
- Modify: `experiments/opcode-gas/tests/test_higher_layer_calibration.py:336-610`

**Interfaces:**

- Consumes: `NativeTransferApproximationSpec`, validated round rows, existing exact row inventory,
  `_higher_layer_repeat_values`, `_higher_layer_stable_map`, and the frozen ordinary sweep gates.
- Produces: `_evaluate_native_transfer_approximation` returning `dict[str, Any]`,
  `_higher_layer_nonzero_dependencies` returning an ordered `tuple`, round
  `fixed_cost_statuses`, and terminal model status `accepted_with_declared_approximation`.

- [ ] **Step 1: Rewrite the synthetic accepted fixture to exercise the frozen coefficient**

  In `HigherLayerFixedRoundTests.rows`, set only the synthetic native term to
  `Decimal("5017")`. Leave `tx_base`, `block_base`, and `proposal_startup` unchanged. Update the
  accepted expectation to:

  ```python
  self.assertEqual(result["status"], "accepted_with_declared_approximation")
  self.assertEqual(result["decision"], "accepted")
  self.assertEqual(
      result["fixed_cost_statuses"],
      {
          "proposal_startup": "accepted",
          "block_base": "accepted",
          "tx_base": "accepted",
          "native_value_transfer": "declared_approximation",
      },
  )
  self.assertEqual(result["fixed_costs"]["native_value_transfer"], "5017")
  ```

- [ ] **Step 2: Add failing approximation and dependency tests**

  Add focused tests with these exact invariants:

  ```python
  def test_native_approximation_uses_target_total_materiality_and_all_frozen_counts(self):
      result = self.evaluate(self.rows(128), 128)
      evidence = result["overhead_results"]["native_value_transfer"]
      self.assertEqual(evidence["status"], "declared_approximation")
      self.assertEqual(evidence["o_p"], "5017")
      self.assertEqual(evidence["materiality_budget"], "0.002")
      self.assertEqual(
          [point["count"] for point in evidence["count_evidence"]],
          [1, 2, 4, 8, 16, 32, 64, 128],
      )
      self.assertTrue(all(point["materiality"] == "0" for point in evidence["count_evidence"]))

  def test_native_policy_expands_until_all_frozen_counts_exist(self):
      for bound in (8, 32):
          result = self.evaluate(self.rows(bound), bound)
          self.assertEqual(result["decision"], "expand_next_round")
          self.assertIn(
              "native_approximation_requires_bound_128",
              result["root_rejection_reasons"],
          )

  def test_zero_delta_dependency_does_not_block_block_or_startup(self):
      rows = self.rows(128)
      for row in rows:
          if row["case"] == "native_transfer_positive_vs_zero" and row["lane"] == "target":
              row["prover_gas"] = str(Decimal(row["prover_gas"]) - 4977 * row["target_count"])
      result = self.evaluate(rows, 128)
      self.assertEqual(result["overhead_results"]["native_value_transfer"]["status"], "rejected")
      self.assertEqual(result["overhead_results"]["block_base"]["status"], "accepted")
      self.assertEqual(result["overhead_results"]["proposal_startup"]["status"], "accepted")
  ```

  Rebind workload and execution identities after any mutation to identity-bound row fields. Add the
  converse test by declaring a nonzero `native_value_transfer` delta in the block case, rebinding
  those rows, and asserting `unmeasured_overhead_dependency` while native remains rejected.

  Add tamper/failure cases for: nonzero count with nonpositive delta, changed target repeat,
  changed control repeat, nonzero baseline delta, target total zero, materiality strictly above
  `0.002`, missing count, changed lane inventory, and unexpected native operation deltas.

- [ ] **Step 3: Run the fixed-round class and confirm the expected failures**

  Run:

  ```bash
  env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
    experiments.opcode-gas.tests.test_higher_layer_calibration.HigherLayerFixedRoundTests
  ```

  Expected: FAIL because the evaluator still fits native transfer with OLS, blocks zero-delta
  dependencies, and emits only ordinary `accepted` status.

- [ ] **Step 4: Implement exact native materiality evaluation**

  Add `_evaluate_native_transfer_approximation(manifest, rows,
  generator_max_count)`. It must:

  1. Select only `native_transfer_positive_vs_zero` rows.
  2. Require target/control rows for every `controlled_round_counts(generator_max_count)` count and
     repeat indices `0, 1, 2`.
  3. Require repeat-stable positive `prover_gas`, an exact zero target/control delta at count zero,
     empty expected and observed operation deltas, target features exactly
     `{"native_value_transfer": count, "tx_base": 0}`, and control features exactly `{}`. Every
     row must retain `operation_phase_ownership="transaction_non_anchor_only"`,
     `system_operation_ownership="block_base"`, and `anchor_operation_ownership="block_base"`.
  4. Require `target - control > 0` for every positive count.
  5. Compute every numeric value with `Decimal` and serialize it with `_decimal_text`:

     ```python
     observed_delta = target_prover_gas - control_prover_gas
     predicted_cost = coefficient * Decimal(count)
     absolute_error = abs(predicted_cost - observed_delta)
     materiality = absolute_error / target_prover_gas
     ```

  6. Emit one ordered `count_evidence` row with `count`, `target_prover_gas`,
     `control_prover_gas`, `observed_native_delta`, `predicted_native_cost`, `absolute_error`, and
     `materiality` for every required positive count present.
  7. At bounds 8 and 32, return `status="pending_required_counts"` and reason
     `native_approximation_requires_bound_128`; add that reason to
     `_HIGHER_LAYER_EXPANSION_REASONS`.
  8. At bound 128, return `status="declared_approximation"`, `o_p="5017"`, the exact source
     object, maximum materiality and its count when all points pass. Return `status="rejected"` and
     reason `native_approximation_materiality` if any point exceeds `0.002`.

  Never call `evaluate_controlled_sweep` for this case and never derive a replacement coefficient
  from current rows.

- [ ] **Step 5: Implement nonzero-dependency selection**

  Add:

  ```python
  def _higher_layer_nonzero_dependencies(
      rows: Iterable[Mapping[str, Any]],
      *,
      case_ids: Sequence[str],
      key_id: str,
      declared_dependencies: Sequence[str],
  ) -> tuple:
      selected = [
          row
          for row in rows
          if row.get("case") in case_ids and row.get("lane") == "target"
      ]
      if not selected:
          raise ValueError("higher-layer dependency panel is empty")
      expected_keys = {key_id, *declared_dependencies}
      required = []
      for row in selected:
          features = row.get("expected_feature_deltas")
          if not isinstance(features, Mapping) or set(features) != expected_keys:
              raise ValueError("higher-layer case feature declaration differs")
          if any(isinstance(value, bool) or not isinstance(value, int)
                 for value in features.values()):
              raise ValueError("higher-layer feature deltas must be integers")
          if features[key_id] != row.get("target_count"):
              raise ValueError("higher-layer target feature count differs")
      for dependency in declared_dependencies:
          if any(row["expected_feature_deltas"][dependency] != 0 for row in selected):
              required.append(dependency)
      return tuple(required)
  ```

  The implementation must validate that every target row has exactly
  `{key_id, *declared_dependencies}`, every feature value is an integer, and the key's own units
  retain the case's existing identity rule. Return dependencies in declared order only when at
  least one selected target row has a nonzero delta.

  Keep all declared dependency fields in `_higher_layer_sweep_case`, but subtract and require an
  accepted cost only when that row's dependency delta is nonzero. Use the helper for the block case
  and the two-case startup panel. A missing nonzero dependency emits
  `unmeasured_overhead_dependency`; a declared zero dependency is not read from `accepted`.

- [ ] **Step 6: Integrate the approximation into fixed-round status and decisions**

  Replace the native `fit_single` call with the approximation evaluator. Populate the resolved cost
  map with `Decimal("5017")` only when the approximation status passes. Emit:

  ```python
  fixed_cost_statuses = {
      "proposal_startup": "accepted",
      "block_base": "accepted",
      "tx_base": "accepted",
      "native_value_transfer": "declared_approximation",
  }
  ```

  only for resolved terms, ordered by `manifest.q_formula`. Define completeness as all four values
  resolved, all three ordinary statuses `accepted`, and native status `declared_approximation`.
  For a complete model return `status="accepted_with_declared_approximation"` and
  `decision="accepted"`; keep existing expansion and terminal behavior otherwise. Bump the round
  result schema to `2` because its semantics and fields changed.

- [ ] **Step 7: Run focused tests, inspect the full evaluator diff, and commit after review**

  Run:

  ```bash
  env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
    experiments.opcode-gas.tests.test_higher_layer_calibration.HigherLayerFixedRoundTests \
    experiments.opcode-gas.tests.test_higher_layer_calibration.HigherLayerCampaignInterfaceTests
  git diff --check
  git diff -- experiments/opcode-gas/opcode_gas.py \
    experiments/opcode-gas/tests/test_higher_layer_calibration.py
  ```

  Have an independent reviewer inspect denominator choice, exact counts, repeat stability, baseline,
  positive signal, no refit path, zero/nonzero dependency behavior, and decision termination. Fix
  confirmed findings, rerun the commands, then commit:

  ```bash
  git add experiments/opcode-gas/opcode_gas.py \
    experiments/opcode-gas/tests/test_higher_layer_calibration.py
  git commit -m "feat(zkgas): admit bounded native approximation"
  ```

---

### Task 3: Propagate approximation status through artifacts, state, replay, and sealing

**Files:**

- Modify: `experiments/opcode-gas/opcode_gas.py:21027-22910`
- Modify: `experiments/opcode-gas/tests/test_higher_layer_calibration.py:1080-2365`

**Interfaces:**

- Consumes: accepted v2 round payload containing `fixed_costs` and `fixed_cost_statuses`.
- Produces: schema-v2 fixed-cost and final-verdict artifacts whose four-value vector remains usable
  numerically while preserving the native term's `declared_approximation` provenance.

- [ ] **Step 1: Add failing fixed-artifact and state-verdict assertions**

  Update synthetic fixed fixtures to use native value `5017`, top-level status
  `accepted_with_declared_approximation`, and the exact status map below. In the state fixture,
  replace `observed_gas = 3000 + 340 * tx_count` with
  `observed_gas = 3000 + 5317 * tx_count` so absolute lane predictions contain both `tx_base=300`
  and `native_value_transfer=5017`; target/control deltas remain zero because both lanes have the
  same count.

  ```python
  fixed_cost_statuses = {
      "proposal_startup": "accepted",
      "block_base": "accepted",
      "tx_base": "accepted",
      "native_value_transfer": "declared_approximation",
  }
  ```

  Add assertions that:

  - `_higher_layer_fixed_cost_artifact` rejects a terminal fit labeled ordinary `accepted`;
  - `fixed_cost_rank` remains `4`, meaning four resolved forward-model dimensions, not four OLS
    coefficients;
  - `_higher_layer_fixed_cost_values` rejects a missing, changed, reordered, or relabeled status;
  - state output contains `fixed_model_status` and the exact four `fixed_cost_statuses`;
  - state prediction still uses numeric `5017`, and equal native counts cancel in each pair delta;
  - live replay rejects tampered policy evidence, per-count materiality, coefficient, budget,
    statuses, source hashes, target total, or observed delta;
  - sealed replay rejects the same mutations in `overhead-evidence.json` and rejects a final verdict
    that drops or relabels the approximation status.

- [ ] **Step 2: Run Task 5 artifact/state/sealing tests and confirm the expected failures**

  Run:

  ```bash
  env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest \
    experiments.opcode-gas.tests.test_higher_layer_calibration.HigherLayerTask5FixedCostTests \
    experiments.opcode-gas.tests.test_higher_layer_calibration.HigherLayerTask5StateVerdictTests
  ```

  Expected: FAIL because fixed, final, and sealed schemas still require ordinary `accepted` and do
  not validate status metadata.

- [ ] **Step 3: Make the fixed-cost artifact preserve both values and provenance**

  Set `_HIGHER_LAYER_FIXED_COST_SCHEMA_VERSION = 2`. In
  `_higher_layer_fixed_cost_artifact`, require exactly one terminal `decision="accepted"` whose fit
  has `status="accepted_with_declared_approximation"`, all four positive values in Q order, and the
  exact status map. Emit:

  ```python
  {
      "schema_version": 2,
      "status": "accepted_with_declared_approximation",
      "fixed_cost_rank": 4,
      "fixed_costs": canonical_costs,
      "fixed_cost_statuses": canonical_statuses,
      "fixed_costs_sha256": sha256_bytes(canonical_json(canonical_costs)),
      "source": source,
      "fit": dict(fit),
  }
  ```

  Retain identity, calibration ID, and selected round. Do not include status fields in
  `fixed_costs_sha256`; that digest continues to identify only the ordered numeric vector. Replay
  and the sealed package hash authenticate the complete artifact. Update
  `_higher_layer_fixed_cost_values` to validate both maps before returning numeric `Decimal` values.

- [ ] **Step 4: Carry the model status into the standalone state verdict**

  Bump the state-verdict schema to `2` and add:

  ```python
  "fixed_model_status": fixed["status"],
  "fixed_cost_statuses": dict(fixed["fixed_cost_statuses"]),
  ```

  Keep the state equations and thresholds unchanged. Explicitly test dirty-account pairs with equal
  `native_value_transfer` counts: their predicted lane totals include `5017 * count`, while their
  predicted target-minus-control delta is zero. Signature recovery, witness, dirty-state, and trie
  differences remain observable only through the measured residual and existing state verdict.

- [ ] **Step 5: Version and replay the portable package**

  Set `_HIGHER_LAYER_SEALED_SCHEMA_VERSION = 2`. Keep the exact four-file inventory. The existing
  portable verifier must reconstruct v2 rounds, rebuild the v2 fixed artifact, recompute the state
  verdict, and compare exact canonical JSON. Do not add a parallel approximation file: the v2
  manifest, selected fit, and embedded fixed artifact are the single provenance chain.

- [ ] **Step 6: Run the focused and complete Python suites**

  Run:

  ```bash
  env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
    -s experiments/opcode-gas/tests -p 'test_higher_layer_calibration.py'
  env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
    -s experiments/opcode-gas/tests -p 'test_*.py'
  git diff --check
  ```

  Expected: both suites PASS and `git diff --check` is silent. Record exact counts and skips for the
  later progress update.

- [ ] **Step 7: Obtain independent artifact/replay review, fix findings, and commit**

  The reviewer must try to make a changed coefficient, budget, source, target total, delta, status,
  or per-count evidence survive live and sealed replay; must confirm equal-count cancellation; and
  must verify no proposal/production entrypoint opened. After fixes and a fresh Step 6, commit:

  ```bash
  git add experiments/opcode-gas/opcode_gas.py \
    experiments/opcode-gas/tests/test_higher_layer_calibration.py
  git commit -m "fix(zkgas): bind approximation artifact status"
  ```

---

### Task 4: Update the operator contract and perform pre-campaign verification

**Files:**

- Modify: `experiments/opcode-gas/README.md:900-955`
- Test: `experiments/opcode-gas/tests/test_higher_layer_calibration.py`

**Interfaces:**

- Consumes: the v2 commands and artifact semantics from Tasks 1-3.
- Produces: an exact operator sequence that creates a new identity and cannot accidentally resume
  the rejected v1 run.

- [ ] **Step 1: Update the bounded higher-layer README section**

  Change the manifest argument to
  `experiments/opcode-gas/manifests/sp1-higher-layer-v2.json`. State explicitly:

  - bounds 8 and 32 remain expansion rounds until the complete native count set exists;
  - `5017` and `0.002` are frozen inputs, not fitted outputs;
  - a passing fixed round reports `accepted_with_declared_approximation` with decision `accepted`;
  - the old v1 run remains terminal rejected evidence and must not be resumed;
  - state commands are allowed only after fixed-artifact creation succeeds;
  - proposal validation and production promotion remain out of scope.

- [ ] **Step 2: Run formatting, focused Python, Rust workload, and repository hygiene checks**

  Run:

  ```bash
  cargo fmt --all -- --check
  cargo test -p guest-launcher --test controlled_workload
  env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
    -s experiments/opcode-gas/tests -p 'test_higher_layer_calibration.py'
  env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
    -s experiments/opcode-gas/tests -p 'test_*.py'
  git diff --check
  git status --short
  ```

  Expected: all checks PASS. Only intended source, manifest, test, README, and plan/progress files
  may be modified; generated run data may remain ignored.

- [ ] **Step 3: Perform final independent pre-campaign review and investigate findings**

  Give the reviewer the original approved design and the complete diff from the implementation base.
  Require checks for: target-total denominator, exact Decimal serialization, bound-128 requirement,
  no coefficient selection path, zero/nonzero dependency behavior, content-addressed source
  identity, state gating, live and portable replay, old-run preservation, and closed
  proposal/production scope.
  Fix every confirmed finding and ask the same reviewer to re-check it. Rerun Step 2 after the final
  fix.

- [ ] **Step 4: Commit the operator documentation**

  ```bash
  git add experiments/opcode-gas/README.md
  git commit -m "docs(zkgas): document native approximation campaign"
  ```

  Confirm `git status --short` is clean before beginning Task 5. The clean Git revision is part of
  the new calibration identity.

---

### Task 5: Execute, verify, and seal the fresh SP1 higher-layer campaign

**Files:**

- Create if all gates pass: `identity.json` in the content-addressed directory returned by the
  sealer under `experiments/opcode-gas/derivations/`
- Create if all gates pass: `overhead-evidence.json` in that returned directory
- Create if all gates pass: `state-holdout-evidence.json` in that returned directory
- Create if all gates pass: `higher-layer-calibration.json` in that returned directory
- Modify: `docs/plans/2026-09-26-zkgas-calibration-progress.md`

**Interfaces:**

- Consumes: a clean reviewed implementation revision, canonical v2 manifest, production launcher,
  proposal ELF/VK, operation coverage, and augmented Osaka core.
- Produces: either one explicit terminal higher-layer failure with immutable hashes, or one verified
  portable v2 derivation plus a precise progress record. Neither result opens proposal execution.

- [ ] **Step 1: Build the exact production launcher and capture executable hashes**

  Run:

  ```bash
  cargo build -r -p guest-launcher --features sp1-sdk/profiling
  sha256sum target/release/guest-launcher \
    crates/guests/elf/sp1_shasta_proposal.elf \
    crates/guests/elf/sp1_shasta_proposal.vk.bin
  git status --short
  ```

  Expected: build succeeds and the worktree remains clean. Record all three hashes.

- [ ] **Step 2: Create a new identity and execute only the bounded overhead campaign**

  In one shell, run:

  ```bash
  RUN_PATH_FILE="$(mktemp)"
  ~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
    prepare-higher-layer-calibration \
    --manifest experiments/opcode-gas/manifests/sp1-higher-layer-v2.json \
    --operation-coverage experiments/opcode-gas/manifests/operation-coverage-v1.json \
    --augmented-core \
      experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/core-opcode-submodel.json \
    --guest-launcher target/release/guest-launcher \
    --elf crates/guests/elf/sp1_shasta_proposal.elf \
    --out experiments/opcode-gas/runs \
    --run-path-file "$RUN_PATH_FILE"
  HIGHER_LAYER_RUN="$(<"$RUN_PATH_FILE")"
  ~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
    run-higher-layer-calibration --run "$HIGHER_LAYER_RUN"
  jq '{schema_version,status,decision,fixed_costs,fixed_cost_statuses,root_rejection_reasons}' \
    "$HIGHER_LAYER_RUN/fit/overhead-round-128.json"
  ```

  Required ledger sequence is bounds 8, 32, 128. Bounds 8 and 32 must be
  `expand_next_round`. Continue only if bound 128 is `decision="accepted"`,
  `status="accepted_with_declared_approximation"`, ordinary statuses are `accepted`, native status
  is `declared_approximation`, and maximum native materiality is at most `0.002`.

  If bound 128 is terminal failure, do not run fitting or state commands. Record the run identity,
  three raw/fit hashes, full rejection reasons, and materiality evidence in progress, obtain an
  independent replay check, commit only that progress update, and stop this plan.

- [ ] **Step 3: Create and inspect the fixed-cost artifact**

  Run only after the accepted condition in Step 2:

  ```bash
  ~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
    fit-higher-layer-fixed-costs --run "$HIGHER_LAYER_RUN"
  jq '{
    schema_version,
    status,
    selected_round,
    fixed_cost_rank,
    fixed_costs,
    fixed_cost_statuses,
    source
  }' "$HIGHER_LAYER_RUN/fixed-costs.json"
  ```

  Expected: schema `2`, status `accepted_with_declared_approximation`, selected round `128`, rank
  `4`, native value `5017`, and the exact status map. Any mismatch is terminal; do not open state.

- [ ] **Step 4: Run the six frozen state pairs and finalize their verdict**

  Run:

  ```bash
  ~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
    run-higher-layer-state-holdouts --run "$HIGHER_LAYER_RUN"
  ~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
    finalize-higher-layer-calibration --run "$HIGHER_LAYER_RUN"
  jq '{schema_version,fixed_model_status,fixed_cost_statuses,coarse_state_trie,state_holdouts}' \
    "$HIGHER_LAYER_RUN/higher-layer-calibration.json"
  ```

  Require exactly 36 accepted execution rows: six pairs, two lanes, three repeats. A verdict of
  `coarse_model_accepted` permits sealing. `needs_state_split` or `inconclusive` is evidence, not a
  tuning opportunity: record it, do not alter `5017` or `0.002`, do not seal an accepted coarse
  result, and stop before proposal work.

- [ ] **Step 5: Independently verify the live campaign and seal the portable derivation**

  First have an independent tester run semantic replay against the same run-path file and compare
  round hashes, materiality rows, fixed statuses, all state rows, and the final verdict. Then run:

  ```bash
  ~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
    verify-higher-layer-calibration --run-path-file "$RUN_PATH_FILE"
  DERIVATION_PATH_FILE="$(mktemp)"
  ~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
    seal-higher-layer-calibration \
    --run "$HIGHER_LAYER_RUN" \
    --out-root experiments/opcode-gas/derivations \
    --derivation-path-file "$DERIVATION_PATH_FILE"
  DERIVATION_PATH="$(<"$DERIVATION_PATH_FILE")"
  printf '%s\n' "$DERIVATION_PATH"
  find "$DERIVATION_PATH" -maxdepth 1 -type f -printf '%f\n' | sort
  ```

  Expected: semantic replay succeeds and the directory contains exactly the four files listed in
  this task. Run verification a second time with a run-path file containing `DERIVATION_PATH` to
  prove portable replay rather than live-run replay.

- [ ] **Step 6: Update progress from exact evidence**

  Record in `docs/plans/2026-09-26-zkgas-calibration-progress.md`:

  - implementation revision and v2 manifest artifact/file hashes;
  - launcher, ELF, and VK hashes;
  - calibration ID and full identity hash;
  - each bound's decision, raw SHA256, fit SHA256, and root reasons;
  - all eight native materialities, their maximum/count, coefficient, budget, and status;
  - accepted ordinary fixed costs and all four statuses;
  - fixed-cost digest and selected round;
  - each state pair verdict and aggregate coarse-state verdict;
  - derivation ID and all four sealed file hashes when sealing succeeded;
  - exact Python/Rust validation commands and results;
  - explicit statements that proposal execution and production changes remained closed.

- [ ] **Step 7: Run final verification and commit the derivation/progress result**

  Run:

  ```bash
  env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
    -s experiments/opcode-gas/tests -p 'test_higher_layer_calibration.py'
  env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
    -s experiments/opcode-gas/tests -p 'test_*.py'
  cargo test -p guest-launcher --test controlled_workload
  git diff --check
  git status --short
  ```

  Ask the independent reviewer and tester to re-check any fixes and the final evidence package.
  Inspect the complete diff. For a sealed success, commit the new derivation directory and progress:

  ```bash
  git add experiments/opcode-gas/derivations docs/plans/2026-09-26-zkgas-calibration-progress.md
  git commit -m "data(zkgas): seal higher-layer calibration"
  ```

  For a terminal overhead or state result, commit only the progress document:

  ```bash
  git add docs/plans/2026-09-26-zkgas-calibration-progress.md
  git commit -m "docs(zkgas): record higher-layer calibration result"
  ```

  Do not push or open a PR. Report the exact terminal status, verification evidence, independent
  review outcome, and the next unopened gate.
