# Stateful Opcode Calibration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to
> implement this plan task-by-task. Use superpowers:test-driven-development for each behavioral
> change and superpowers:verification-before-completion before every commit and final handoff.

**Goal:** Measure reproducible SP1 `proverGas` costs for `SLOAD` and `SSTORE` stateful execution,
compare the frozen fixed/access/typed model families, and seal the controlled result without
changing the production zkGas table, proposal trace, or composite estimator.

**Architecture:** Extend the existing `OpcodeLabInput` with a typed, full-width storage contract.
Create a small `no_std`-compatible `raiko2-opcode-lab` crate that is the only constructor for the
Osaka REVM database and transaction used by both the measured SP1 guest and the host-native trace.
Keep the measured guest uninstrumented. Generate a canonical target/control matrix and validate it
with a separate host semantic pass. Orchestrate, fit, verify, and seal the campaign in a focused
Python module that reuses the existing runner, identity, exact-arithmetic, and create-only sealing
helpers from `opcode_gas.py`.

**Tech Stack:** Rust, REVM `41.0.0`, SP1 `6.3.0`, Serde/bincode, Python 3 from `~/.venv`, exact
`Decimal`/`Fraction` arithmetic, canonical JSON/JSONL, SHA256 content addressing, `unittest`/pytest,
Cargo, and `just build-guest sp1`.

**Spec:** `docs/plans/2026-09-28-zkgas-stateful-opcode-calibration-design.md`

## Global Constraints

- The measured quantity is `stateful_revm_execution_cost`. It includes REVM storage execution and
  journal/result-state work, but excludes production witness materialization, persistent committed
  dirty-state processing, trie hashing, and final root construction.
- Both guest and host use `SpecId::OSAKA`, the same `InMemoryDB` constructor, the same transaction
  envelope, and the same access-list constructor. There is one implementation source of truth.
- Preserve backward compatibility for historical stateless `OpcodeLabInput` fixtures. Do not
  rewrite, reclassify, or re-sample historical sealed artifacts.
- Use canonical 32-byte slot and value encodings. Every fixture bytecode uses `PUSH32` for storage
  slots and store values, including `0`, `1`, and `2`.
- The measured guest may validate input and execute REVM. It must not add an inspector, observer,
  post-state walk, or per-operation semantic report.
- The host-native trace and semantic check are mandatory admission gates. Their output must never
  be substituted for the formal guest's `proverGas`.
- Counts are frozen at fit `[1, 2, 4, 8, 16]`, holdout `32`, checkpoint `64`, plus structural zero;
  every row has exactly three repeats. There is no result-driven count expansion.
- Every reference opcode in a target/control difference resolves against the exact sealed Osaka
  core at `experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/` with core SHA256
  `b66d7951bfa416810f93319f99c30ce91969026ee9fa3a402a74cbc2214f7e8b`.
- Fit and decision arithmetic uses `Decimal` or exact `Fraction`, never binary `float`. Persist exact
  numerators/denominators and an 80-digit half-even decimal projection.
- This plan does not produce a 105-opcode production registry, edit operation coverage, add typed
  proposal events, replay proposals, change Unzen multipliers, or alter runtime/deployment config.
- Generated run directories remain ignored. Only a passing, independently verified, create-only,
  content-addressed result directory may be committed.
- Use repository-relative paths in persisted artifacts. No machine-specific absolute path may enter
  a manifest, row, report, seal, README example, or test fixture.

## Frozen Campaign Contract

The campaign manifest declares every scenario and exact expected semantic identity. It includes:

- SLOAD: warm/cold times zero/nonzero;
- clean SSTORE: zero/nonzero no-op, set, clear, and nonzero reset, each warm/cold;
- dirty SSTORE: rewrite and restore-zero/nonzero with an identical target/control prefix;
- high-limb value and high-slot diagnostics from the approved design;
- fit/holdout/checkpoint/structural-zero counts and three repeats;
- the current sealed registry path and hash;
- exact guest, launcher, manifest, trace, input, and implementation identities.

Each target/control relation persists a signed execution ledger:

```text
L_s[k] = target_count_per_active_slot[k] - control_count_per_active_slot[k]
L_s[state_opcode] = 1

r_s = sum(L_s[k] * sealed_absolute_cost[k] for k != state_opcode)
absolute_stateful_cost_s = fitted_relative_slope_s - r_s
```

