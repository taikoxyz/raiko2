# Context Approximation And Block Comparison Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert the six strictly rejected production-context families into explicit conservative
approximations, retain SP1 low-level diagnostic evidence, and accept the resulting review-only
estimator only when it outperforms the current Unzen schedule on an untouched controlled-block
validation partition without regressing underprediction tails.

**Architecture:** Preserve strict result `a41befb63e663890896ba67d` byte-for-byte. Derive context
prices from same-production-guest target/control differences plus the canonical V5 cost of the
lane-exclusive replacement instruction,
never from discovery-ELF coefficients or SP1 diagnostics. Store RISC-V opcode, syscall, and memory
metrics in a separately content-addressed diagnostic sidecar. Promote declared approximations into
a new review-only operation coverage/estimator lineage, then compare it with a scalar-normalized
Unzen `finalized_block_zkgas` baseline on frozen controlled blocks.

**Tech Stack:** Python 3 via `~/.venv`, exact `Decimal`/`Fraction` arithmetic, canonical JSON/JSONL,
SHA256 content addressing, Rust guest-launcher output, SP1 6.3.0 gas-estimator, pytest/unittest, and
the existing schema-4 host-native trace.

**Spec:** `docs/plans/2026-09-26-zkgas-calibration-design.md`

## Global Constraints

- Result `a41befb63e663890896ba67d`, V5 coverage, historical opcode registry, higher-layer artifacts,
  guest ELF/VK files, and their verdicts remain immutable.
- The only permitted strict source has full identity
  `a41befb63e663890896ba67de48a51e65de49c4b281229a37b0e271a094e1943`. Its approximation identity
  binds `result.json` SHA256 `634783f4518151ad16aa5f92022f54cb1e5d258e85a5b16196159bfdc9c285a2`,
  `rows.jsonl` SHA256 `651e7423330b428f346306c8dda52a7c2dff3fd5ae15f82bed02bdbaf7154c06`,
  every other filename/hash in `result_identity.file_sha256s`, calibration identity
  `138ef55b8c1f9d48cc92e98fb17c7976d1bf86a6df37f851e5284668ce92ff1b`, manifest identity
  `00c375a82a9a5b98a7011af26e603ab53c9779fae83199f81100154382c8be50`, and the V5/higher-layer
  identities carried by the sealed source files. A short-prefix match is insufficient.
- The strict six-family result remains rejected; successor parameters are always labelled
  `declared_approximation`.
- Approximation coefficients use only production-guest target/control rows and the canonical V5
  marginal cost of the lane-exclusive replacement instruction: one `PUSH0`, or one `SWAP1` for
  `CALLDATALOAD`. Common setup and cleanup are excluded. No context-ELF number, global ELF bridge,
  validation row, current Unzen multiplier, or SP1 diagnostic count enters a coefficient.
- `CALLDATASIZE` is constant per executed opcode in this successor. Calldata ingestion remains
  transaction-owned and is not multiplied by the number of `CALLDATASIZE` executions.
- SP1 RISC-V opcode counts, syscall counts, and touched-memory totals are diagnostic outputs only;
  online estimation remains host-computable before guest execution.
- All candidate and Unzen-normalization parameters are sealed before the controlled-block
  validation partition is opened. Validation cannot fit an offset, scalar, coefficient, fallback,
  feature, threshold, or ownership rule.
- The old strict operation holdout is already open and may be reused only as approximation
  training/diagnostic evidence. Acceptance uses a newly frozen untouched block validation split.
- The final 60-proposal corpus remains unopened. No production schedule, zkGas limit, runtime,
  deployment, Boundless configuration, or generated guest artifact is modified.

---

### Task 1: Freeze The Approximation And Comparative Acceptance Contract

**Files:**

- Modify: `docs/plans/2026-09-26-zkgas-calibration-design.md`
- Modify: `docs/plans/2026-09-26-zkgas-calibration-progress.md`
- Modify: `docs/plans/2026-09-27-zkgas-composite-block-estimator-implementation-plan.md`
- Modify: `docs/plans/2026-09-29-zkgas-context-production-calibration-implementation-plan.md`
- Create: `docs/plans/2026-09-30-zkgas-context-approximation-implementation-plan.md`

**Interfaces:**

- Produces the binding approximation equation, metric ownership boundary, Unzen normalization,
  error definitions, and comparative block acceptance gates consumed by Tasks 2-4.

- [ ] **Step 1: Record the same-guest approximation equation**

  Freeze `D_s(n)`, lane-exclusive V5 replacement-cost addition, nonnegative class maximum, and the
  requirement that strict rejection evidence remains visible.

- [ ] **Step 2: Record the comparative block gate**

  Freeze `kappa_unzen`, SPE/APE/UPE formulas, nearest-rank quantiles, complete coverage, strictly
  improved MAPE, non-regressed maximum APE, mean SPE `>= -0.05`, p95 UPE `<= 0.10`, and maximum UPE
  `<= 0.20`; both UPE metrics must also be no worse than Unzen.

- [ ] **Step 3: Verify and commit**

  Run `git diff --check`, inspect the complete docs diff, then commit with
  `docs(zkgas): define comparative block acceptance`.

