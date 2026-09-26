# ZKGas Osaka Operation Supplement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve the sealed 101-opcode baseline, move the controlled REVM lab to Osaka, measure
only the missing ISZERO and CLZ relations, and seal a provenance-complete 103-opcode augmented core
artifact without opening higher-layer or proposal work.

**Architecture:** The historical core and dynamic artifacts remain immutable. A small non-fitting
compatibility canary checks whether representative Prague-lab relations transport to the Osaka lab;
then the normal adaptive relation engine samples only ISZERO and CLZ. A create-only augmentation
artifact binds the baseline, canary, supplement, and resulting two added models.

**Tech Stack:** Python 3.11 standard library via `~/.venv`, Rust/revm, the existing SP1
gas-estimator execution path, exact `Fraction`/80-digit `Decimal` artifact math, JSON/JSONL, and Git.

**Spec:** `docs/plans/2026-09-26-zkgas-calibration-design.md`

**Progress:** `docs/plans/2026-09-26-zkgas-calibration-progress.md`

## Status

In progress. Task 1's Osaka/CLZ/SWAP1 portion is implemented in the working tree but remains
uncommitted. Structured version identity, independent root verification, guest artifact rebuild,
and adversarial review remain open.

## Global Constraints

- Use Taiko Unzen schedule identity with REVM `SpecId::OSAKA` execution semantics.
- Do not resample all 101 historical opcode coefficients.
- Do not mutate the sealed derivation `3e1d97c461cd2ef9a40e6a02` or source run
  `09ebb08d76d3f461086b0cf4`.
- The compatibility canary is pass/fail reuse evidence only; it cannot refit or rescale the baseline.
- Formal supplemental sampling is limited to `opcode:0x15:canonical` and
  `opcode:0x1e:canonical`.
- Use the existing adaptive relation fitter, checkpoint rules, three-repeat rule, and canonical
  precision; do not add a second fitting implementation.
- The augmented artifact remains `candidate_eligible = false`.
- Do not run proposal, state/trie, transaction, block, RISC0, Boundless, or production-promotion
  work under this plan.
- Generated ELF/VK/provenance files are regenerated only by `just build-guest sp1`, never hand-edited.
- Use repository-relative paths in every tracked artifact and document.

## File Map

- `guests/sp1/src/revm_opcode_lab_impl.rs`: selects the REVM execution spec for the opcode lab.
- `guests/sp1/src/lib.rs`: proves the real guest library executes Osaka CLZ semantics.
- `experiments/opcode-gas/manifests/sp1-calibration-v1.toml`: materialized CLZ case and measurement
  key.
- `experiments/opcode-gas/opcode_gas.py`: matched controls, inventory, subset runner, canary,
  augmentation builder, and CLI wiring.
- `crates/primitives/src/chain_spec.rs`: single runtime source for the Taiko-fork-to-REVM-spec
  mapping consumed by the exporter.
- `xtask/src/export_unzen_zk_gas_schedule.rs`: structured Unzen/Fusaka/Osaka schedule identity.
- `experiments/opcode-gas/tests/test_fixture_emit.py`: exact ISZERO/CLZ target-control programs and
  signed raw-gas maps.
- `experiments/opcode-gas/tests/test_manifest.py`: materialized manifest identity and ordering.
- `experiments/opcode-gas/tests/test_inventory.py`: 103-key core inventory.
- `experiments/opcode-gas/tests/test_calibration_model.py`: rank-99 relation basis.
- `experiments/opcode-gas/tests/test_hierarchical_model.py`: exact 103-key typed-registry replay.
- `experiments/opcode-gas/tests/test_sp1_candidate_report.py`: candidate-side inventory invariants.
- `experiments/opcode-gas/tests/test_osaka_supplement.py`: canary, subset execution, augmentation,
  provenance, and mutation coverage added by Task 2.
- `experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml`: exact frozen
  historical manifest for schema-aware baseline validation.
- `experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.sha256`: tracked
  checksum file that makes the historical fixture independently verifiable from a clean checkout.
- `experiments/opcode-gas/README.md`: operator commands and baseline-plus-supplement evidence rules.
- `crates/guests/elf/`: generated SP1 guest artifacts from the reviewed source.
- `experiments/opcode-gas/derivations/<augmentation-id>/`: create-only tracked augmentation evidence.
- `docs/plans/2026-09-26-zkgas-calibration-progress.md`: milestone status after each completed gate.