Reference entries bind the exact typed registry input and predicted cost, not only a key name or raw
gas. Missing or non-exact reference models reject the row. The trace also verifies that the signed
total operation-event count is zero where the common dispatch term is expected to cancel.

Model families and gates are exactly those in the approved design. In particular, all candidate
families are fitted on the fit rows before holdout/checkpoint/high-limb decisions are opened, and
the fixed selection order is `M_fixed`, `M_access`, `M_typed`. The raw-gas relation is diagnostic.

---

### Task 1: Add the typed storage input contract and canonical fixture generator

**Files:**

- Modify: `crates/primitives/src/opcode_lab.rs`
- Modify: `crates/primitives/tests/opcode_lab_input.rs`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Create: `experiments/opcode-gas/stateful_opcode_campaign.py`
- Create: `experiments/opcode-gas/tests/test_stateful_opcode.py`

**Interfaces:**

- Add serializable `OpcodeLabStorageInput`, `OpcodeLabStorageAccess`, and
  `OpcodeLabStorageOperation` types and `OpcodeLabInput.storage: Option<...>`.
- `OpcodeLabStorageInput` carries `measurement_opcode: 0x54 | 0x55` and
  `lane: target | control`. The existing top-level `OpcodeLabInput.opcode` is the concrete lane's
  declared opcode: measurement opcode for target, reference opcode for control. The structured
  operation remains the intended state scenario in both lanes so their DB and access list match.
- Represent slot/original/current/new/expected values as canonical `[u8; 32]` wire values with
  `0x`-prefixed 64-hex-digit JSON. Reject shorter, longer, negative, or non-hex values.
- Add `validate_storage_contract()` and call it from `validate_controlled_contract()`.
- Add a Python `StatefulCampaignManifest` loader and deterministic case/program generator. Keep
  Python domain logic in `stateful_opcode_campaign.py`; expose only thin lazy CLI dispatch from
  `opcode_gas.py` when needed.

- [ ] **Step 1: Write failing Rust contract tests**

  Cover stateless JSON compatibility, full-width JSON/bincode round trips, invalid lane,
  target measurement-opcode mismatch, control lane containing a storage opcode, missing SSTORE
  current/new values, SLOAD with store values, expected-load mismatch, noncanonical widths, and
  dirty-prefix relationships. The lane-aware validator must permit a control top-level reference
  opcode while still binding the pair to one measurement opcode and semantic scenario.

- [ ] **Step 2: Run the Rust tests and observe the intended failure**

  ```bash
  cargo test -p raiko2-primitives opcode_lab
  ```

  The new tests must fail because the storage types and validation do not exist, not because a
  fixture or dependency is missing.

- [ ] **Step 3: Implement the smallest typed Rust contract**

  Keep parsing and validation in `crates/primitives/src/opcode_lab.rs`. Do not make primitives
  depend on REVM. Preserve `OpcodeLabInput::default()` and existing stateless serialization.

- [ ] **Step 4: Write failing Python manifest and generator tests**

  Assert the exact frozen scenario inventory, exact counts/repeats, `PUSH32` operands, dirty-prefix
  identity, deterministic case IDs, and rejection of any undeclared scenario or malformed U256.
  Persist ordered operand-immediate spans and a program-shape hash after zeroing those spans.
  Compare low/high variants byte-for-byte after masking only those exact spans; reject a changed
  PUSH opcode, operand position, suffix, reference instruction, or any unrelated byte.

- [ ] **Step 5: Implement canonical generation**

  Reuse canonical JSON/hash/path helpers and fixed-microprogram framing from `opcode_gas.py`.
  Generate target and control lanes from structured case data; never infer semantics from the free
  text scenario name. Emit the storage object into both the case record and actual guest input.

- [ ] **Step 6: Verify and commit Task 1**

  ```bash
  cargo test -p raiko2-primitives opcode_lab
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_stateful_opcode.py -q
  git diff --check
  ```

  Inspect the full Task 1 diff, then commit:

  ```bash
  git add crates/primitives experiments/opcode-gas
  git commit -m "feat(zkgas): define stateful opcode fixtures"
  ```

---