---

### Task 2: Capture Replayable SP1 Diagnostics Without Changing Canonical Rows

**Files:**

- Create: `experiments/opcode-gas/context_approximation.py`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_context_production.py`
- Modify: `experiments/opcode-gas/README.md`

**Interfaces:**

- Produce `extract_sp1_diagnostics(report) -> dict[str, object]` with exactly
  `total_instruction_count`, `total_syscall_count`, `touched_memory_addresses`, `opcode_counts`,
  and `syscall_counts`.
- Produce `build_context_diagnostic_sidecar(result, reports) -> dict[str, object]`, keyed by the
  immutable source row IDs and source result identity.
- Add `run-context-diagnostics --result --launcher --out` and
  `verify-context-diagnostics --diagnostics`. The runner reconstructs only a frozen 88-row panel
  from source portable inputs, always using repeat index zero and both target/control lanes. It runs
  counts `0,16` for these 11 fit scenarios: `address_canonical`, `caller_canonical`,
  `callvalue_zero`, `callvalue_nonzero_7`, `calldataload_empty_offset_0`,
  `calldataload_out_of_range_4_offset_64`, `calldataload_partial_33_offset_17`,
  `calldataload_full_32_offset_0`, `calldatasize_0`, `calldatasize_64`, and
  `timestamp_post_unzen_delta_17`. It runs counts `0,64` for these 11 final-holdout scenarios:
  `address_alternate`, `caller_alternate`, `callvalue_zero_calldata_1`,
  `callvalue_nonzero_4294967297`, `calldataload_empty_offset_1`,
  `calldataload_out_of_range_96_offset_128`, `calldataload_partial_31_offset_30`,
  `calldataload_full_96_offset_32`, `calldatasize_15`, `calldatasize_127`, and
  `timestamp_post_unzen_delta_86400`. It never modifies or reseals the source result.

- [ ] **Step 1: Write failing diagnostic-schema tests**

  Cover the exact 88-row selector, sorted unique labels, nonnegative integer counts, opcode/syscall
  totals matching their declared totals, duplicate labels, missing fields, floats, source-row
  mismatch, and source-result mismatch. Assert that no diagnostic field appears in a coefficient
  input.

- [ ] **Step 2: Run the focused test and observe the missing API failure**

  Run `~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_context_production.py -q`.

- [ ] **Step 3: Implement the diagnostic extractor and create-only sidecar**

  Reconstruct the selected source GuestInputs, execute the unchanged production ELF with the SP1
  gas-estimator, canonicalize count entries as sorted `{label, count}` arrays, bind the complete
  source result identity plus launcher/ELF/report hashes, and reject an existing output path. Keep
  source `rows.jsonl` bytes unchanged. Persist each completed row before continuing so the bounded
  88-row run resumes exactly after interruption.

- [ ] **Step 4: Run focused and complete Python tests**

  Run the focused test, `~/.venv/bin/python -m pytest experiments/opcode-gas/tests -q`,
  `~/.venv/bin/python -m py_compile experiments/opcode-gas/*.py`, and `git diff --check`.

- [ ] **Step 5: Commit**

  Commit with `feat(zkgas): retain context SP1 diagnostics` after independent review.

---

### Task 3: Seal Six Declared Context Approximations

**Files:**

- Modify: `experiments/opcode-gas/context_approximation.py`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_context_production.py`
- Create after verification: one content-addressed directory under
  `experiments/opcode-gas/derivations/`
- Modify after result: `docs/plans/2026-09-26-zkgas-calibration-progress.md`

**Interfaces:**

- Produce `build_declared_context_approximation(manifest, rows, subtotal_model) -> dict`.
- For scenario `s` and nonzero count `n`, compute the paired increment
  `D_s(n) = (P_t(n)-P_c(n))-(P_t(0)-P_c(0))` from exact-repeat rows.
- Fit a through-origin scenario slope using exact `Fraction` arithmetic. Add the per-event canonical
  V5 cost of exactly one lane-exclusive replacement instruction. Remove `n` target measurement
  events from the target ledger and `n` replacement events from the control ledger, then require the
  remaining raw-gas and event-count ledgers to be identical. Group `ADDRESS`, `CALLER`,
  `CALLDATASIZE`, and `TIMESTAMP` as constants; group `CALLVALUE` by `zero|nonzero`; group
  `CALLDATALOAD` by `zero|partial|full`. Select `max(0, max(scenario_costs))` for each class.
- Emit each controlled row's predicted marginal, observed target-control marginal, error, and
  `whole_guest_materiality = abs(error) / P_target(n)` using `Decimal`.
- Produce create-only `seal-context-approximation --source-result --out-root` and
  `verify-context-approximation --approximation` commands.

- [ ] **Step 1: Write failing pairing, pricing, and provenance tests**

  Cover the exact full source identity and all source file hashes, rejection of a different result
  or matching short prefix, count-zero subtraction, exact target/control pairing, missing/duplicate
  repeats, lane-exclusive replacement V5 addition exactly once, and rejection when the residual
  ledgers differ. Cover the `CALLDATALOAD` case where one shared cleanup `SWAP1` cancels while only
  the second control `SWAP1` is added back. Also cover negative-floor behavior, conservative class
  maximum, constant `CALLDATASIZE`, class inventory, immutable strict rejection references,
  forbidden discovery coefficients, deterministic content identity, tamper rejection, and
  create-only collision.

- [ ] **Step 2: Run the focused test and observe the missing approximation failure**

  Run `~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_context_production.py -q`.

- [ ] **Step 3: Implement the minimum exact approximation builder and verifier**

  Reuse `ProductionContextManifest`, `_normalized_fit_row`, `ProductionSubtotalModel`, and
  `evaluate_v5_subtotal`; do not duplicate V5 registry evaluation. Persist full-precision fractions
  and decimal renderings plus all rejected strict-family reasons.

- [ ] **Step 4: Build from the exact strict source, verify, and independently review**

  Require full source identity
  `a41befb63e663890896ba67de48a51e65de49c4b281229a37b0e271a094e1943`, seal once from its
  immutable rows, replay from a clean checkout, and obtain independent adversarial review plus
  independent arithmetic recomputation before committing the artifact.

- [ ] **Step 5: Run the complete Python suite and commit**

  Run `~/.venv/bin/python -m pytest experiments/opcode-gas/tests -q`, Python compilation, and
  `git diff --check`. Commit source before the separately reviewed artifact and progress update.

---

### Task 4: Compare The Review-Only Candidate With Current Unzen On Frozen Blocks

**Files:**

- Modify: `experiments/opcode-gas/composite_estimator.py`
- Modify: `experiments/opcode-gas/context_approximation.py`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_sp1_composite_estimator.py`
- Modify: `experiments/opcode-gas/tests/test_context_production.py`
- Create: `experiments/opcode-gas/manifests/sp1-block-comparison-v1.json`
- Create and seal before any block execution: one content-addressed fully evaluable candidate under
  `experiments/opcode-gas/derivations/`
- Create after calibration: one content-addressed Unzen-normalization directory under
  `experiments/opcode-gas/derivations/`
- Create after validation: one content-addressed comparison directory under
  `experiments/opcode-gas/derivations/`
- Modify after result: `docs/plans/2026-09-26-zkgas-calibration-progress.md`

**Interfaces:**

- Extend the review-only estimator with a separate `declared_context_approximation` section; do not
  rewrite the historical opcode registry.
- Produce a create-only fully evaluable candidate before block execution. Its identity binds V5,
  declared approximation, operation coverage, fixed/higher-layer artifacts, estimator source,
  trace schema, launcher, and production ELF/VK.
- Produce `fit_unzen_scalar(calibration_rows) -> Decimal` as the exact median of
  `observed_prover_gas / finalized_block_zkgas`; reject zero zkGas and non-calibration rows.
- Produce `compare_block_models(validation_rows, kappa_unzen) -> dict` using exact SPE/APE/UPE and
  nearest-rank p95 UPE.

- [ ] **Step 1: Write failing estimator and comparative-gate tests**

  Cover every context class, constant `CALLDATASIZE`, no double charging, missing typed input,
  complete coverage, calibration/validation isolation, even-length exact median, Decimal-only
  arithmetic, nearest-rank p95, strict MAPE improvement, max-APE regression, signed-bias failure,
  p95/max underprediction failures, and changed candidate/manifest/source hashes.

- [ ] **Step 2: Implement and seal the review-only estimator extension**

  Keep candidate evaluation deterministic and side-effect free. Seal its complete identity before
  either block partition executes. Unzen normalization later reads only the calibration partition;
  comparison reads only the sealed scalar, frozen candidate, and untouched validation rows.

- [ ] **Step 3: Freeze the controlled-block manifest before execution**

  Use separate calibration and validation workload IDs, cover simple arithmetic, context-heavy,
  calldata-boundary, memory-expansion, storage, mixed transaction, and native-transfer blocks, and
  bind Osaka execution, Unzen schedule, production ELF/VK, launcher, trace schema, all source
  artifacts, and the exact sealed candidate identity. The manifest must contain no final-proposal
  ID, and every emitted row must repeat the candidate identity.

- [ ] **Step 4: Execute and seal calibration, then seal Unzen normalization**

  Execute only the calibration split with bounded resume. Seal and replay its rows, compute the
  exact-median `kappa_unzen`, and create-only seal a normalization artifact that binds the candidate,
  manifest, calibration rows, formula, and thresholds. Do not execute a validation row yet.

- [ ] **Step 5: Execute untouched validation and seal the comparison**

  Require the exact candidate and normalization identities before execution. Run only the validation
  split, bind both identities into every row, compute the frozen comparison once, and seal/replay the
  rows and report. Obtain independent adversarial review and behavioral verification.

- [ ] **Step 6: Run final checks and commit the milestone**

  Run the focused Python suites, the full opcode-gas suite, `cargo test -p raiko2-zkgas-trace`,
  `cargo test -p guest-launcher --test controlled_workload`, Python compilation, formatting, and
  `git diff --check`. Record exact results without changing production or opening final proposals.
