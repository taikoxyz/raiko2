# Ordinary-path PGU tariff recalibration

This experiment investigates why the candidate meter tracks the completed Hoodi
mixed-spam cohort less accurately than Unzen. PGU remains the measurement target;
GPU proving time is a separate future experiment. The opened spam observations
are development/regression data for this correction, not untouched validation.

## Baseline comparison

The eight-hour trial completed 240 slots and 3,840 verified transactions. It
contains 18 training proposals, 55 evaluation proposals and three boundary
proposals. Each score receives its own positive global scale from training only.
The following errors are measured against the same whole-proposal SP1 PGU:

| Diagnostic | Unzen | Candidate node block sum | Candidate full proposal |
| --- | ---: | ---: | ---: |
| Evaluation WAPE | 6.28% | 16.32% | 8.70% |
| Evaluation Spearman correlation | 0.513 | 0.669 | 0.689 |

A training-only constant predictor has 6.06% WAPE. These are repeated synthetic
recipes over a narrow workload range, so this cohort does not establish broad
application accuracy. Unzen wins average magnitude accuracy here; the candidate
has better rank ordering. Neither result by itself demonstrates capacity
protection. Raw Unzen difficulty is **not PGU**.

Literal candidate node quotes sum to 1.371x actual and full quotes to 2.400x;
neither underquotes any of the 55 evaluation proposals. Those literal ratios
must remain separate from the independently scaled workload-shape diagnostics.
No proposal in this trial exceeds the illustrative 3.6B-PGU capacity reference.

## Identified pricing problems

Normal successful routes retain a 5M-PGU accepted event price for CODESIZE,
small CODECOPY, LOG3/LOG4 and confirmed STATICCALL. Route-maximum composition
keeps that price even when an incremental candidate is much smaller. Actual
Hoodi opcode traces confirm these prices affect ordinary transfers, calls and
creation paths. A Solidity scenario labelled `call` actually executes STATICCALL;
it must not be reported as CALL coverage.

The inherited fallback conversion also used whole-input PGU / Unzen-score
ratios without an intercept. That mixes fixed program cost into a putative
marginal conversion. A descriptive affine fit is a diagnostic, not a universal
replacement coefficient. This correction leaves unmeasured fallback routes and
the global conversion unchanged.

More terms do not guarantee lower prediction error: many existing coefficients
come from conservative positive residuals, maxima, floors and margins rather
than one jointly minimized prediction loss. Separate components can accumulate
conservatism. Component-removal diagnostics alone did not resolve the spam error.

## Frozen experiment

The reusable producer is
[`ordinary_tariffs.py`](../../bin/zkgas/bench/cases/ordinary_tariffs.py), with usage
in the [bench README](../../bin/zkgas/bench/README.md). It constructs legal
`evm-gas-only` inputs with exact gas, completed/attempted counts, result guards,
matched N0 setup and explicit prestate contracts. Native construction enforces
full/minimized witness replay equality. Independent tests decode emitted LOG
topics/data; native reports themselves do not export raw log arrays.

- Program ELF: `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`.
- SDK/executor/runner: 6.3.0; execute-only CPU gas estimator.
- PGU: `floor(execution_report.gas * 10 / 191)` at each observed endpoint.
- Parent model: `eea6f1da87ee129d2d9f759ae6cef01cb8ac93988376a86efb290398e6db5da6`.
- Selection: 31 new fit inputs, 14 reserved inputs, 49 unselected catalog rows.
  One common-input observer parity run permits reuse of 15 retained LOG4 rows.
- All 46 selected new invocations, including parity, completed successfully;
  there were no automatic retries. The two remaining budget slots were unused.
- Resource limits: 6GiB process-tree PSS, 1,200 seconds per stage, bounded output
  and exact owned SDK-resource cleanup. The normal collector stays on its
  original pins and shares an execution lock with finite calibration runs.

Same-input old/current observer reports match complete execution reports, raw
gas, PGU, public output and instruction/syscall counts. Only executable/source
provenance and elapsed time differ; retained-event preset ordering is normalized.
This is local-observer equivalence, not production SDK or GPU parity.

## Candidate and reserved validation

