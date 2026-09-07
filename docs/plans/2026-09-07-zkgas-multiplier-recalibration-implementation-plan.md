# ZKGas Multiplier Recalibration Incremental Plan

## Status

Review-only delta against
`docs/plans/2026-06-08-sp1-opcode-prover-gas-experiment-implementation-plan.md`.

After this delta is reviewed, fold the accepted tasks into that existing implementation plan and
delete this file. Do not leave two authoritative implementation plans for the same experiment.

> **For Codex:** Use `superpowers:executing-plans` to implement this plan task by task, and
> `superpowers:test-driven-development` for each behavioral change.

**Goal:** Reuse the existing opcode experiment framework to produce one fixed SP1 multiplier
recalibration report from saved Mainnet and Hoodi proposal fixtures.

**Architecture:** Keep the existing generator, SP1 lab guests, batch launcher, schedule exporter,
fit code, and inventory. Add immutable run/corpus metadata around them, add one host-native EVM
breakdown path with exact current-schedule parity checks, then generate a candidate table and fixed
proposal validation report. Nothing promotes the candidate or changes runtime behavior.

**Tech stack:** Rust/revm for proposal tracing, existing `guest-launcher`, Python 3.11 standard
library via `~/.venv`, JSON/JSONL/Markdown artifacts.

**Design:**
`docs/plans/2026-09-06-zkgas-multiplier-recalibration-experiment-design.md`

---

## Existing Foundation To Reuse

Do not rebuild these components:

- `xtask export-unzen-zk-gas-schedule` and its Cargo-pinned schedule source;
- `experiments/opcode-gas/opcode_gas.py` generation, batching, fit, damage, and inventory helpers;
- `sp1-revm-opcode-lab`, `sp1-precompile-lab`, and their existing fixture templates;
- `guest-launcher` local SP1 execute and normalized execution metrics;
- the current opcode experiment test suite.

Unmeasured state/environment, CALL/CREATE, halting, CLZ, and p256 entries retain their current
schedule values in the first candidate. They remain explicit coverage gaps in the report.

## Task 1: Freeze One Run And Proposal Corpus

**Files:**

- Modify: `experiments/opcode-gas/opcode_gas.py`
- Create: `experiments/opcode-gas/tests/test_run_manifest.py`
- Create: `experiments/opcode-gas/manifests/proposals/sp1-mainnet-hoodi-v1.json`
- Modify: `experiments/opcode-gas/README.md`

Add the smallest orchestration layer needed to:

- create `experiment.json` from the current source/dependency/SDK/ELF/schedule identities;
- validate repository-relative proposal paths and GuestInput SHA256 values;
- batch the fixed corpus through the existing proposal runner;
- resume only matching, valid completed rows;
- reject mixed provenance, duplicate cases, modified fixtures, and incomplete final reports.

Write failing tests first for deterministic identity, fixture hash mismatch, duplicate proposal,
resume without duplication, and provenance mismatch. Then implement `prepare-run` and
`run-proposals` without changing existing command behavior.

**Focused verification:**

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_run_manifest.py'
```

## Task 2: Add EVM ZKGas Breakdown With A Consensus Oracle

This is the main implementation and review task.

**Files:**

- Create: `crates/stateless/src/zkgas_trace.rs`
- Modify: `crates/stateless/src/lib.rs`
- Modify: `crates/stateless/src/validation.rs`
- Modify: `bin/guest-launcher/Cargo.toml`
- Modify: `bin/guest-launcher/src/main.rs`
- Create: `crates/stateless/tests/zkgas_trace.rs`

Add an opt-in host-native `proposal-trace` path. It executes the canonical blocks already present in
each saved GuestInput with an inner revm inspector and records:

- opcode execution count and actual raw EVM gas by opcode;
- CALL/CREATE spawn versus non-spawn charging inputs;
- precompile address, call count, native gas, and input-size buckets when observable;
- transaction intrinsic zkGas count;
- block and transaction counts;
- current-schedule charge by component and total.

This path is used only by the experiment command. Do not change the ordinary proposal guest,
production executor selection, schedule selection, or online prover behavior.

The primary correctness oracle is exact reconciliation for every fixed canonical block:

```text
trace_current_schedule_total == block.header.difficulty
```

Fail the trace if the equality does not hold. Do not weaken it to a tolerance.

Write focused tests before the implementation for:

- ordinary fixed-gas and dynamic-gas opcodes;
- CALL/CREATE spawn and non-spawn paths;
- precompile dispatch without double-counting the wrapper;
- transaction intrinsic charging;
- reverted transactions, which still consumed proving work;
- invalid/failed block execution producing no accepted trace;
- one saved Mainnet and one saved Hoodi proposal reconciling every block exactly.

The SP1 report's existing `opcode_counts` are RISC-V instruction counts and are not an EVM trace;
do not use them as the proposal feature vector.

**Focused verification:**

```bash
cargo test -p raiko2-stateless --test zkgas_trace
cargo test -p guest-launcher proposal_trace
```

## Task 3: Build And Validate The Candidate Table

**Files:**

- Modify: `experiments/opcode-gas/opcode_gas.py`
- Create: `experiments/opcode-gas/tests/test_candidate_report.py`
- Modify: `experiments/opcode-gas/README.md`

Reuse accepted SP1 microbenchmark slopes. Convert relative SP1 cost into integer zkGas multipliers
with one deterministic global normalization: on the fixed `fit` split, preserve the current table's
median proposal total zkGas. Keep current values for unmeasured entries and report every retained
entry.

Add `fit-multipliers` and `report` steps that:

- reject non-finite, non-positive, duplicate, or overflowing coefficients;
- state the rounding rule and validate the final `u16` representation;
- recompute current and candidate totals from the same proposal feature rows;
- fit/report proposal, block, transaction, witness/input/blob, and residual overhead separately;
- report per-network and combined MAPE, maximum error, underprediction count/max, and worst rows;
- emit JSON, Markdown, exact candidate JSON, and a review-only Rust table rendering;
- never write into alethia-reth or generated ELF paths.

Write tests first for deterministic normalization, preserved unmeasured entries, integer rounding,
overflow rejection, holdout isolation, exact report recomputation, and a no-production-write guard.

**Focused verification:**

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_candidate_report.py'
```

## Task 4: Run The Fixed Experiment And Freeze The Report

**Files:**

- Create: `experiments/opcode-gas/runs/<run-id>/experiment.json`
- Create: `experiments/opcode-gas/runs/<run-id>/proposal-manifest.json`
- Create: normalized raw, fit, candidate, `report.json`, and `report.md` files under that run
- Modify: `experiments/opcode-gas/README.md` with the exact completed-run command sequence

Run the existing synthetic suite with the revm-backed opcode lab and direct precompile lab, trace
the fixed proposal corpus, execute those proposals locally with SP1, then generate the final report.
Do not submit proofs or query live RPC after fixtures are fixed.

Before freezing the report:

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
cargo fmt --all -- --check
cargo test -p raiko2-stateless
cargo test -p guest-launcher
cargo clippy -p raiko2-stateless -p guest-launcher -- -D warnings
```

Independently review the complete implementation diff, with particular attention to Task 2's
CALL/CREATE, precompile, transaction, revert, and block reconciliation behavior. Independently rerun
the two-network fixture reconciliation and one report recomputation before declaring the report
fixed.

The final handoff contains the immutable run ID, exact command results, proposal corpus hashes,
coverage gaps, current-versus-candidate metrics, and candidate table. Updating alethia-reth remains
a separate manual developer decision and separate PR.
