# Cap-constrained zkGas workload comparison

This is the original `c0877d65…` checkpoint. The
[frozen current-model validation](2026-11-11-frozen-model-capacity.md) retains its
matched-input comparison and expands the eligible panel after the precision adjustments.

The candidate has a measured capacity-control advantage on selected hidden-state/container
workloads. It does not dominate every workload family: several ordinary dense-opcode cases
already fit the old Unzen cap at modest actual cost, while small SHA256 repetition is
substantially overquoted by the candidate. This experiment freezes prices and compares
workload inside each budget; rejecting a near-30M EVM-gas input alone is not protection evidence.

## Method and scope

- Fixed model: `c0877d65a0b5fac178342645b5689001e0801718fe1dae0150c59c39ee1c15c1`.
- Fixed research ELF: `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`;
  SDK/executor/runner6.3.0, execute-only gas estimator. Actual PGU is exactly
  `floor(execution_report.gas * 10 / 191)`.
- 32 retained pressure inputs plus45 ordinary calibration inputs (31 fit,14 reserved),
  reused without new execution; eight new under-cap measurements.85 measured inputs total:
  73 single-block and12 multi-block proposals. Four above-boundary inputs remain native-only.
- Selection was sealed before new truth. Four families each target raw Unzen80M and100M;
  native/quote evidence for the next repetition above100M is retained. Eight of12 allowed
  new SP1 attempts used, zero failures/retries. Four slots remain unused.
- Two observer executables are kept in separate strictly pinned report manifests. Nine
  retained actuals use the earlier observer. The prior same-input full-report parity gate
  was reverified before the numerical union, including instructions/syscalls/raw gas;
  only executable/source/wall-time provenance and preset ordering differ. This is finite
  observer parity, not production backend or GPU certification.
- Historical admission sensitivity uses50 retained Mainnet inputs dated2026-10-27..29,
  9,600 block records, newly quoted with the fixed model. Their older-program actuals
  are excluded. This panel is historical regression, not recent or representative traffic.

Raw Unzen scores stay in their own unit. The old rule is raw score<=100M **per block**.
All new-node thresholds below are offline diagnostics; no new admission mode or limit was
implemented. Runtime remains Disabled/Shadow, with ordinary Unzen behavior unchanged.
Laboratory `evm-gas-only` retains the old score while lifting only its admission cap.

Only single-block proposals within30M charged EVM gas enter block-cap actual maxima.
Actual PGU includes the complete proposal and startup. No multi-block actual is divided
among blocks or transactions. The admitted workload objective differs from symmetric
average-error measures such as WAPE.

## Budget frontier

These are maximum **observed** PGU over the same73 eligible samples, not upper bounds.
The historical column tests existing block quotes; it does not simulate transaction
repacking, certify normal transaction rejection rates or infer historical per-block PGU.

| Score rule | Eligible samples passing | Maximum actual PGU passing | Historical block quotes passing |
| --- | ---: | ---: | ---: |
| Unzen raw100M | 64/73 | 5.952587B | 9600/9600 |
| Node 0.100000000B PGU | 37/73 | 0.184911B | 9537/9600 |
| Node 0.250000000B PGU | 51/73 | 0.295068B | 9583/9600 |
| Node 0.500000000B PGU | 54/73 | 0.546425B | 9584/9600 |
| Node 1.000000000B PGU | 58/73 | 0.755431B | 9584/9600 |
| Node 2.000000000B PGU | 59/73 | 1.375197B | 9585/9600 |
| Node 2.266932279B PGU | 59/73 | 1.375197B | 9600/9600 |
| Node 3.600000000B PGU | 61/73 | 2.246391B | 9600/9600 |
| Node 10.000000000B PGU | 64/73 | 4.700585B | 9600/9600 |

The2,266,932,279-PGU threshold is derived as the maximum new quote among this
historical panel's old-admitted blocks. Its100% coverage is descriptive by construction,
not independent validation or a production recommendation. The3.6B reference was discussed
before this experiment; neither threshold is enabled. A per-block cap also does not bound
an entire multi-block proposal or establish a proving deadline/shared proposal budget.

At the offline3.6B reference there are53 samples passing both conditions,11 passing
only Unzen,8 passing only the new condition, and1 passing neither.12 multi-block actuals
remain unallocated and4 native-only inputs remain unmeasured. Sample counts reflect this
selected panel, not prevalence or transaction throughput.

## High-workload cases inside old Unzen100M

Every row below is a single-block proposal passing the raw Unzen100M condition. These
rows show workload that the old cap permits, not a claim that Unzen violates its original
GPU/proving-time policy. Candidate quotes are diagnostics, not actual enabled rejections.

| Case | Charged EVM gas | Raw Unzen score | Actual PGU | New node PGU | Full quote PGU |
| --- | ---: | ---: | ---: | ---: | ---: |
| `aligned-k7680/aligned-k7680` | 14,782,761 | 1,874,728 | 5,773,513,175 | 11,465,883,509 | 11,620,825,340 |
| `account-size/successor` | 10,055,942 | 33,676,971 | 5,073,040,910 | 20,250,154,959 | 20,405,394,006 |
| `old-account/long-r1400` | 11,166,706 | 49,071,635 | 5,952,586,609 | 15,455,283,710 | 15,610,374,020 |
| `cache-state-warm-successor` | 29,669,999 | 28,412,590 | 5,027,210,167 | 17,441,792,010 | 17,606,198,729 |
| `ecmul-mixed-r1` | 416,425 | 15,224,853 | 4,700,584,886 | 5,571,271,231 | 5,726,213,574 |

