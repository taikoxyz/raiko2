# Gas-bounded shortfall and the cost of pricing complexity

The current worklist uses the
[evaluation standard's gas-bounded priority rule](../zkgas-evaluation-standard.md#gas-bounded-priority-and-pricing-complexity)
across all pricing paths, including possible simplification of earlier rules.
The practical decision balances missing workload against the complexity of
observing and pricing it. A measured discrepancy need not immediately produce
a new selector, special branch or tariff. The present focus stays block-aware
workload pricing; the rule helps choose which gaps justify that work.

Existing opcode, witness and level-aware rules are eligible for retrospective
comparison with simpler candidates. Retain both price surfaces, identical-program
actuals, fixed controls and normal-workload compatibility. Apply the threshold to
the combined block, including freely mixed transactions under the same gas budget;
per-family results alone cannot classify every mixture. Published fixtures and
historical results remain unchanged when a new candidate is evaluated.

## What the187.5M storage increment actually consumed

Use the frozen `c8936124…`26-input storage panel, SDK6.3/ELFb76 execute-only
actuals and original receipt gas. The source data is retained in
[Atlas #24](https://github.com/taikoxyz/raiko2-atlas/pull/24) and its
[immutable Release](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-storage-composition-growth-v1).
The example is `storage-access-present-s16384-n4096`, compared with its ownN0.

| Quantity | OwnN0 |4096 distinct present-slot reads |
| --- | ---: | ---: |
| Charged user transaction EVM gas |23,232|8,844,288|
| Charged whole-block EVM gas, including Anchor |190,587|9,011,643|
| Actual proposal SP1 PGU |161,319,444|348,839,656|
| Node estimate PGU |7,685,962|70,769,586|
| Full proposal estimate PGU |163,040,329|412,610,793|

Therefore187,520,212 additional actual PGU consumed8,821,056 additional EVM gas.
It did not arise from a negligible-gas transaction. Node pricing increased by
63,083,623.845PGU; the measured whole-input node shortfall is278,070,070PGU.
This is a one-block controlled proposal, not an allocation of a larger proposal's
actual workload to that block.

## Illustrative30M projections, not new actual measurements

For a matched series, keep baseline startup once and project only its increment:

```text
t = (30,000,000 - G0) / (G1 - G0)
A_projected = A0 + t * (A1 - A0)
Q_projected = Q0 + t * (Q1 - Q0)
D_projected = max(0, A_projected - Q_projected)
```

`G` is whole-block receipt gas. Calculations use unrounded scaled quote values
for the projected node total; original literal rounded estimates remain retained.
The formulas below are exact-rational arithmetic rounded only for display.

| Assumed growth segment | Projected actual at30M, billion PGU | Projected node, billion PGU | Projected node shortfall, billion PGU |
| --- | ---: | ---: | ---: |
| Present16384,N0→4096 |0.7950|0.2209|0.5741|
| Present16384,N0→1024 |1.0295|0.2209|0.8086|
| Present16384,N0→64 |1.7147|0.2209|1.4938|
| Dirty REVERT,N0→32768 |0.8774|0.6946|0.1828|

Shorter storage series have higher observed workload per gas because proof
sharing differs. The64-read slope is retained rather than selecting only the
cheaper4096-read slope. Extending that slope is not an actual measurement of
packing many small transactions or independent accounts; it does not account
for all changes in signatures, state shape, shared proofs or other composition.
These projections are neither a saturation result nor a conservative upper bound.

Across the65 single-block inputs from the two panels, the largest measured node
shortfall is278,070,070PGU. The largest measured block gas usage is10,255,483;
none is a30M saturated block. The six four-block inputs remain separate
whole-proposal observations and are not divided into invented block actuals.

## Current worklist consequence

The observed deficits and these projections do not establish a greater-than2B
shortfall at30M. Record the storage and dirty-rollback findings as provisional
low-priority items, with saturation and mixed-composition coverage still open.
Their earlier source attribution and underpricing flags remain valid. This
triage does not assert a maximum workload, a production deadline or filter
admissibility at an unspecified node limit.

Continue finding and correcting consequential block-aware gaps. Give priority
to a valid gas-bounded combination that exceeds2B missing PGU, or to a small,
well-supported correction whose implementation cost is justified. Complicated
MPT topology instrumentation and extra mutation-accounting machinery can remain
TODOs while their gas-bounded impact is below the priority threshold in available
evidence. Prospective saturation and composition tests can raise their priority.
The same evidence can later justify simplifying old rules instead of continually
adding special cases. No pricing, runtime selector, gas limit, guest, SDK or
released fixture changes in this record.