For each operation/length/pattern, compare all fit target count pairs. Use exact
PGU endpoint differences divided by additional executed events. Variable loop,
operand preparation and result-check costs remain included; paired scaffolds
are diagnostics and receive no subtraction credit. LOG4 has two users, hence
`2 * repetitions` events.

Fixed candidates are `ceil_to_64(1.25 * maximum_positive_marginal_PGU)`.
LOG3/4 have separate zero-length bases and one common nonnegative per-word
residual envelope, with the same margin/rounding. No high-degree fit is needed.

| Accepted successful-event component | Parent | Sealed candidate |
| --- | ---: | ---: |
| CODESIZE, exact raw gas 2 | 5,000,000 | 2,048 |
| CODECOPY, raw gas at most 12 | 5,000,000 | 1,600 |
| Confirmed STATICCALL body | 5,000,000 | 9,280 |
| LOG3 base | 5,000,000 | 45,888 |
| LOG4 base | 5,000,000 | 64,064 |
| LOG3/4 additional data word | shared memory only | 2,688 plus inherited memory |

These are component prices, not assigned per-opcode actual PGU. Existing
incremental/structural maxima remain; in particular STATICCALL still has its
18,291-PGU incremental event candidate. CODECOPY outside the small envelope,
feature-error pricing, shared memory and independent workload protections are
unchanged. LOG pricing is base plus `2,688 * ceil(data_bytes / 32)` plus memory.

Candidate measurement-fact SHA256:
`255f2589e02efab825ed6cf1364b64d64a946da3322036fe54cac69b47f7a44e`.
It was sealed before any reserved PGU execution. Independent reconstruction
reproduced all coefficients from 46 measurement facts and 34 fit count pairs.
All 14 reserved actuals passed integrity checks; all 27 applicable incremental
comparisons retained the sealed 25% margin, with no refit. COPY97 is a separate
unchanged-fallback boundary diagnostic, not evidence for applying the ordinary
1,600-PGU body outside raw gas 12.

This validates finite marginal observations, not a universal bound. The generated
model is `c0877d65a0b5fac178342645b5689001e0801718fe1dae0150c59c39ee1c15c1`.
Its source replay, both generators, 67 Python pricing tests, eight independent
compiled evaluator tests and all 273 focused EVM `zk_gas` tests passed. Strict
targeted EVM clippy passed. Full `just clippy` stopped in the environment's rustup
bootstrap before Rust checking; it is not recorded as passing. No guest or SDK
was rebuilt.

## Complete quote regression

All 155 selected inputs completed: 32 retained pressure controls, 45 new controlled
inputs, 76 opened spam proposals and two later natural proposals. Every actual
report is joined to the canonical input, program, public output and native/quote
records. All baseline quotes use the authenticated EEA model. Nonoperation
selectors and prices remain equal between parent and successor.

The initial controller selected ten legacy retained quote pointers instead of
their existing EEA requotes. The isolation check stopped that run. Its evidence
was preserved; audited EEA pointers and explicit parent-model checks corrected
the index. Twenty-three valid successor quotes were strictly revalidated and
reused, with no actual measurement repeated. This was a controller provenance
error, not a pricing or execution change.

Using the same original 18 training and 55 evaluation proposals, with the three
boundary proposals excluded, each model gets its own training-only scale:
`scale = sum(training actual PGU) / sum(training score)`.

| Evaluation diagnostic | Unzen | Parent node | New node | Parent full | New full |
| --- | ---: | ---: | ---: | ---: | ---: |
| Scaled WAPE | 6.28% | 16.32% | 14.91% | 8.70% | 8.47% |
| Spearman | 0.513 | 0.669 | 0.756 | 0.689 | 0.762 |

The correction improves ranking and magnitude accuracy, but **does not beat
Unzen's average prediction error on this cohort**. The constant training baseline
still has 6.06% WAPE. These opened observations are development evidence.

Literal node sum / actual falls from 1.371x to 1.202x; literal full / actual falls
from 2.400x to 2.230x. Neither successor surface underquotes any of the 55 proposals.
The 32 pressure controls retain nine node and three full underquotes, with maximum
positive gaps unchanged at 1,597,968,115 and 1,261,582,685 PGU respectively. No new
underquote appears. All five single-block controls above 3.6B actual PGU remain
above 3.6B in node quotes; those same inputs fit current Unzen's 100M/block cap.
This compares a capacity reference, not Unzen's original proving-time guarantee.