---

### Task 1: Land Osaka Semantics And Exact ISZERO/CLZ Controls

**Files:**

- Modify: `guests/sp1/src/revm_opcode_lab_impl.rs`
- Modify: `guests/sp1/src/lib.rs`
- Modify: `experiments/opcode-gas/manifests/sp1-calibration-v1.toml`
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_fixture_emit.py`
- Modify: `experiments/opcode-gas/tests/test_manifest.py`
- Modify: `experiments/opcode-gas/tests/test_inventory.py`
- Modify: `experiments/opcode-gas/tests/test_calibration_model.py`
- Modify: `experiments/opcode-gas/tests/test_hierarchical_model.py`
- Modify: `experiments/opcode-gas/tests/test_sp1_candidate_report.py`
- Modify: `experiments/opcode-gas/tests/test_run_manifest.py`
- Modify: `experiments/opcode-gas/README.md`
- Modify: `crates/primitives/src/chain_spec.rs`
- Modify: `xtask/src/export_unzen_zk_gas_schedule.rs`
- Regenerate: SP1 ELF/VK/provenance files under `crates/guests/elf/`

**Interfaces:**

- Consumes: existing `CaseSpec`, `matched_control_spec`, `generate_relation_cases`,
  `fit_formal_relation_round`, and sealed 101-opcode artifact schema.
- Produces: Osaka opcode guest; materialized `opcode:0x1e`; 103-key ordered core inventory; exact
  rank-99 relation contract; formal ISZERO/CLZ relations controlled by one `SWAP1` each; structured
  Unzen/Fusaka/Osaka calibration identity.

- [ ] **Step 1: Keep the failing fixture and rank tests**

The tests must assert these exact signed raw-gas maps:

```python
expected = {
    0x15: {"opcode:0x15": 3, "opcode:0x90": -3},
    0x1E: {"opcode:0x1e": 5, "opcode:0x90": -3},
}
```

They must also assert one target/control opcode per repetition, two input stack items, final stack
height two, no `NOT` coefficient, ordered core length 103, and exact matrix rank 99 with the four
natural anchors unchanged.

- [ ] **Step 2: Keep the real Osaka CLZ activation regression**

The library test executes bytecode `PUSH1 1; CLZ; STOP` and asserts that the accumulator's encoded
transaction-success bit is `1`. This proves REVM recognizes and successfully executes CLZ under
Osaka; it does not claim that `CLZ(1) == 1`. The same bytecode must not report success under Prague,
and a parser-only test is not sufficient. Name the test
`revm_opcode_lab_accepts_osaka_clz_opcode` to reflect that contract.

- [ ] **Step 3: Implement the smallest production change**

Set the opcode lab to `SpecId::OSAKA`, add `PURE_OPCODE_DEFAULTS[0x1E]`, materialize the CLZ case and
measurement key, and special-case only ISZERO/CLZ to use `SWAP1`. Leave every other historical
control unchanged.

- [ ] **Step 4: Add structured version identity to the exported source of truth**

Make `TaikoFork` expose the single shared fork-to-REVM-spec mapping used by `ForkId::as_spec_id`, so
both runtime chain selection and the exporter obtain `TaikoFork::Unzen -> SpecId::OSAKA` from
`crates/primitives/src/chain_spec.rs`. Do not copy `"OSAKA"` into an exporter-local mapping.

Extend the Rust Unzen schedule export with:

```json
{
  "taiko_fork": "Unzen",
  "production_schedule": "UNZEN_ZK_GAS_SCHEDULE",
  "ethereum_upgrade": "Fusaka",
  "revm_spec_id": "OSAKA"
}
```

The exporter binds `taiko_fork` and `production_schedule` to its concrete
`UNZEN_ZK_GAS_SCHEDULE`, derives `revm_spec_id` through the shared `TaikoFork` mapping, and emits
`ethereum_upgrade = "Fusaka"` as an explicitly declared vocabulary label rather than a
schedule-derived value. Add a Rust regression asserting that the exported Unzen identity and the
runtime chain-spec mapping both resolve to `SpecId::OSAKA`.

`prepare-calibration` copies those fields and adds `proving_backend = "sp1"` and
`primary_metric = "proverGas"` under one `version_identity` object in `calibration_identity`.
Include that object in the calibration ID. Tests mutate each field independently and require a
different calibration ID plus mismatch rejection during run, supplement, and seal validation. Do
not reconstruct these labels independently in the supplement command.

- [ ] **Step 5: Run the focused and complete source tests**

Run:

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
cargo test --manifest-path guests/sp1/Cargo.toml --lib
cargo test -p raiko2-primitives chain_spec
cargo test -p xtask export_unzen_zk_gas_schedule
cargo fmt --all -- --check
rustfmt --edition 2021 --check \
  guests/sp1/src/lib.rs guests/sp1/src/revm_opcode_lab_impl.rs
git diff --check
```

