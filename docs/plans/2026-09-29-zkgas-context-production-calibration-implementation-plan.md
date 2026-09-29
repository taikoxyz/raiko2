# Feature Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fit production-proposal-guest SP1 `proverGas` functions for `ADDRESS`, `CALLER`,
`CALLVALUE`, `CALLDATALOAD`, `CALLDATASIZE`, and `TIMESTAMP`, promote only passing complete
families into operation coverage V6 and a review-only composite estimator, and preserve all failed
or partial evidence without changing the production zkGas schedule.

**Architecture:** Keep the sealed context-opcode ELF result as function-shape discovery only. Add
minimal host-native typed inputs to the existing `raiko2-zkgas-trace` observer, construct paired
synthetic blocks that execute the unchanged production SP1 proposal guest, and fit each target from
its own count-differenced production residual. The historical V5 registry prices only known
non-target work; no context-ELF slope, cross-ELF scale, `body_scale`, or historical control body is
used to recover a target coefficient. Store passing context functions in a separate typed model,
like typed storage, rather than rewriting the immutable historical opcode registry.

**Tech Stack:** Rust, REVM `41.0.0`, the existing `raiko2-zkgas-trace` inspector, SP1 `6.3.0`
`ExecutionReport::gas`, the production Shasta proposal ELF, Python 3 from `~/.venv`, exact
`Decimal`/`Fraction` arithmetic, canonical JSON/JSONL, SHA256 content addressing, Cargo,
`unittest`/pytest, and `just build-guest sp1`.

**Spec:** `docs/plans/2026-09-26-zkgas-calibration-design.md`,
`docs/plans/2026-09-29-zkgas-context-operation-implementation-plan.md`, and the `Next Gate` in
`docs/plans/2026-09-26-zkgas-calibration-progress.md`.

## Global Constraints

- The sealed discovery result at
  `experiments/opcode-gas/derivations/79dcfe2d5be2c3432987a671/` is immutable and permanently
  non-candidate. It may justify the feature vocabulary, but none of its numeric coefficients enter
  this fit.
- Every measured row executes `crates/guests/elf/sp1_shasta_proposal.elf` through
  `guest-launcher --stage controlled-block --proof-type sp1 --mode execute --sp1-prover local
  --sp1-execution-engine gas-estimator`.
- The trace observer remains host-native. Do not add instrumentation to the production guest, copy
  Alethia execution/filtering logic, or change an Alethia dependency pin for this campaign.
- Use Osaka REVM execution semantics under the actual Unzen schedule identity and Fusaka opcode
  vocabulary. Persist all three axes independently.
- Keep the historical opcode registry and operation coverage V5 byte-for-byte unchanged. Context
  costs are a separate typed model whose units are production-guest SP1 `proverGas` per executed
  event.
- A context target coefficient is derived only from target-lane production rows. The paired control
  lane is a contamination and residualization gate; its historical cost is not algebraically added
  to the target coefficient.
- For every scenario and count, target and control fixtures have identical block, transaction,
  state, calldata/value/timestamp, bytecode length, fixed-width count encoding, stack height, and
  non-target operation ledger. Only the declared measurement instruction differs.
- Counts are frozen at fit `[0, 1, 2, 4, 8, 16]`, count holdout `32`, and extrapolation checkpoint
  `64`. Every executed row has exactly three repeats. There is no adaptive count expansion.
- The final scenario holdout is never used to choose a model, coefficient, threshold, bucket, or
  fallback. `CALLDATASIZE` has a separate predeclared model-selection split before the final
  holdout.
- Fit and validation arithmetic uses `Decimal` or exact `Fraction`, never binary `float`.
- A family is promotable only when every required class/sibling passes exact event matching,
  control-lane contamination, repeat, signal, fit, count holdout, extrapolation, and scenario
  holdout gates. Partial application within one typed class is forbidden.
- A failed family remains an explicit gap. Do not replace it with the current Unzen multiplier, a
  discovery-ELF projection, a global ELF bridge, or an upper-layer offset.
- A count-, value-, length-, or range-correlated residual returns to the context operation family.
  Only a workload-independent residual may be investigated later at transaction or block scope.