Small controls still expose the node/proposal-once distinction: 13/14 ordinary
reserved inputs have node sums below complete proposal PGU, with a largest gap
of about 153.43M; none has a full-quote shortfall. Preserve these startup gaps.
The two later quiet proposals have node/actual 1.01766x and full/actual 2.09142x.
Two samples do not certify broader accuracy or capacity.

## Remaining error: an input-insensitive PC07 floor

All 15 known recipe families were inspected. The correction removes 17.560B PGU
from evaluation node quotes; matched single-campaign-transaction blocks explain
99.915% of that reduction. These are quote components, not assigned actual PGU.

Remaining PC07-related families account for 69.277% of matched non-dispatch body
charges while occupying only 72 of 2,798 matched evaluation blocks. Literal,
unpaired proposal means expose the skew:

| Observed arm | Actual PGU | Node quote | Node / actual |
| --- | ---: | ---: | ---: |
| Quiet control | 1.7573B | 1.7882B | 1.01761x |
| General | 1.7838B | 1.8581B | 1.04161x |
| Targeted without ECMUL | 1.8233B | 1.9449B | 1.06668x |
| Mixed | 1.8323B | 3.0174B | 1.64675x |
| Scalar sensitivity | 2.1179B | 2.4686B | 1.16560x |

These groups differ in content and state history. They do not measure the actual
cost of an individual transaction or opcode. A global scale cannot align these
different relative quote levels.

The existing PC07 route chooses the maximum of accepted, incremental and
structural route aggregates. For one successful 6,000-native-gas call:

- Accepted: `max(20M, 131.39022498… * native_gas)` = 20M PGU.
- Incremental: `29,901 * native_gas` = 179.406M PGU.
- Structural: `572,936 * work_units`.

Scalar information already exists. Both execution paths forward the reduced
BN254 scalar's bit length plus popcount through `OperationModelInput::WorkUnits`.
Scalar 2 has three units, giving a 1,718,808-PGU structural charge, yet the
unconditional 179.406M native-gas term wins. The known nested mixed recipe calls
PC07 twice with point `(1, 2)` and scalar 2: 358.812M quoted PGU per transaction.
Its known occurrences average 1.076436B per mixed proposal and explain 87.6% of
the mixed-versus-control mean **quote** difference.

A descriptive ablation that removes only these known nested route charges and
repeats the same training-only normalization gives 3.554% evaluation WAPE.
Removing all known PC07 charges instead gives 5.781% and worsens the direct
scalar arm, removing useful variation. Neither ablation is a price candidate or
a measurement of actual PC07 cost. It locates an overly broad floor regime.

The next calibration should test floor applicability using the existing scalar
selector. It needs controlled same-program count differences for cheap and
heavy scalars, multiple valid points and mixed calls, followed by retained heavy
regressions. Preserve heavy-input protection; do not replace prices from the
opened-spam ablation or discard PC07 charges globally. No new scalar feature is
required merely to begin that investigation.

## Reporting requirements

The [published 60-input Atlas fixture](https://github.com/taikoxyz/raiko2-atlas/tree/fixture-unzen-ordinary-tariff-recalibration-20261110-v1/fixtures/benchmarks/unzen-ordinary-tariff-recalibration-20261110-v1)
retains original controlled inputs, requests, native/SP1 reports, EEA/successor
quotes, parity and standard manifest views. Its supplemental 155-row numerical
projection supports checking these summary statistics; it does not claim those
additional raw inputs are inside the 60-input archive. Consumers supply their
execution software and program artifacts.

Report literal node block sums, complete proposal quotes and actual proposal PGU
separately. Preserve absolute N0/startup gaps, all failures and known residuals.
Do not distribute whole-proposal PGU among transactions or blocks. Repricing the
opened spam cohort is development evidence; later untouched inputs are required
for prospective validation. Preserve the existing high-workload panel and flag
any newly exposed gap above 2B PGU within a justified 30M-gas block budget.

Neither pending pricing/tool PR is authorized for merging or deployment by this
experiment.
