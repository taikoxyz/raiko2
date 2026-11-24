# Repricing retained synthetic rollback workloads

Current model5f covers all16 retained full-R rollback workloads using29.782M–29.998M EVM gas.
All32 completed-proposal estimates cover the recorded SP1 actuals. Eight small K16/R0
literal node gaps remain, maximum127,894,499 PGU; none reaches the >2B-within30M screen.
Pricing is unchanged. This is32 fresh native quotes of previously measured synthetic inputs,
with zero new SP1 executions, not another32 new inputs or actual measurements.

## Reused evidence and identities

The original panel spans leaf/ancestor REVERT, K16/256, depth2/4, caller prewarm false/true
and own-R0/full-R at four users:32 inputs. These are controlled post-Unzen synthetic fixtures,
not historical onchain Mainnet blocks. The original reports, signed inputs, receipts and
configured worlds are unchanged. All32 current default requests and parameter objects match
the old goldens exactly; current quote1390 actually decoded and replayed all32 input bincodes.
No decoder or compatibility path was added.

Actuals retain their original estimator identity
`1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b` and null
`source_revision`. They are not relabelled as the newer `dfaee7ac…` register observer.
Both contexts use ELF`b76baf2e…`, program manifest`8d5b4a48…`, SDK6.3 and identical
SDK manifest/lock identities and executor options. Raw gas, complete status, input/public
outputs and `floor(raw ExecutionReport.gas * 10 / 191)` normalization were independently checked.
The strict current-observer verifier correctly rejects the two differing identity fields;
it was neither weakened nor used to certify these reports as new observer executions.

The32 original bincodes total11,546,416 bytes and were available without archive restoration.
Original native constructor identity is`ad0d78dd…`. Its21 relevant primitive/Shasta source files
still match, and actual current decode/replay supplies the compatibility check. This supports
historical-program workload validation; it does not certify a current production ELF or SDK.
These are opened development data, not an unopened holdout or an IID sample.

## Results and the additive change

The old quoted model`c8936124…` is a previous **new-pricing** snapshot. It is not the Unzen
model, and neither old nor new PGU estimates here are inferred from raw Unzen difficulty.
Every row compares the same input's original complete actual PGU with literal node and
completed-proposal estimates under the explicit sealed model5f.

| Body K / level | Rows | Block gas range | Recorded actual PGU range | Current node PGU range | Node gaps |
| --- | ---: | ---: | ---: | ---: | ---: |
| 16 / R0 | 8 | 699,831–844,291 | 165,516,561–166,110,629 | 37,784,171–38,675,811 | 8 |
| 16 / full-R | 8 | 29,986,103–29,997,635 | 883,799,037–887,059,584 | 1,389,331,435–1,396,247,632 | 0 |
| 256 / R0 | 8 | 6,935,031–8,818,051 | 216,073,095–219,480,213 | 340,178,394–349,990,790 | 0 |
| 256 / full-R | 8 | 29,782,327–29,984,247 | 725,477,674–781,996,971 | 5,722,213,915–6,249,779,891 | 0 |

The previous c893 node quote fell below actual in32/32 cases, including16/16 full-R cases;
its maximum deficit was128,975,075 PGU. Under5f this becomes8/32, all K16/R0 controls.
Both models' completed-proposal quotes cover all32 recorded actuals. This is an in-program
repricing comparison between the two declared snapshots, not a comparison against Unzen
and not evidence that any coefficient was refitted in this round.

Independent comparison finds every non-journal pricing object, resource/MPT/payload selector
and public output unchanged. The node and completed-proposal PGU differences are exactly
`384 * journal_work_units`. For this family:

```text
C = ceil((K + 8)/8)
journal_work_units = 4*C*(14*K + 6 + 4*K*R) + 54
proposal_once journal_work_units = 0
```

This is the existing comparison-work reserve, not a measured hash-probe count. Leaf versus
ancestor placement and depth do not change that journal term for equal K/R/warmth. Generated
frames retain their original EVM success and rollback semantics.

For full-R cases node/actual is1.572–7.992x and proposal/actual1.749–8.213x. High conservatism
on deliberately repetitive dirty writes is retained. It does not establish normal onchain
transaction frequency or admission usability, justify a global discount, or bound worst-case
work for arbitrary30M-gas blocks. The remaining small literal node gaps are not discarded or
multiplied as complete proposal startup once per hypothetical block; their precise causal
allocation is not established by these aggregate comparisons.

## Verification and retention

Each new quote ran once with the frozen1390 executable and model
`5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11`, using
`evm-gas-only`. The existing process-tree guard retained2GiB PSS,180s,16MiB output,2MiB log
and576MiB free-disk limits. There were zero retries, model/runtime changes, guest/SDK builds or
new SP1 measurements. All old file hashes were rechecked before and after quoting.

Independent readback verified all32 raw reports/native inputs/new quotes,200 source/input
bindings,32 guard outcomes, exact tariff arithmetic, once-only composition and non-journal
object equality. The index SHA is`2721260f47e678e55afa6843f2aa5207cc6bb114ead435fe568ad096084f8221`;
new quote summary SHA is`b2d1387544131916d3ad90351aa4279a6088c08c917dbbede50cce6be813eab5`.
Original data and both quote snapshots belong in Atlas; executables and strategies stay with
their consumers. No original report identity is rewritten to make an interface match.

The [new eight-input rollback panel](2026-11-05-storage-rollback-capacity.md) is a separate
cohort. Its incomplete auxiliary register diagnostics remain incomplete; this repricing
does not repair or extend them. Account-journal correction, future L1 frequency sampling,
other workload compositions and the later unrestricted-workload campaign remain separate TODOs.