### Task 2: Share the Osaka REVM database and transaction constructor

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `crates/opcode-lab/Cargo.toml`
- Create: `crates/opcode-lab/src/lib.rs`
- Modify: `bin/guest-launcher/Cargo.toml`
- Modify: `guests/sp1/Cargo.toml`
- Modify: `guests/sp1/Cargo.lock`
- Modify: `guests/sp1/src/revm_opcode_lab_impl.rs`
- Modify: `guests/sp1/src/revm_opcode_lab.rs`

**Interfaces:**

- Add workspace crate `raiko2-opcode-lab`, depending only on `raiko2-primitives` and
  `revm = 41.0.0` with default features disabled.
- Export `OPCODE_LAB_SPEC_ID: SpecId = SpecId::OSAKA`.
- Export `build_benchmark_db(Bytecode, Option<&OpcodeLabStorageInput>) -> InMemoryDB`.
- Export `build_benchmark_tx(u64, Option<&OpcodeLabStorageInput>) -> Result<TxEnv,
  TxEnvBuildError>`.
- Use REVM benchmark caller/target constants, insert declared storage into `BENCH_TARGET`, and add
  exactly `BENCH_TARGET + slot` to the transaction access list only for warm cases.

- [ ] **Step 1: Write failing shared-constructor tests**

  Prove Osaka identity, zero/nonzero storage insertion, empty cold access list, exact warm access
  list, unchanged stateless benchmark behavior, and no dependence on host-only `std` APIs.

- [ ] **Step 2: Run the tests and observe the intended failure**

  ```bash
  cargo test -p raiko2-opcode-lab
  ```

- [ ] **Step 3: Implement the shared crate**

  Build only the concrete DB and `TxEnv`; do not expose a duplicated host/guest execution wrapper
  or a generic REVM `Context` type. Return construction errors; the DB insert may only use an
  infallible backing database.

- [ ] **Step 4: Replace the guest-local constructor**

  Keep `sp1-revm-opcode-lab` as the existing binary. Pass `input.storage.as_ref()` into the shared
  builder for every fixed microprogram. Preserve the accumulator/output commitment and the Osaka
  opcode canary.

- [ ] **Step 5: Add guest-side behavioral tests**

  Cover SLOAD zero/nonzero and SSTORE `0 -> 1` / `1 -> 0`, plus the existing Osaka CLZ canary. The
  tests may inspect success and returned in-memory state on the host target; the formal guest path
  must not perform that inspection.

- [ ] **Step 6: Verify and commit Task 2**

  ```bash
  cargo test -p raiko2-opcode-lab
  cargo test --manifest-path guests/sp1/Cargo.toml revm_opcode_lab
  cargo fmt --all --check
  cargo clippy -p raiko2-opcode-lab -- -D warnings
  git diff --check
  ```

  Commit:

  ```bash
  git add Cargo.toml Cargo.lock crates/opcode-lab bin/guest-launcher/Cargo.toml \
    guests/sp1/Cargo.toml guests/sp1/Cargo.lock guests/sp1/src/revm_opcode_lab.rs \
    guests/sp1/src/revm_opcode_lab_impl.rs
  git commit -m "feat(zkgas): share Osaka opcode lab state"
  ```

---

### Task 3: Add exact host trace and semantic admission checks

**Files:**

- Modify: `bin/guest-launcher/src/controlled_workload.rs`
- Modify: `bin/guest-launcher/src/main.rs`
- Modify: `bin/guest-launcher/tests/controlled_workload.rs`
- Modify: `experiments/opcode-gas/stateful_opcode_campaign.py`
- Modify: `experiments/opcode-gas/tests/test_stateful_opcode.py`

**Interfaces:**

- Make `controlled_opcode_workload_spec()` record Osaka and the canonical storage contract.
- Make `trace_revm_opcode_workload()` use `raiko2-opcode-lab` for DB and `TxEnv` construction.
- Extend `ControlledOpcodeTrace` with executed per-opcode counts/raw gas, structured storage
  identity, access class, expected/observed load or transition, signed target/control execution
  ledger inputs, result status, and exact backend-input identity.
- Add a separate host-native semantic checker that executes the same canonical input and inspects
  load output/REVM result state. It is never called by the measured guest.

- [ ] **Step 1: Write failing host trace tests**

  Cover warm/cold SLOAD raw-gas distinction, every clean SSTORE branch, dirty prefix/second-op
  transition, high U256 value and slot, exact target count, Osaka identity, workload-state
  provenance, and malformed declared transition rejection.