Expected: all Python and Rust tests pass; the touched Rust files and the complete diff are clean.
Any unrelated pre-existing formatting failure must be recorded separately rather than reported as a
pass.

- [ ] **Step 6: Build and verify the reviewed SP1 artifacts**

Run:

```bash
just build-guest sp1
cargo test --manifest-path guests/sp1/Cargo.toml --lib
git diff --check
```

The only SP1 binary artifacts allowed to change are `sp1_revm_opcode_lab.elf` and
`sp1_revm_opcode_lab.vk.bin`; `sp1.provenance.json` must change only to record their new identities.
Require byte-identical SHA256 values for `sp1_opcode_lab`, `sp1_precompile_lab`,
`sp1_shasta_proposal`, and `sp1_shasta_aggregation` ELF/VK pairs. Do not hand-edit any generated
file.

- [ ] **Step 7: Independently review the complete source and artifact diff**

The reviewer must verify Osaka execution, CLZ manifest identity, exact SWAP1 control shape, absence
of NOT in both new equations, 103-key ordering/rank, historical wording, and the explicit absence of
sampling or full-resample behavior. The review also verifies the structured version identity and
that every generated artifact matches the reviewed guest source.

- [ ] **Step 8: Commit the verified change**

```bash
git add guests/sp1/src/lib.rs guests/sp1/src/revm_opcode_lab_impl.rs \
  experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  experiments/opcode-gas/opcode_gas.py experiments/opcode-gas/tests \
  experiments/opcode-gas/README.md crates/primitives/src/chain_spec.rs \
  xtask/src/export_unzen_zk_gas_schedule.rs \
  crates/guests/elf
git commit -m "feat(zkgas): add Osaka opcode supplement inputs"
```

---

### Task 2: Add A Bounded Canary And Supplemental Artifact Path

**Files:**

- Modify: `experiments/opcode-gas/opcode_gas.py`
- Create: `experiments/opcode-gas/tests/test_osaka_supplement.py`
- Create: `experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml`
- Create: `experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.sha256`
- Modify: `experiments/opcode-gas/README.md`

**Interfaces:**

- Consumes: sealed baseline derivation, historical relation artifact and decision ledger, current
  calibration provenance, `generate_relation_cases`, `cmd_run`, and `fit_formal_relation_round`.
- Produces: schema-aware `validate_historical_core_opcode_baseline`,
  `run-osaka-opcode-supplement`, `verify-osaka-opcode-supplement`,
  `seal-osaka-opcode-augmentation`, and `verify-osaka-opcode-augmentation` CLIs,
  `compatibility-canary.json`, `opcode-supplement.json`, `augmentation.json`, and an augmented
  schema-versioned core artifact.

- [ ] **Step 1: Write a real historical-schema regression**

Track the exact frozen `09ebb08d76d3f461086b0cf4/controlled-manifest.toml` bytes as the historical
fixture and assert SHA256
`4140fe1a0ccc533dbee8940a63da6db41be8a26955cc0022ccae5e1aedc3e01e`. Load the tracked
`3e1d97c461cd2ef9a40e6a02` derivation, dynamic artifact, and core artifact with that fixture after
the current inventory has become 103 keys.

Track a canonical `controlled-manifest.sha256` beside the fixture. The Task 3 operator commands and
every run, seal, and replay entry point must execute the equivalent checksum verification before
accepting the fixture; a missing or mismatched checksum is a hard failure. Its checksum record uses
the manifest path relative to the repository root so the documented root-level `sha256sum --check`
command resolves the same tracked file from a clean checkout.

