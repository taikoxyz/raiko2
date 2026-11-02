# Transaction order and cold-frame implementation plan

> **For agentic workers:** Use subagent-driven development with independent
> source review and behavioral verification. Existing user authorization covers
> execution; do not stop for another design-approval question.

**Goal:** Measure36 new matched-control/full inputs for transaction order and
repeated child-access workload under frozen pricing.

**Architecture:** Two Python stdlib producers use canonical ProgramRequest and
existing native/quote/SP1 tools. Root owns docs, sequential actuals and integration.

**Tech stack:** Python stdlib/unittest, existing feature-gated fixture API,
SDK6.3 execute-only ELF. No Rust/guest/SDK/pricing change.

**Spec:** `docs/superpowers/specs/2026-11-02-order-cold-frame-design.md`.

## Constraints and review focus

- 30M means charged block receipts including Anchor; native header remains31M.
- All full rows must occupy at least99% without exceeding30M charged gas.
- Signed users share calldata/target/envelope; contract state selects order.
- Track phase-slot originals per transaction separately from journal slot2.
- Check nonempty calldata intrinsic and applicable floor gas, including N0.
- Rollback restores only warmth introduced after the checkpoint, not outer warmth.
- Coldness does not imply DB reread/MPT materialization; retain actual selectors.
- Preserve semantic native-success negatives, rejected candidates and all controls.
- Modelc893 / SDK6.3 / ELFb76 stay frozen; incomplete actuals stay unknown.

## Tasks

1. [x] Order producer owner creates
   `bin/zkgas/bench/cases/order_composition.py` and
   `bin/zkgas/bench/tests/test_order_composition.py`.
   Write expected-failing tests, implement24 inputs, validate bounded actual
   operand/state histories and full count/gas formulas, retain meaningful
   order/phase/read/write mutations. Existing producers remain byte-identical.
   Native smoke precedes any actual. Hand off source hashes and usage fragment;
   do not edit shared README, run SP1, commit or push.
2. [x] Cold-frame owner creates
   `bin/zkgas/bench/cases/cold_frame_composition.py` and
   `bin/zkgas/bench/tests/test_cold_frame_composition.py`.
   Apply the same TDD cycle to12 inputs, concrete warmset checkpoints and child
   markers, exact zero/full gas arithmetic and bounded cold/prewarm controls.
   Keep child return/revert and transaction reset semantics explicit. Hand off
   source hashes and usage; no shared README/backend build/SP1/commit/push.
3. [x] Reviewer independently checks spec and both complete diffs for scoped
   labels, dispatch/input bounds, operand guards, state/original/refund semantics,
   exact counts/gas, cold rollback and expected-negative handling. Authors fix
   confirmed findings; reviewer rechecks up to two material iterations.
4. [x] Root integrates usage; run the complete stdlib bench suite. Independent
   tester qualifies all36 canonical native/quote inputs and retained negative
   controls, including ordered signed input identity and exact receipt gas.
   Native descriptors/31M remaining envelopes must match predictions.
5. [x] Root commits coherent reviewed source and freezes final source/catalog/
   qualified inventory/config before sequential SP1 measurements. Use existing
   bounded measurement tools; keep all attempts and owned-resource cleanup.
6. [x] Independently audit canonical joins, normalization, both literal/full
   shortfalls,18 exact baselines, priority and resources. Root records actual
   results with finite scope, updates pending #296 and retains data-only Atlas
   fixtures following preseal/archive/fresh-download checks. No automatic merge.

Completed evidence: [measured research record](../../research/2026-11-02-order-cold-frame.md),
[Atlas #26](https://github.com/taikoxyz/raiko2-atlas/pull/26) and immutable
`fixture-unzen-order-cold-frame-boundaries-v1` Release. No price changes;
the severe REVERT overquote remains explicit alongside coverage results.

Focused commands use `~/.venv/bin/python -S -m unittest discover -s
bin/zkgas/bench/tests -p test_order_composition.py -v` and the corresponding
cold-frame module. Full discovery removes `-p`. Optional native/quote lanes use
existing `NATIVE_FIXTURE_BIN` and `ZKGAS_QUOTE_BIN` with retained frozen binaries.