- [ ] **Step 2: Run the tests and observe the intended failure**

  ```bash
  cargo test -p guest-launcher controlled_workload
  ```

- [ ] **Step 3: Implement Osaka tracing and semantic validation**

  Reuse the shared constructors. The inspector records what executed; the semantic checker verifies
  the declared result. Do not mutate or commit post-state in the measured path. Preserve existing
  stateless trace output compatibility.

- [ ] **Step 4: Add target/control pair admission tests**

  Reject differing transaction envelope, gas limit, bytecode length, access list, prestate,
  dirty-prefix program, inactive-slot layout, reference ledger, or REVM identity.
  Permit only the declared active target/control opcode substitution. Validate the structured
  measurement opcode/lane against the concrete lane opcode and prove the control trace executes no
  `SLOAD` or `SSTORE`. Each lane's trace/report backend-input hash must equal that lane's own
  canonical guest-input hash. The pair/workload identity binds the ordered
  `(target_hash, control_hash)` plus scenario, measurement opcode, count, and repeat; tests reject
  target/control swaps, cross-pair joins, and a report attached to the other lane.

- [ ] **Step 5: Bind trace admission in the Python campaign**

  A generated row becomes runnable only after exact host trace and semantic checks pass. Persist
  their canonical hashes and identities, not machine paths. Reject zero, duplicate, or mismatched
  trace matches.

- [ ] **Step 6: Verify and commit Task 3**

  ```bash
  cargo test -p guest-launcher controlled_workload
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_stateful_opcode.py -q
  cargo fmt --all --check
  cargo clippy -p raiko2-opcode-lab -p guest-launcher -- -D warnings
  git diff --check
  ```

  Commit:

  ```bash
  git add bin/guest-launcher experiments/opcode-gas
  git commit -m "feat(zkgas): validate stateful opcode traces"
  ```

---

### Task 4: Implement the bounded campaign runner and immutable row ledger

**Files:**