- Do not open the final 60-proposal validation corpus. After a reviewed V6 candidate exists, only
  the two already-open ad-hoc diagnostics may be replayed, once, without tuning.
- Do not edit the production multiplier table, integer scale, block zkGas limit, runtime config,
  deployment config, Boundless config, or generated guest ELF files by hand.
- Persist only repository-relative paths. Generated run directories remain ignored; only a passing
  or explicit fail-closed create-only result package may be committed.

## Frozen Production Model Vocabulary

The trace emits one typed input for each of the six opcodes:

| Opcode | Trace input | Candidate cost function |
| --- | --- | --- |
| `ADDRESS` | `context_fixed` | `address_constant` |
| `CALLER` | `context_fixed` | `caller_constant` |
| `CALLVALUE` | `context_value { value_class }` | `callvalue_zero` or `callvalue_nonzero` |
| `CALLDATALOAD` | `calldata_load { access_class }` | `load_zero`, `load_partial`, or `load_full` |
| `CALLDATASIZE` | `calldata_size { input_length }` | primary affine length model; frozen boundary fallback |
| `TIMESTAMP` | `context_value { value_class }` | `timestamp_nonzero`; zero is unreachable post-Unzen |

`value_class` is exactly `zero | nonzero`; the nonzero holdout uses a materially different value
to test whether this coarse class is sufficient. `access_class` is exactly `zero | partial | full`,
where zero means no calldata byte is available at the requested offset, partial means `1..31`
bytes are available, and full means at least 32 bytes are available. Empty calldata and a nonempty
fully out-of-range read are required siblings of the same zero class.

The fixed one-output target/control pairs use measured `PUSH0` for `ADDRESS`, `CALLER`,
`CALLVALUE`, `CALLDATASIZE`, and `TIMESTAMP`. `CALLDATALOAD` keeps the same two-item stack setup and
uses measured `SWAP1`, followed by identical cleanup. Those controls have matching instruction
width and raw EVM gas, but their stored historical bodies remain diagnostic inputs only and never
appear in the target-fit equation below.

The `CALLDATASIZE` candidates are frozen before execution:

```text
M_length(L)   = beta_0 + beta_length * L
M_boundary(L) = beta_0
              + beta_words * ceil(L / 32)
              + beta_partial * I(L mod 32 != 0)
```

Fit both candidates on the fit split. Evaluate them on the model-selection split in fixed order:
select `M_length` if it passes all gates, otherwise select `M_boundary` if it passes, otherwise
reject the family. Only the selected model is then evaluated on the untouched final holdout. Never
switch models after the final holdout is opened.

## Frozen Scenario Matrix

All scenario names below are semantic enums in the manifest and Rust fixture builder, not free-text
instructions interpreted by the runner.

| Family | Fit contexts | Selection contexts | Final holdout contexts |
| --- | --- | --- | --- |
| `ADDRESS` | canonical controlled contract address | none | alternate controlled contract address |
| `CALLER` | canonical deterministic signer | none | alternate deterministic signer |
| `CALLVALUE` | `0`, `7` | none | `4_294_967_297` for the nonzero class |
| `CALLDATALOAD` | empty/offset 0, 32 bytes/offset 0, 33 bytes/offset 17, 4 bytes/offset 64 | none | 31 bytes/offset 30, 96 bytes/offset 32 |
| `CALLDATASIZE` | lengths `0, 1, 31, 32, 33, 64` | lengths `2, 63, 65, 96` | lengths `15, 47, 127, 255` |
| `TIMESTAMP` | post-Unzen timestamp delta `17` | none | post-Unzen timestamp delta `86_400` |

Fit contexts run counts `0, 1, 2, 4, 8, 16`. Selection and final-holdout contexts run `0, 32,
64`; the zero row establishes the context-specific intercept without fitting it into the operation
cost. The alternate address/caller profiles, transaction values, calldata, and timestamps are
resolved by the Rust builder and bound into the workload identity and backend-input hash.

`TIMESTAMP=0` cannot occur in a valid post-Unzen production block. The result records it as
`unreachable_under_version_identity`, not measured, zero-priced, or inferred from the discovery
ELF. If the version identity later admits a zero timestamp, that is a new campaign.