The new validator must verify the historical derivation/content hashes, schema versions, ordered
102-key manifest inventory, 101 modeled registry entries, ISZERO-only missing core key, dynamic
source identity, and 49 unsupported named opcodes. It must not call current `_core_opcode_keys`, add
CLZ, or rebuild/reinterpret the old baseline under the 103-key schema. Mutating the fixture, any
derivation file, schema, registry key, or source hash must fail.

- [ ] **Step 2: Write exact subset-identity tests**

Define one ordered constant for the eleven design-specified canary relation IDs and one ordered
constant for:

```python
OSAKA_SUPPLEMENT_RELATION_IDS = (
    "opcode:0x15:canonical",
    "opcode:0x1e:canonical",
)
```

Tests reject a missing, reordered, duplicated, additional, renamed, or baseline-unknown canary ID.
They also reject any supplement ID other than the exact two above.

Run the focused test and confirm it fails because the supplement APIs do not exist:

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_osaka_supplement.py' -v
```

- [ ] **Step 3: Write canary gate tests**

Add table tests for:

- identical slope: pass;
- same sign, exactly 10% APE: pass;
- greater than 10% APE: fail;
- aggregate MAPE exactly 5%: pass;
- aggregate MAPE greater than 5%: fail;
- sign change, zero/non-finite slope, raw-gas-map mismatch, program/hash mismatch, repeat failure, or
  baseline artifact mismatch: fail.

The artifact must preserve every observed/baseline slope, signed residual, APE, MAPE, and reason. No
test may observe a fitted replacement coefficient.

- [ ] **Step 4: Write supplemental-run tests**

Use temporary directories and a fake launcher to prove the runner:

- generates and executes only the eleven canaries plus two supplemental relations;
- reuses historical selected counts for canaries;
- applies normal adaptive prefixes/checkpoints only to ISZERO and CLZ;
- requires three repeats and exact provenance;
- writes create-only, atomically completed artifacts;
- resumes only after replaying every persisted row and hash;
- never writes the full-run canonical relation path or invokes proposal/controlled-block commands.

- [ ] **Step 5: Implement the historical validator and bounded runner by composition**

Add `validate_historical_core_opcode_baseline` and `cmd_run_osaka_opcode_supplement`. The former is
the only accepted baseline entrypoint for augmentation. The latter must call existing generation,
guest execution, fitting, precision, and validation helpers. Do not copy the relation state machine.
The runner accepts exactly `--run-path-file`, `--controlled-manifest`, `--baseline-derivation`,
`--historical-manifest`, `--guest-launcher`, and `--elf`; it resolves the calibration directory from
the durable run-path file rather than a shell placeholder.
Store outputs below:

```text
experiments/opcode-gas/runs/<calibration-id>/osaka-opcode-supplement/
  fixtures/
  raw/
  decisions.json
  compatibility-canary.json
  opcode-supplement.json
```

Refuse a dirty implementation/guest input, wrong ELF, wrong baseline, existing conflicting output,
or any requested relation outside the frozen sets.

- [ ] **Step 6: Write exact augmentation tests**

Construct a minimal baseline registry containing SWAP1 and unrelated static/dynamic entries. Verify:

```text
mu(ISZERO) = mu(SWAP1) + d_iszero / 3
mu(CLZ)    = (d_clz + 3 * mu(SWAP1)) / 5
```

The builder must preserve all 101 baseline models, dynamic parameters, `body_scale`, and
`common_dispatch` byte-for-byte in canonical representation; add only the two new keys; report 103
modeled and 47 unsupported named opcodes; replay both new equations; and remain
`candidate_eligible = false`.

Mutation tests must fail on changed baseline bytes/hash, canary bytes/hash, supplement rows/hash,
SWAP1 coefficient, equation map, control opcode, body scale, common dispatch, manifest, guest ELF,
implementation revision, or any altered old model. Reject negative or non-finite solved bodies.

- [ ] **Step 7: Implement create-only augmentation sealing and replay**

Add `cmd_seal_osaka_opcode_augmentation`. Its canonical identity hashes:

- baseline derivation and core/dynamic file hashes;
- Osaka calibration provenance and guest identity;
- canary artifact and source-row hashes;
- supplement decisions, raw rows, and relation artifact hashes;
- controlled manifest and schedule hashes;
- analysis implementation revision and schema version;
- the exact structured `version_identity` object from calibration provenance.

Write a create-only directory:

```text
experiments/opcode-gas/derivations/<augmentation-id>/
  augmentation.json
  compatibility-canary.json
  opcode-supplement.json
  core-opcode-submodel.json