- Create: `experiments/opcode-gas/manifests/sp1-stateful-opcode-v1.json`
- Modify: `experiments/opcode-gas/stateful_opcode_campaign.py`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_stateful_opcode.py`
- Modify: `experiments/opcode-gas/tests/test_runner.py`
- Modify: `experiments/opcode-gas/tests/test_run_manifest.py`
- Modify: `experiments/opcode-gas/tests/test_sp1_guest_manifest.py`

**Interfaces:**

- Add thin CLI commands `run-stateful-opcode-campaign` and
  `verify-stateful-opcode-campaign`; core implementation remains in the focused module.
- Reuse existing calibration identity freeze, batched `gas-estimator` execution, canonical report
  normalization, repeat pairing, resume/decision ledger, path safety, and content hashing.
- Persist one immutable row identity for `(scenario, lane, count, repeat, input hash, ELF hash,
  launcher hash, trace hash)`. Resume may reuse only an exact identity match.

- [ ] **Step 1: Write failing manifest and CLI tests**

  Assert exact scenario/count/repeat inventory, exact source registry path/hash, exact stage
  `revm-opcode-lab`, `execute + gas-estimator`, Osaka identity, and refusal to accept extra cases,
  relative paths escaping the repo, symlinks, hash drift, or another guest artifact.

- [ ] **Step 2: Write failing runner/resume tests**

  Cover deterministic generation, target/control pairing by repeat, exact three-repeat requirement,
  partial resume, duplicate-row rejection, stale report rejection, failed guest execution, missing
  trace/semantic evidence, and create-only terminal campaign output.

- [ ] **Step 3: Implement the manifest and runner**

  Keep the existing runner as the execution source of truth. Do not copy its subprocess,
  cancellation, retry, report parser, or state-machine logic into the new module. The wrapper may
  supply stateful cases and validate state-specific evidence before/after each reused primitive.

- [ ] **Step 4: Implement portable campaign verification**

  `verify-stateful-opcode-campaign` must replay row inventory, canonical hashes, repeat pairing,
  target/control identities, trace admission, and state-machine decisions using only the run
  directory plus explicitly supplied repository artifacts. It must not execute the guest.

- [ ] **Step 5: Verify and commit Task 4**

  ```bash
  ~/.venv/bin/python -m pytest \
    experiments/opcode-gas/tests/test_stateful_opcode.py \
    experiments/opcode-gas/tests/test_runner.py \
    experiments/opcode-gas/tests/test_run_manifest.py \
    experiments/opcode-gas/tests/test_sp1_guest_manifest.py -q
  git diff --check
  ```

  Commit:

  ```bash
  git add experiments/opcode-gas
  git commit -m "feat(zkgas): run stateful opcode campaign"
  ```

---

### Task 5: Fit absolute stateful costs and select the frozen model family

**Files:**

- Modify: `experiments/opcode-gas/stateful_opcode_campaign.py`
- Modify: `experiments/opcode-gas/tests/test_stateful_opcode.py`

**Interfaces:**

- Add exact signed execution-count ledgers for each target/control pair.
- Resolve every reference entry through its exact typed model in the sealed 103-opcode registry.
- Fit each scenario with a free nuisance intercept on fit rows only; reconstruct absolute stateful
  cost by subtracting the sealed signed reference cost.
- Joint-fit `M_fixed`, `M_access`, `M_typed`, and diagnostic raw-gas models before opening holdout,
  checkpoint, or high-limb decisions.
- Seal exact coefficients and all derived Decimal projections, gate inputs, decisions, and rejection
  reasons. Set `candidate_eligible = false` for the existing composite estimator.

- [ ] **Step 1: Write failing exact-ledger and reconstruction tests**

  Use synthetic rational costs to prove signed positive/negative references, typed reference model
  evaluation, common-dispatch cancellation, `absolute = slope - reference`, exact Fraction
  persistence, 80-digit projection, and failure on missing/unsupported/non-exact references.

- [ ] **Step 2: Write failing per-scenario gate tests**

  Cover `R2`, relative slope standard error, residual/signal, structural-zero/intercept,
  repeat-spread, holdout APE, checkpoint APE, positive-signal/noise floor, and every zero-denominator
  fail-closed path using the formulas in the approved design.

- [ ] **Step 3: Write failing model-comparison tests**

  Prove joint nuisance intercepts, shared warm/access/branch parameters, fixed selection order,
  required-scenario completeness, low/high consistency, diagnostic-only raw-gas output, no
  post-holdout refit, and `no_eligible_model` when all production families fail.

- [ ] **Step 4: Implement exact fitting and decisions**

  Convert canonical Decimal strings to `Fraction` before solving. Persist numerator/denominator,
  project with an 80-digit half-even context, and require replay residual at most `1e-75`. Keep raw
  observations separate from derived reports so portable verification can recompute every result.

- [ ] **Step 5: Verify and commit Task 5**

  ```bash
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_stateful_opcode.py -q
  git diff --check
  ```

  Commit:

  ```bash
  git add experiments/opcode-gas/stateful_opcode_campaign.py \
    experiments/opcode-gas/tests/test_stateful_opcode.py
  git commit -m "feat(zkgas): fit stateful opcode models"
  ```

---

### Task 6: Add create-only result sealing and operator documentation

**Files:**

- Modify: `experiments/opcode-gas/stateful_opcode_campaign.py`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_stateful_opcode.py`
- Modify: `experiments/opcode-gas/README.md`

**Interfaces:**

- Add `seal-stateful-opcode-result` and `verify-stateful-opcode-result`.
- Seal a content-addressed directory containing the immutable manifest, normalized observations,
  trace/semantic ledger, scenario fits, model reports, selection/rejection decision, ownership
  statement, and provenance.
- Verification recomputes every hash, fit, gate, and decision from the sealed directory. It rejects
  extra/missing files, symlinks, path escape, tampering, dirty or mismatched source identity, and a
  pre-existing non-identical destination.

- [ ] **Step 1: Write failing seal/replay tests**

  Cover deterministic directory name, exact inventory, idempotent identical seal, conflicting
  destination, file/symlink/tamper rejection, provenance drift, registry drift, row drift, model
  decision drift, and ownership/candidate-boundary fields.

- [ ] **Step 2: Implement create-only seal and replay**

  Reuse canonical JSON and path-guard primitives from the Osaka augmentation workflow. Do not copy
  or mutate the historical registry. The sealed result is experimental evidence, not an augmented
  production core.

- [ ] **Step 3: Document exact operator commands and boundaries**

  Document preparation, guest build, run/resume, run verification, result sealing, result replay,
  generated-vs-tracked paths, expected runtimes, and the explicit prohibition on proposal replay or
  production promotion under this plan.

