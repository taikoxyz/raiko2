# FI Fee Sensitivity Implementation Plan

> **For agentic workers:** Use the existing isolated workload-tools worktree. Implement the calculator and focused tests, then obtain independent whole-diff review and CLI verification.

**Goal:** Reproducibly compare FI backlog fees with explicitly assumed incremental SP1 cost,
without changing zkGas prices or claiming an Etna measurement.

**Architecture:** A standalone standard-library Python CLI reads a strict bounded JSON study
configuration and produces deterministic JSON. It compares linear and interpolated doubling
backlog curves with exact arithmetic. Source/version scope and the Etna P04/P07 measurement
contract stay in the research document.

**Tech stack:** Existing Python virtual environment, standard library, unittest.

**Spec:** `docs/research/2026-11-19-fi-workload-fee.md`.

## Global constraints

- No runtime/consensus/fee/tariff, dependency, ELF/VK or timer changes.
- Tools remain in Raiko; Atlas receives actual immutable fixtures when available.
- Label all outputs as sensitivity only; distinguish partial SP1 cost coverage from full proof
  cost and capacity acceptance. No implicit actuals, market prices or default-zero missing costs.
- Use bounded exact integer/rational arithmetic, explicit units and no binary floating point.
- Preserve unrelated untracked plans and the primary checkout.

## Review focus

- A huge pending/threshold ratio must report overflow without allocating a huge integer.
- A partial batch reaching uint256 overflow must not appear to be a completely priced batch.
- Economic coverage must use the reserve-adjusted cost with ceiling-to-wei, not a rounded USD
  display or a fiat price inferred from the environment.
- Existing output files and malformed/duplicate-key configurations must be preserved/rejected.
- Consumed-but-unlanded entries stay pending; the no-head-advancement batch assumption is explicit.

## Task 1: Offline calculator and focused regression

**Files:**
- Create `bin/zkgas/bench/fi_fee_report.py`.
- Create `bin/zkgas/bench/tests/test_fi_fee_report.py`.
- Create `bin/zkgas/bench/examples/fi-fee-sensitivity.json`.
- Update `bin/zkgas/bench/README.md` with the independent tool's usage and limits.

- [x] Write meaningful failing tests for the fee curves, known 900-entry linear example,
  cost/wei conversion, overflow, malformed inputs, deterministic CLI and no-overwrite behavior.
- [x] Implement the bounded calculator and CLI using only the standard library.
- [x] Run the focused tests and existing Python bench regression as appropriate.

## Task 2: Retained numerical result and independent verification

**Files:**
- Update `docs/research/2026-11-19-fi-workload-fee.md` with observed arithmetic and limitations.
- Create `docs/research/data/2026-11-19-fi-fee-sensitivity.json` from the checked-in example.

- [x] Run the actual CLI; independently reproduce the baseline and selected doubling/cost rows.
- [x] Review the complete final diff and adversarial inputs independently; fix/recheck findings.
- [x] Verify portable paths, unchanged runtime/tariff/ELF inputs and untouched primary changes.
- [x] Commit only this deliverable with Conventional Commits. Keep pending PRs unmerged.

Real FI PGU and legal two-proof acceptance remain dependent on Etna P04/P07. This offline phase
does not mark those milestones complete.