## Frozen Fit And Gate Equations

Let `P_s(n)` be production-guest `proverGas` for target scenario `s` at count `n`. Let `K_s(n)` be
the frozen V5 composite subtotal for all higher-layer and non-target operation work in that same
fixture, explicitly excluding the target context opcode. The observed target increment is:

```text
Y_s(n) = [P_s(n) - P_s(0)] - [K_s(n) - K_s(0)]
Y_hat_s(n) = n * f_s(context_features)
```

The fit is through the origin after the count-zero difference. No context-ELF term, `body_scale`,
control-body term, proposal intercept, transaction intercept, or block intercept appears in this
equation.

For the paired control lane, `K_control_s(n)` includes every operation because the control opcode is
already measured in V5:

```text
C_s(n) = [P_control_s(n) - P_control_s(0)]
       - [K_control_s(n) - K_control_s(0)]
```

`C_s(n)` is never added to `Y_s(n)`. It must remain below the frozen contamination gate; otherwise
the scenario is rejected because code/input/envelope work is still count-correlated.

Quality gates are:

- three byte-identical repeats with identical `proverGas`, public output, input hash, and host trace;
- signal at count `16` at least `max(1000, 20 * 143)` proverGas;
- fit `R2 >= 0.995`;
- relative coefficient standard error `<= 0.05` for every nonzero coefficient;
- maximum fit residual divided by the maximum observed signal `<= 0.02`;
- control-lane `max(abs(C_s(n))) <= max(1000, 20 * 143)`;
- count-32 and count-64 APE each `<= 0.10`;
- final-scenario per-row APE `<= 0.10` and family MAPE `<= 0.05`;
- coefficients and every tested predicted event cost are finite and nonnegative;
- siblings mapped to one class differ by at most 5% in independently fitted diagnostic slopes.

All APE values use:

```text
APE = abs(predicted_increment - observed_increment) / abs(observed_increment)
```

The signal gate runs before APE, so a zero denominator is a rejection rather than a substituted
epsilon.

---

### Task 1: Freeze the production campaign contract

**Files:**

