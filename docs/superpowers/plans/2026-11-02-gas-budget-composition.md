# Gas-budget composition implementation plan

> **For agentic workers:** Use subagent-driven implementation with independent
> source review and native/behavioral verification. Existing user authorization
> covers execution; do not stop for another permission question.

**Goal:** Measure32 new matched-control/high-occupancy inputs under frozen pricing.

**Architecture:** Two independent stdlib ProgramRequest generators use existing
frozen native/quote/SP1 tools. Root alone orchestrates sequential actual sampling.

**Tech stack:** Python stdlib/unittest, canonical fixture API, retained SDK6.3 ELF.

**Spec:** `docs/superpowers/specs/2026-11-02-gas-budget-composition-design.md`.

## Constraints and review focus

- Thirty-million gas means charged whole-block receipt gas, with Anchor included;
  full inputs must occupy at least99%, without exceeding the budget.
- Predict gross gas, refunds and user-envelope sufficiency separately; the last
  transaction must fit the remaining native block gas.
- Exact executed operands/body multiplicities and counter guards must resist
  count-preserving mutations, not merely pass a receipt.
- Keep32 distinct inputs and16 exact ownN0 matches, with failed plans retained.
- Freeze model/program before actuals and retain literal and full comparisons.

## Tasks

1. [x] Density implementer owns its generator/tests. Write expected-failing tests,
   implement20 cases, then run focused stdlib and frozen-native smoke checks.
   Hand off a usage fragment and exact file hashes; do not edit shared README.
2. [x] Journal implementer owns its generator/tests. Apply the same TDD cycle to
   12 cases, including refund arithmetic, caught status/marker and semantic
   negative gates. Preserve old producer bytes; no Rust or guest build.
3. [x] Root owns new `bin/zkgas/bench/gas_budget_report.py` and
   `bin/zkgas/bench/tests/test_gas_budget_report.py`. Test receipt arithmetic,
   strict2B boundary, unmeasured/multi-block handling and optional node limit
   before implementing the read-only existing-workload report command.
4. [x] Reviewer independently reads the final diffs/spec and verifies gas,
   counter, stack/state/frame ownership and negative-oracle behavior. Fix and
   re-check findings. Root integrates usage; run full stdlib bench suite once.
5. [x] Tester independently qualifies all32 actual native/quote inputs, checks
   declared gas arithmetic against receipts, captures occupancy and canonical
   binaries, and retains meaningful native/semantic negatives.
6. [x] Root commits the reviewed source/input inventory and freezes configuration.
   Run existing bounded SP1 measure sequentially on qualified inputs; preserve
   every attempt and resource receipt. No coefficient or guest change.
7. [ ] Independently audit raw normalization/input/output/model joins, all32
   comparisons,2B priority, exact ownN0/packing differences and resource cleanup.
   Record research/usage, update the existing pending source PR and retain
   immutable fixtures in Atlas following its existing publication checks.

Use `~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests -p
test_gas_budget_density.py -v` and the corresponding journal module for focused
checks; whole-suite discovery removes `-p`. Native/quote smoke lanes opt in via
existing `NATIVE_FIXTURE_BIN` and `ZKGAS_QUOTE_BIN`. No local Cargo checks are
needed unless an independently justified Rust change becomes necessary.