- [ ] **Step 4: Verify and commit Task 6**

  ```bash
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_stateful_opcode.py -q
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests -q
  git diff --check
  ```

  Commit:

  ```bash
  git add experiments/opcode-gas
  git commit -m "docs(zkgas): document stateful opcode campaign"
  ```

---

### Task 7: Build the measured guest and run the formal controlled campaign

**Files:**

- Generated, ignored: stateful campaign run directory under `target/`
- Modify and commit before identity freeze: `crates/guests/elf/sp1_revm_opcode_lab.elf`
- Modify and commit before identity freeze: `crates/guests/elf/sp1_revm_opcode_lab.vk.bin`
- Modify and commit before identity freeze: `crates/guests/elf/sp1.provenance.json`
- Create on success: one content-addressed result directory under
  `experiments/opcode-gas/derivations/`
- Modify after a verified result: `docs/plans/2026-09-26-zkgas-calibration-progress.md`
- Modify after a verified result: `experiments/opcode-gas/README.md`

- [ ] **Step 1: Run the full preflight verification**

  ```bash
  cargo fmt --all --check
  cargo clippy -p raiko2-opcode-lab -p guest-launcher -- -D warnings
  cargo test -p raiko2-primitives opcode_lab
  cargo test -p raiko2-opcode-lab
  cargo test --manifest-path guests/sp1/Cargo.toml revm_opcode_lab
  cargo test -p guest-launcher controlled_workload
  ~/.venv/bin/python -m pytest experiments/opcode-gas/tests -q
  git diff --check
  ```

- [ ] **Step 2: Build the formal SP1 guest**

  ```bash
  just build-guest sp1
  ```

  Record the exact `sp1-revm-opcode-lab` ELF/VK hashes and provenance. Do not hand-edit generated
  ELF files. Start from a clean tree, inspect the build diff, and require that only deterministic
  SP1 guest artifacts/provenance expected from this source change moved. Commit those tracked
  artifacts before preparing calibration identity:

  ```bash
  git add crates/guests/elf
  git commit -m "build(guest): refresh stateful opcode lab artifacts"
  just build-guest sp1 --check
  git status --short
  ```

  The provenance check must pass and the final status command must be empty. If the build changes
  any other SP1 artifact, inspect and explain it before committing; never silently widen the
  expected artifact set. Run the existing calibration identity freeze only after this commit so
  its clean-tree gate binds the committed ELF/VK/provenance rather than a dirty build workspace.

- [ ] **Step 3: Generate and trace every frozen fixture**

  Run the documented prepare/generate command. Verify exact scenario inventory, target/control
  pairing, structural zero, counts, repeats, high-limb rows, signed reference ledgers, semantic
  checks, and all identity hashes before guest execution.

- [ ] **Step 4: Run and resume the formal campaign**

  Use `--mode execute --sp1-execution-engine gas-estimator`. Resume only exact persisted row
  identities. A missing, unstable, or failed row remains explicit; do not add counts, change
  fixtures, or relax a gate after observing data.

- [ ] **Step 5: Verify and seal the result**

  Run the portable campaign verifier, seal the create-only result, then run the directory-only
  result verifier. Independently recompute representative target/control deltas, reference-cost
  subtraction, model parameters, holdout/checkpoint/high-limb APE, and the final selection.

- [ ] **Step 6: Update progress and commit the evidence**

  Record exact revisions and hashes, observed costs, every gate, the selected model or terminal
  rejection, cost ownership, limitations, and `candidate_eligible = false`. Do not describe a
  passing controlled model as deployed or proposal-validated.

  ```bash
  git add experiments/opcode-gas/derivations experiments/opcode-gas/README.md \
    docs/plans/2026-09-26-zkgas-calibration-progress.md
  git commit -m "data(zkgas): seal stateful opcode calibration"
  ```

## Final Review And Handoff

- An independent reviewer inspects the original request, approved design, every implementation
  commit, complete branch diff, measured-guest instrumentation boundary, host/guest constructor
  identity, fitting math, seal replay, and final result.
- Any material finding is fixed and re-reviewed by the agent that raised it. Use at most two normal
  review/fix iterations before escalating a design disagreement.
- Before pushing the final checkpoint, rerun the Task 7 preflight plus result replay, inspect the
  complete diff against the branch base, and verify no production table, operation coverage,
  proposal trace, runtime config, or deployment file changed.
- Push only after the branch is coherent and all required checks have current evidence.