The new quote covers actual in all five. They motivate charging container populations,
key concentration, cache lifetime and precompile operands. Conservative reserves are an
intentional capacity tradeoff; their average-error penalty does not erase this benefit.

## New measurements near the old cap

Whole-proposal actuals below are newly measured, not scaled from near-full gas results.
Signed envelopes remain the existing maximal references, while runtime repetitions,
operands/input length, counter guards and exact receipt gas are independently qualified.

| Case | EVM gas | Raw Unzen score | Node PGU | Full quote PGU | Actual PGU |
| --- | ---: | ---: | ---: | ---: | ---: |
| `division-density-sdiv-limb4-n1298` | 2,512,407 | 79,970,176 | 219,857,839 | 374,850,926 | 280,637,474 |
| `division-density-sdiv-limb4-n1631` | 3,103,149 | 99,941,518 | 274,240,475 | 429,233,562 | 311,249,438 |
| `modular-density-mulmod-limb4-n599` | 1,732,485 | 79,989,816 | 166,448,520 | 321,441,606 | 267,857,195 |
| `modular-density-mulmod-limb4-n752` | 2,121,411 | 99,878,286 | 206,945,130 | 361,938,216 | 295,068,323 |
| `sha256-bulk-a5-l4096-n394` | 1,678,631 | 79,857,156 | 15,796,350,703 | 15,951,344,302 | 208,498,270 |
| `sha256-bulk-a5-l4096-n496` | 2,058,479 | 99,974,208 | 19,883,722,618 | 20,038,716,217 | 220,715,271 |
| `sha256-bulk-a5-l64-n466` | 535,853 | 79,937,610 | 18,681,520,608 | 18,836,514,207 | 173,887,970 |
| `sha256-bulk-a5-l64-n586` | 619,853 | 99,975,930 | 23,490,193,449 | 23,645,187,048 | 177,134,961 |

MULMOD/SDIV actuals are about0.268..0.311B PGU; these finite old-cap points do not
demonstrate an expensive Unzen-admitted arithmetic workload. Their node shortfalls
37..101M remain visible and their full quotes cover actual. This does not certify all
operands, mixtures or other repetition kernels.

The four repeated small-SHA cases cost about0.174..0.221B actual PGU but quote15.8..23.5B
node PGU. This is a substantial constructed overquote and would fail the illustrative
new3.6B condition while passing old Unzen100M. It is retained as a price/compatibility TODO,
not repriced from this comparison. No historical normal transaction rejection is
established by these synthetic recipes; all9,600 retained historical blocks pass3.6B.

## Remaining shortfalls and tradeoffs

Across all85 measured inputs, nine retained pressure node shortfalls and three retained
full-proposal shortfalls remain; ordinary and new small-input node gaps remain too.
The maximum node/full shortfalls are unchanged at1,597,968,115/1,261,582,685PGU in
multi-block storage-prefix proposals. They are not block actuals and cannot enter the
single-block frontier. The maximum positive node shortfall among either admitted
single-block set is153,438,578PGU; startup is not subtracted from the denominator.

New3.6B also admits eight samples rejected by old100M, including dense MULMOD/SDIV,
EXP, SHA256 bulk, MCOPY, memory loads and two STATICCALL repetition points. All eight
measured actuals fit3.6B, but this is a finite quote-cap comparison rather than implemented
admission. Conversely, six of the11 Unzen-only samples have actual below3.6B: the four
small SHA cases and two constructed access-list arrangements. Their overquote remains
visible alongside the five intended expensive exclusions.

## Reproduction and verification

The [published Atlas fixture](https://github.com/taikoxyz/raiko2-atlas/tree/fixture-unzen-cap-frontier-v1/fixtures/benchmarks/unzen-cap-frontier-v1)
retains original data and portable manifests; its [case CSV](https://github.com/taikoxyz/raiko2-atlas/blob/fixture-unzen-cap-frontier-v1/fixtures/benchmarks/unzen-cap-frontier-v1/cases.csv)
contains all89 rows and both offline cap classifications.

The reusable [cap-frontier CLI](../../bin/zkgas/bench/cap_frontier_report.py) accepts explicit
hash-bound input/native/quote/actual manifests. Its [usage](../../bin/zkgas/bench/README.md#offline-cap-frontier-reports)
retains original inputs and reports, validates program/model/public joins and per-block
rounding, and separates quote-only natural sensitivity. The modular/division/SHA producers
now support bounded explicit repetitions without changing any of their84 original default
rows. Construction, native qualification, quote and external observation remain separate.

Independent pretruth verification covered12 bytecode intents,36 typed transaction
signatures,24 user code/hash bindings, receipts/routes, frozen sources and all four real
100M boundary neighbors. Independent final verification reconstructed all85 raw rows,
normalization, frontiers, four-way categories, natural50/9600 hashes and observer parity.
Eight resource outcomes passed owned-resource cleanup. No new native or SP1 executions
were performed by verification agents.

Validation:32 focused producer tests (four optional native skips),12 final report tests,
22 combined report tests (one opt-in skip),49 existing harness tests and the full545-test
bench run (65 optional skips) passed. The final two report-test additions also passed in
the12-test focused run. Independent source reviews passed for both producer/report changes.
No guest,SDK,ELF,observer or Rust workspace build was needed.

The source model and pending PR policy remain unchanged. This supports a finite
capacity-control advantage and exposes explicit overquotes; it is not a universal PGU
bound, proof-time guarantee, untouched validation result or deployment/merge decision.