```

Never rewrite the baseline derivation or claim the 101 baseline rows ran under Osaka.

Add `cmd_verify_osaka_opcode_supplement`, which reads the durable run path, raw rows, decisions,
canary, supplement, baseline, and historical manifest and prints the verified calibration ID without
executing a guest. Add `cmd_verify_osaka_opcode_augmentation`, which reads only the augmentation
directory and declared historical fixture, recomputes all hashes and equations, and prints the
verified augmentation ID. Seal and both replay commands reject any changed or missing
`version_identity` field.
The sealer accepts exactly `--run-path-file`, `--baseline-derivation`, `--historical-manifest`,
`--out-root`, and `--augmentation-path-file`. The verifier accepts
`--augmentation-path-file` and `--historical-manifest`; the supplement verifier accepts exactly
`--run-path-file`, `--controlled-manifest`, `--baseline-derivation`, and
`--historical-manifest`.

- [ ] **Step 8: Run all experiment tests and review**

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
git diff --check
```

Independently review the full diff against the architecture design, then fix and recheck every
confirmed finding.

- [ ] **Step 9: Commit the tooling**

```bash
git add experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/tests/test_osaka_supplement.py \
  experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml \
  experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.sha256 \
  experiments/opcode-gas/README.md
git commit -m "feat(zkgas): add Osaka opcode supplement sealing"
```

---

### Task 3: Execute And Seal Only The Osaka Supplement

**Files:**

- Create: runtime data under `experiments/opcode-gas/runs/<calibration-id>/`
- Create: tracked create-only evidence under `experiments/opcode-gas/derivations/<augmentation-id>/`
- Modify: `docs/plans/2026-09-26-zkgas-calibration-progress.md`
- Modify: `experiments/opcode-gas/README.md` only for exact completed commands and result IDs

**Interfaces:**

- Consumes: clean Task 1/2 revisions, rebuilt Osaka ELF, sealed baseline, and materialized manifest.
- Produces: compatibility verdict, two formal relation results, augmented 103-key core artifact or
  an immutable failed supplement record.

- [ ] **Step 1: Freeze a clean calibration identity**

Run exactly:

```bash
cargo build -r -p guest-launcher --features sp1-sdk/profiling
git status --short
sha256sum --check \
  experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.sha256
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-calibration \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --guest-launcher target/release/guest-launcher \
  --out experiments/opcode-gas \
  --run-path-file target/zkgas-osaka-run-path
```

`git status --short` must be empty before `prepare-calibration`. The command durably records the
calibration ID and structured version/guest/launcher/schedule hashes before guest execution.

- [ ] **Step 2: Run the bounded supplement command**

Run exactly:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-osaka-opcode-supplement \
  --run-path-file target/zkgas-osaka-run-path \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --baseline-derivation experiments/opcode-gas/derivations/3e1d97c461cd2ef9a40e6a02 \
  --historical-manifest \
    experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_revm_opcode_lab.elf
```

Monitor bounded subprocesses. The command must not start a complete relation campaign if a canary
or supplemental relation fails.

- [ ] **Step 3: Inspect the canary before sealing**

Run the read-only replay:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py verify-osaka-opcode-supplement \
  --run-path-file target/zkgas-osaka-run-path \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --baseline-derivation experiments/opcode-gas/derivations/3e1d97c461cd2ef9a40e6a02 \
  --historical-manifest \
    experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml
```

It verifies eleven exact rows, per-row APE at most 10%, aggregate MAPE at most 5%, unchanged
signs/maps, and `refit_performed = false`. On failure, preserve the report, mark the progress gate
blocked, and stop this plan without a full resample.

- [ ] **Step 4: Inspect ISZERO and CLZ evidence**

The same replay must require both relations to pass their adaptive, repeat, signal, fit, checkpoint,
trace, provenance, and structured version-identity gates. It recomputes both equations independently
from raw rows before the sealer may run.

- [ ] **Step 5: Seal and independently replay the augmentation**

Run exactly:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py seal-osaka-opcode-augmentation \
  --run-path-file target/zkgas-osaka-run-path \
  --baseline-derivation experiments/opcode-gas/derivations/3e1d97c461cd2ef9a40e6a02 \
  --historical-manifest \
    experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml \
  --out-root experiments/opcode-gas/derivations \
  --augmentation-path-file target/zkgas-osaka-augmentation-path
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  verify-osaka-opcode-augmentation \
  --augmentation-path-file target/zkgas-osaka-augmentation-path \
  --historical-manifest \
    experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml
```

The independent verifier recomputes the augmentation ID, every source hash, both solved
coefficients, the two relation predictions, 103/150 coverage, and all 101 unchanged baseline
entries. Each run, seal, and replay command rechecks the tracked fixture SHA256 internally; the
explicit `sha256sum --check` in Step 1 is the operator-visible preflight for the same invariant.

- [ ] **Step 6: Update progress and commit evidence**

Record exact IDs, revisions, ELF hashes, coverage, commands, review result, proposal-opened status,
and production-change status in the progress ledger. Stage only the declared create-only files and
documentation; exclude locks, caches, raw logs, and transient runtime files.

Use a Conventional Commit message:

```bash
git add "$(tr -d '\n' < target/zkgas-osaka-augmentation-path)" \
  docs/plans/2026-09-26-zkgas-calibration-progress.md \
  experiments/opcode-gas/README.md
git commit -m "chore(zkgas): seal Osaka opcode supplement"
```

---

### Task 4: Freeze The Remaining Operation-Layer Classification

**Files:**

- Modify: `experiments/opcode-gas/opcode_gas.py`
- Modify: `experiments/opcode-gas/tests/test_inventory.py`
- Create: `experiments/opcode-gas/tests/test_operation_coverage.py`
- Create: `experiments/opcode-gas/manifests/operation-coverage-v1.json`
- Modify: `docs/plans/2026-09-26-zkgas-calibration-progress.md`

**Interfaces:**

- Consumes: augmented core artifact, exported Unzen schedule, existing precompile inventory, and
  trace event schema.
- Produces: one manifest with independent execution-coverage and side-effect/event-ownership ledgers
  that is the input to the next state/transaction/block design checkpoint.

- [ ] **Step 1: Write exhaustive classification tests**

Require every named active opcode and precompile body to appear exactly once under
`execution_coverage` with one of:

```text
static_raw_gas
structured_opcode
direct_precompile
inactive_or_unreachable
explicitly_unsupported
```

Require every declared emitted work component to appear exactly once under `side_effect_ownership`
with one owner from:

```text
operation_wrapper
state_trie
transaction
block
```

The tests include `SSTORE` with an opcode-execution entry plus separate dirty-state/final-trie
events, and `CALL` with an opcode-execution entry plus confirmed-wrapper and child-execution events.
Reject unknown schedule keys, duplicate component/event ownership, missing evidence, a measured key
without an artifact reference, an unsupported key without a machine-readable reason, and any schema
that assigns an entire SSTORE/CALL opcode identity to a higher layer.

- [ ] **Step 2: Generate the manifest from sources of truth**

Extend the inventory builder to combine the exported schedule, typed registry, and precompile
inventory into `execution_coverage`, then combine trace event declarations with explicit layer
ownership into `side_effect_ownership`. Do not hand-maintain a second opcode-price table in Python.

- [ ] **Step 3: Review every non-modeled active key**

For each execution key not covered by the 103-opcode artifact, select exactly one declared execution
classification and attach its source evidence. Separately review every side-effect event for exactly
one owner. This step classifies scope; it does not fabricate a coefficient or run a proposal.

- [ ] **Step 4: Verify and commit the operation boundary**

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
git diff --check
```

Independently review the full manifest for missing, duplicate, or wrongly owned work and specifically
replay the SSTORE and CALL multi-component examples. Commit the generator, tests, manifest, and
progress update together.

```bash
git add experiments/opcode-gas/opcode_gas.py \
  experiments/opcode-gas/tests/test_inventory.py \
  experiments/opcode-gas/tests/test_operation_coverage.py \
  experiments/opcode-gas/manifests/operation-coverage-v1.json \
  docs/plans/2026-09-26-zkgas-calibration-progress.md
git commit -m "chore(zkgas): freeze operation coverage ownership"
```

## Plan Exit Boundary

This plan ends after the operation-layer classification manifest is sealed. The next implementation
plan may start state/trie, transaction, and controlled-block calibration only after reviewing that
manifest and the actual Osaka supplement result. Final proposals, cross-backend transport, and
production table promotion remain closed.