- Create: `experiments/opcode-gas/manifests/sp1-context-production-v1.json`
- Create: `experiments/opcode-gas/context_production_campaign.py`
- Create: `experiments/opcode-gas/tests/test_context_production.py`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/README.md`

**Interfaces:**

- Add strict `ProductionContextManifest`, `ProductionContextScenario`, and model-candidate parsing.
- Add deterministic row/workload IDs from canonical semantic content.
- Add a thin `opcode_gas.py` CLI dispatch for prepare/run/fit/verify/seal operations; keep domain
  logic in `context_production_campaign.py`.
- Bind operation coverage V5, the sealed higher-layer artifact, the sealed discovery result, trace
  schema source, launcher, production ELF/VK, implementation revision, and version identity.

- [ ] **Step 1: Write failing manifest tests**

  Cover the exact six-key inventory, all contexts/counts/splits, exactly three repeats, exact model
  selection order, exact formulas/gates, post-Unzen timestamp semantics, required zero/out-of-range
  siblings, and rejection of unknown fields, duplicate rows, overlapping splits, undeclared model
  terms, binary floats, absolute paths, or a discovery coefficient.

- [ ] **Step 2: Run the focused tests and observe the intended failure**

  ```bash
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_context_production.py -q
  ```

  Failures must be missing production-manifest/parser behavior, not missing dependencies.

- [ ] **Step 3: Implement strict parsing and canonical IDs**

  Reject any manifest that can choose a feature or threshold after rows are opened. Validate the
  exact pinned source artifacts from disk, including canonical bytes and content identities.

- [ ] **Step 4: Add README command skeletons without claiming results**

  Document only implemented command names, their create-only outputs, and the boundary between
  discovery and production coefficients. Do not write future result IDs or example absolute paths.

- [ ] **Step 5: Verify and commit Task 1**

  ```bash
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_context_production.py -q
  ~/.venv/bin/python -m py_compile experiments/opcode-gas/context_production_campaign.py
  git diff --check
  ```

  Inspect the complete Task 1 diff, then commit:

  ```bash
  git add experiments/opcode-gas
  git commit -m "feat(zkgas): freeze production context campaign"
  ```

---

### Task 2: Emit typed context inputs from the host-native production trace

**Files:**

- Modify: `crates/zkgas-trace/src/inspector.rs`
- Modify: `crates/zkgas-trace/src/lib.rs`
- Modify: `crates/zkgas-trace/tests/inspector.rs`
- Modify: `crates/zkgas-trace/tests/reconstruction.rs`
- Modify: `experiments/opcode-gas/tests/test_sp1_composite_estimator.py`

**Interfaces:**

- Add serde enums for `ContextValueClass` and `CalldataLoadAccessClass`.
- Extend `OpcodeModelInput` with exact `context_fixed`, `context_value`, `calldata_load`, and
  `calldata_size` variants.
- Capture `CALLVALUE` from the current interpreter frame's `InputsTr::call_value()`, calldata from
  the current frame's `CallInput`, and `TIMESTAMP` from the actual block environment.
- Classify a `CALLDATALOAD` offset larger than the host index range as zero available bytes, not a
  feature error. Do not serialize the calldata bytes or stack word into every operation row.
- Bump the proposal operation trace schema from 3 to 4. Existing sealed schema-3 artifacts remain
  immutable and are not relabeled.

- [ ] **Step 1: Write failing serde and live-inspector tests**

  Cover both fixed opcodes, zero/nonzero value, zero/partial/full calldata load, calldata sizes at
  0/31/32/33, post-Unzen timestamp, nested-frame calldata/value semantics, oversized load offset,
  exact JSON fields, and wrong opcode/input combinations.

- [ ] **Step 2: Run the focused Rust tests and observe the intended failure**

  ```bash
  cargo test -p raiko2-zkgas-trace --test inspector
  cargo test -p raiko2-zkgas-trace --test reconstruction
  ```

- [ ] **Step 3: Implement capture and fail-closed validation**

  Extend the existing `PendingOpcodeModelInput` path; do not add a second inspector or perform a
  second EVM execution. Preserve post-step charging semantics and all storage/memory typed inputs.

- [ ] **Step 4: Update trace-schema consumers to reject stale/malformed inputs**

  Existing estimator schemas remain able to validate their sealed documents, but applying them to
  a schema-4 trace must fail on the trace-version join. The later Task 7 estimator is the first
  schema-4 consumer.

- [ ] **Step 5: Verify and commit Task 2**

  ```bash
  cargo fmt --all -- --check
  cargo test -p raiko2-zkgas-trace
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_sp1_composite_estimator.py -q
  git diff --check
  ```

  Inspect serialization compatibility and the complete diff, then commit:

  ```bash
  git add crates/zkgas-trace experiments/opcode-gas/tests/test_sp1_composite_estimator.py
  git commit -m "feat(zkgas): trace typed context inputs"
  ```

---

### Task 3: Build paired controlled production blocks

**Files:**

- Modify: `bin/guest-launcher/src/controlled_workload.rs`
- Modify: `bin/guest-launcher/src/main.rs`
- Modify: `bin/guest-launcher/tests/controlled_workload.rs`
- Modify: `experiments/opcode-gas/context_production_campaign.py`
- Modify: `experiments/opcode-gas/tests/test_context_production.py`

**Interfaces:**

- Add a structured `ControlledProgram::ContextOpcodeLoop` contract with opcode, lane, count, and a
  typed context profile. Do not overload the old `family/scenario` strings.
- Extend the controlled contract fixture only as needed to select deterministic caller/target
  profiles, calldata, transaction value, and a post-Unzen timestamp delta.
- Use a fixed 256-byte code footprint, fixed `PUSH3` count encoding, fixed transaction gas limit,
  and target/control programs with identical setup, loop, cleanup, and terminal stack height.
- Emit exact context-feature counts separately from higher-layer features and raw-gas ledgers.
- Continue using the production proposal guest; do not create another guest or ELF.

- [ ] **Step 1: Write failing builder-contract tests**

  Cover every frozen context profile and lane, row-ID binding, target/control non-target ledger
  equality, exact target feature delta, count zero, maximum count 64, fixed footprint, fixed gas
  limit, post-Unzen timestamp, positive-value balance accounting, alternate signer/target state,
  final root, and rejection of a free-text or mismatched opcode/profile.

- [ ] **Step 2: Run the focused tests and observe the intended failure**

  ```bash
  cargo test -p guest-launcher --test controlled_workload context
  ```

- [ ] **Step 3: Implement the smallest builder extension**

  Reuse `build_overhead_guest_input_with_topology`, transaction signing, witness reconstruction,
  production tracing, and public-output parity. Parameterize the existing source of truth instead
  of copying it into a second builder.

- [ ] **Step 4: Write and implement Python fixture-generation tests**

  Generate every manifest row deterministically, invoke the Rust builder's identity/trace path,
  and require exact equality between declared and observed raw-gas, context-feature, higher-feature,
  diagnostic, and final-state ledgers before SP1 execution can start.

- [ ] **Step 5: Verify gas-estimator parity on one frozen row**

  Execute the same row once with `standard` and once with `gas-estimator`; require identical gas,
  instruction/syscall counts, public values, input identity, host trace, and exit code. Persist this
  parity row in the run identity, not as a model sample.

- [ ] **Step 6: Verify and commit Task 3**

  ```bash
  cargo fmt --all -- --check
  cargo test -p guest-launcher --test controlled_workload context
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_context_production.py -q
  git diff --check
  ```

  Inspect the complete fixture/identity diff, then commit:

  ```bash
  git add bin/guest-launcher experiments/opcode-gas
  git commit -m "feat(zkgas): add production context fixtures"
  ```

---

### Task 4: Implement production-native residualization and model selection

**Files:**

- Modify: `experiments/opcode-gas/context_production_campaign.py`
- Modify: `experiments/opcode-gas/tests/test_context_production.py`
- Modify: `experiments/opcode-gas/opcode_gas.py`

**Interfaces:**

- Add prepare/resume/run commands with create-only identity, row, decision, and terminal files.
- Add a strict V5 subtotal evaluator for controlled rows. It prices fixed higher-layer terms and
  every known non-target operation, but deliberately leaves the target context key unpriced.
- Add exact count-zero differencing, target fitting, independent control residual checks, fixed
  `CALLDATASIZE` model selection, and untouched final holdout evaluation.
- Persist observed/predicted increments, APE, class/sibling decisions, coefficients, exact design
  matrices, rank, residuals, and rejection reasons.

- [ ] **Step 1: Write failing residualization tests**

  Prove that target coefficients do not change when a synthetic historical control coefficient is
  perturbed, while the control diagnostic changes and can reject the row. Reject missing count-zero
  rows, non-identical non-target ledgers, unpriced non-target work, target work accidentally included
  in `K`, duplicate repeats, and a context-ELF/body-scale field anywhere in the fit input.

- [ ] **Step 2: Write failing model and gate tests**

  Cover exact recovery for all six families, zero/nonzero and range siblings, insufficient signal,
  rank loss, negative/nonfinite coefficients, repeat drift, nonlinear count residual, count-32 and
  count-64 failure, sibling inconsistency, selection order, final-holdout isolation, and APE's exact
  denominator.

- [ ] **Step 3: Run the focused tests and observe the intended failures**

  ```bash
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_context_production.py -q
  ```

- [ ] **Step 4: Implement exact fitting and fail-closed decisions**

  Use explicit matrices and deterministic elimination/least-squares helpers under the 80-digit
  decimal context. Reject rank-deficient systems; do not use NumPy defaults, binary floats, silent
  regularization, coefficient clipping, or post-result feature removal.

- [ ] **Step 5: Implement bounded execution and resume**

  Persist each completed row before starting the next. Resume only when run identity, exact input,
  trace, launcher, production ELF, and existing row hashes match. A subprocess failure preserves
  completed evidence and exits; it never manufactures a terminal decision.

- [ ] **Step 6: Verify and commit Task 4**

  ```bash
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_context_production.py -q
  ~/.venv/bin/python -m py_compile experiments/opcode-gas/context_production_campaign.py
  git diff --check
  ```

  Inspect the full fit/runner diff, then commit:

  ```bash
  git add experiments/opcode-gas
  git commit -m "feat(zkgas): fit production context costs"
  ```

---

### Task 5: Seal and replay the production context result

**Files:**

- Modify: `experiments/opcode-gas/context_production_campaign.py`
- Modify: `experiments/opcode-gas/tests/test_context_production.py`
- Modify: `experiments/opcode-gas/README.md`

**Result inventory:**

- `result.json`
- `campaign-manifest.json`
- `calibration-identity.json`
- `rows.jsonl` or a deterministic compressed equivalent with an explicit size limit
- `campaign-decisions.json`
- `model-report.json`
- `source-coverage-v5.json`
- `source-higher-layer.json`
- `source-discovery.json`
- `source-code-sha256s.json`

- [ ] **Step 1: Write failing seal/replay tests**

  Cover canonical create-only publication, directory-only replay, content identity, exact file
  inventory, canonical compression if used, symlink/FIFO/oversize rejection, path traversal,
  source/result overlap, caller-resealed source forgery, row deletion/reorder/duplication, model or
  gate tampering, dirty implementation tree, wrong revision, and discovery-result promotion.

- [ ] **Step 2: Implement seal and portable verification**

  Recompute every fixture ID, subtotal, matrix, coefficient, prediction, gate, model choice, source
  hash, and result hash without executing SP1. Preserve rejected families and their raw evidence in
  the same package.

- [ ] **Step 3: Verify and commit Task 5**

  ```bash
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_context_production.py -q
  git diff --check
  ```

  Inspect all file/path handling, then commit:

  ```bash
  git add experiments/opcode-gas
  git commit -m "feat(zkgas): seal production context models"
  ```

---

### Task 6: Review the implementation, build the guest, and run the campaign

**Files:**

- Generated then, if verified, add one content-addressed result directory under
  `experiments/opcode-gas/derivations/`; its basename is the first 24 hexadecimal characters of
  the canonical result identity.
- Modify after result only: `docs/plans/2026-09-26-zkgas-calibration-progress.md`

- [ ] **Step 1: Run pre-execution verification**

  ```bash
  cargo fmt --all -- --check
  cargo test -p raiko2-zkgas-trace
  cargo test -p guest-launcher --test controlled_workload context
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_context_production.py -q
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests -q
  ~/.venv/bin/python -m py_compile experiments/opcode-gas/*.py
  git diff --check
  just build-guest sp1
  ```

- [ ] **Step 2: Obtain independent adversarial review and behavioral verification**

  The reviewer inspects the original goal and complete diff, especially target/control separation,
  trace feature semantics, builder parity, final-holdout isolation, and promotion boundaries. The
  tester independently runs malformed-manifest, trace, resume, and seal failure cases plus one real
  standard/gas-estimator parity row. Fix and have the finding owner recheck every confirmed issue.

- [ ] **Step 3: Freeze the clean implementation identity**

  Commit all reviewed source changes. Prepare a fresh run only from that clean revision; bind the
  exact launcher, production ELF/VK, manifest, V5, higher-layer, discovery, trace source, Rust/SP1,
  and version identities.

- [ ] **Step 4: Execute with bounded progress reporting**

  Run the production campaign under a shell-level timeout appropriate to the precomputed row count.
  Persist row-by-row progress and diagnose any repeated failure rather than waiting indefinitely.
  Do not weaken gates, add contexts, or change models after seeing output.

- [ ] **Step 5: Fit, seal, and replay**

  Seal accepted, partial, or terminal-rejected evidence exactly as produced. Verify from the sealed
  directory and from a clean checkout at the bound revision. Commit the result only after both
  independent review and replay pass.

- [ ] **Step 6: Record the milestone**

  Update the progress ledger with exact result ID, source/ELF/revision identities, row counts,
  selected functions, coefficients, gate outcomes, failed families, and explicit statements that
  no final proposal or production configuration was opened or changed.

---

### Task 7: Promote passing families into V6 and a schema-4 composite estimator

**Files:**

- Modify: `experiments/opcode-gas/context_production_campaign.py`
- Modify: `experiments/opcode-gas/composite_estimator.py`
- Modify: `experiments/opcode-gas/tests/test_operation_coverage.py`
- Modify: `experiments/opcode-gas/tests/test_sp1_composite_estimator.py`
- Create after successful promotion: `experiments/opcode-gas/manifests/operation-coverage-v6.json`
- Create after successful promotion one content-addressed estimator directory under
  `experiments/opcode-gas/derivations/`; its basename follows the existing first-24-hex identity
  rule.

**Interfaces:**

- Promote only complete passing keys to `structured_context`; leave every rejected key explicitly
  unsupported with its previous V5 reason plus a reference to the rejected production result.
- Add a separate `context_model` section to composite estimator schema 4. Do not add context models
  to, rescale, or rewrite the historical `OpcodeRegistry`.
- Add `_context_cost()` with exact schema and Decimal validation for the four typed input variants.
- Bind trace schema 4 and the exact sealed production result. The discovery result is provenance of
  the production result, not an independent source of numeric parameters.
- Preserve typed storage, fixed higher-layer costs, coarse state/trie ownership, coverage counters,
  and explicit gaps.

- [ ] **Step 1: Write failing V6 promotion tests**

  Cover six passing keys, partial-family failure, required-sibling failure, wrong source result,
  discovery-only result, changed V5 row, changed unrelated coverage row, source-code drift, and
  canonical V5 immutability.

- [ ] **Step 2: Write failing estimator tests**

  Cover every typed class and boundary, post-Unzen timestamp restriction, malformed model input,
  key/model mismatch, nonnegative cost, exact subtotal, gap/coverage accounting, schema-3 versus
  schema-4 trace joins, and rejection of any fallback multiplier.

- [ ] **Step 3: Implement V6 and schema-4 evaluation**

  Keep the evaluator deterministic and side-effect free. A missing or incompatible typed input is a
  visible operation gap, never a static-raw-gas fallback.

- [ ] **Step 4: Seal and replay the new candidate**

  Build V6 and the review-only estimator only from the independently verified production result.
  Replay exact source hashes and canonical bytes, then perform independent diff review and
  behavioral verification.

- [ ] **Step 5: Verify and commit Task 7**

  ```bash
  ~/.venv/bin/python -m pytest \
    experiments/opcode-gas/tests/test_context_production.py \
    experiments/opcode-gas/tests/test_operation_coverage.py \
    experiments/opcode-gas/tests/test_sp1_composite_estimator.py -q
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests -q
  git diff --check
  ```

  Commit source first, then the separately verified content-addressed artifacts with conventional
  commit messages.

---

### Task 8: Replay only the already-open diagnostics and close the milestone

**Files:**

- Modify: `docs/plans/2026-09-26-zkgas-calibration-progress.md`
- Modify if commands or artifact paths changed: `experiments/opcode-gas/README.md`

- [ ] **Step 1: Recreate schema-4 traces for the two existing ad-hoc fixtures**

  Use the same Mainnet proposal fixtures previously diagnosed. Do not discover replacements, open
  the frozen integration smokes, or open any member of the final 60-proposal corpus.

- [ ] **Step 2: Apply the sealed estimator once**

  Record old/new operation-count coverage, raw-gas coverage, typed-feature coverage, context gaps,
  modeled subtotal, actual `proverGas`, APE, and residual shape. Do not tune any coefficient,
  function, threshold, fixed cost, or ownership rule from these rows.

- [ ] **Step 3: Decide the next family from coverage, not residual fitting**

  Expected next ordinary families are returndata/copy/output, LOG, and EXTCODE/account access.
  Choose the next controlled campaign from explicit remaining gaps. Proposal residuals may
  prioritize a gap but cannot fit it.

- [ ] **Step 4: Final verification and handoff**

  ```bash
  cargo fmt --all -- --check
  cargo test -p raiko2-zkgas-trace
  cargo test -p guest-launcher --test controlled_workload
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests -q
  ~/.venv/bin/python -m py_compile experiments/opcode-gas/*.py
  git diff --check
  git status --short
  ```

  Report exact commands and outcomes, independent review/test status, accepted and rejected context
  keys, diagnostic changes, remaining gaps, and the fact that production/runtime/final-validation
  state remained unchanged.
