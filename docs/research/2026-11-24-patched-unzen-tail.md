# Patched-Unzen pressure tails and historical packing

The frozen comparison is complete. Adding only scalar-aware ECMUL does not remove the
observed high-workload tails admitted by Unzen. Access-list and container accounting are
also needed on this input set. Once those reserves are added, the patched baseline and the
current node model both exclude every observed single-block actual above the illustrative
3.6B PGU reference, while retaining all 9,600 historical blocks at their respective stated
3.6B score thresholds. This supports retaining the protection mechanisms; it does not prove
that replacing all Unzen operation prices is necessary.

The models have different availability errors. At that reference they admit the same number
of capacity inputs, but each admits three constructed inputs rejected by the other. A second,
explicitly retrospective comparison preserving the same historical blocks also finds the
complete patched baseline competitive with the current model. Neither comparison selects
production limits or establishes a global workload bound.

## Frozen scope and completed execution

- Original selection: 322 canonical inputs, unchanged from the November 12 seal
  `1d2fe6f32c237cdc2c150aa69c2cc0be8f324e0b13af8dc8999badc4a5657f25`.
- Main reports: 270 existing whole-proposal actuals and 50 quote-only historical proposals.
  The actuals comprise 172 receipt-qualified controlled inputs and 98 actual-only proposals.
  Two access-list development calibration controls remain separate from the main comparison.
- Capacity: exactly 160 receipt-qualified single-block proposals, each within 30M charged
  gas. The other 12 receipt-qualified proposals have two blocks; their actuals remain whole
  and do not enter this capacity table. Header gas does not grant receipt qualification to
  the 98 actual-only proposals.
- History: the same 50 Mainnet proposals / 9,600 blocks, with chain and proof-carry chain
  identities both 167000 and Unzen fork segments. Every block fits original raw Unzen
  100M. These are existing-packing observations, not a representative application sample,
  transaction repacking experiment or individual transaction rejection rate.
- Numeric model remains `80160649d7d29a85879cd9524b35c271a4b8638f64de94eef91d016e56121cf8`.
  Actuals retain research ELF `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`,
  SP1 SDK/executor/runner 6.3.0, gas-estimator execution and `floor(raw gas * 10 / 191)`.
  No actual, guest, SDK, operation price or deployment was changed.
- Both original baseline profiles and the original 18-training / 55-opened-evaluation
  partition remain fixed. The [earlier shape result](2026-11-12-patched-unzen-shape.md) is
  unchanged. All these observations are opened research data, not new holdout evidence.

The original 89 successful / three timeout / one interrupted / 229 unstarted record is
preserved. An optimized host-only quote executable was built from the unchanged Rust source
and lockfile. Three overlap controls retain the complete semantic quote body; all 270
available earlier quote bodies retain every field other than the added semantic observation
object. Public outputs agree. The prior three timeout inputs complete in 114.294, 171.945
and 114.646 seconds. The remaining 230 inputs complete without failure; 89 original quotes
are reused and 233 continuation quotes selected. Three additional overlap runs qualify the
new executable. No constructor or SP1 execution is involved.

Host build: 213.384 seconds, peak process-tree PSS 931,540,992 bytes. Qualification and
continuation peaks are 54,847,488 and 84,688,896 bytes. The sequential replay guard retains
6GiB PSS, 1200 seconds per input, 64MiB output, 8MiB logs and a 2GiB free-disk floor.
These are host experiment measurements, not prover memory or throughput results.

## Two budgets, kept separate

Raw Unzen retains its score unit and original 100M reference. The unchanged training-only
shape conversion is:

```text
k = 33858680912 / 8514368607
patched score = k * raw Unzen + explicit additive reserves
inherited exact budget = k * 100000000 = approximately 397665199.556
```

The conversion is not an independently calibrated marginal PGU exchange rate. Patched
scores are PGU-equivalent counterfactuals. The separate illustrative 3.6B score threshold is
about 9.05 times the inherited converted budget; a difference between those thresholds is
not an improvement in metering. Exact inherited comparisons precede score ceiling; the
additional conservative integer-rounding sensitivity is retained in the full reports.

Each stage below has all 160 capacity inputs available. No failure, missing selector or
change of comparison population explains a reserve's effect.

| Baseline stage | Admitted at inherited budget | Of these, actual >3.6B | Admitted at 3.6B score | Of these, actual >3.6B |
| --- | ---: | ---: | ---: | ---: |
| Calibrated Unzen, no reserve | 149 | 5 | 156 | 5 |
| + scalar-aware ECMUL | 142 | 4 | 155 | 4 |
| Linear profile: + raw access-list items/keys | 139 | 3 | 153 | 3 |
| Linear profile: + access-list pairs | 138 | 3 | 152 | 3 |
| Linear profile: + container reserves | 133 | 0 | 149 | 0 |
| Pair-aware profile: + items, explicit zero key coefficient | 142 | 4 | 155 | 4 |
| Pair-aware profile: + access-list pairs | 139 | 3 | 152 | 3 |
| Pair-aware profile: + container reserves | 134 | 0 | 149 | 0 |

The two profiles were declared before these results. The linear profile retains its explicit
raw-key reserve before adding pairs; the pair-aware alternative explicitly replaces that
key coefficient with zero. Both remain reported; neither is selected from the outcome.

At the inherited budget, scalar-aware ECMUL and every later stage retain 9,584 / 9,600
historical blocks. At the separate 3.6B score threshold, every stage retains 9,600 / 9,600.
The tighter inherited protection therefore has a visible historical-packing cost.

| Stated operating point | Admitted /160 | Maximum admitted actual PGU | Admitted actual >3.6B | Historical blocks retained |
| --- | ---: | ---: | ---: | ---: |
| Raw Unzen score <=100M | 149 | 5,952,586,609 | 5 | 9,600 |
| Complete patched baseline <=3.6B, either profile | 149 | 2,101,911,525 | 0 | 9,600 |
| Current literal node quote <=3.6B | 149 | 2,246,391,418 | 0 | 9,600 |

These are finite admitted-workload observations. In particular, comparing admitted actual
PGU is meaningful; treating raw Unzen scores as actual PGU or counting raw-score PGU
underquotes is not.

## Which protection removes each old-admitted tail

The five rows below are all the actual-above-3.6B inputs admitted by original raw Unzen
100M in the qualified single-block set. All whole-proposal actuals and gas figures are
retained observations, not allocations from a multi-block result.

| Retained case | Charged block gas | Raw Unzen score | Actual PGU | First effective additional protection in this ladder |
| --- | ---: | ---: | ---: | --- |
| ECMUL mixed R1 | 416,425 | 15,224,853 | 4,700,584,886 | Scalar work |
| Aligned access-list K7680 | 14,782,761 | 1,874,728 | 5,773,513,175 | Linear keys or pair-aware construction |
| Account-size successor | 10,055,942 | 33,676,971 | 5,073,040,910 | Container work |
| Long-account R1400 | 11,166,706 | 49,071,635 | 5,952,586,609 | Container work |
| Warm block-cache successor | 29,669,999 | 28,412,590 | 5,027,210,167 | Container work |

Thus the conclusion is not that ECMUL alone makes Unzen equivalent to the current model.
The complete counterfactual reuses scalar, access-list and stateful container selectors and
tariffs established by this research. Its container stage adds storage-journal, account-
journal, account-cache and storage-transition reserves. It does not implement the reviewer's
alternative guest-container optimization, and it is not independent calibration of those
shared priors. The experiment establishes their effect on the frozen inputs, not the
minimum possible implementation or a universal set of necessary formulas.

## Equal admission counts conceal availability differences

At the separate 3.6B reference the complete patched baseline and current node model admit
149 inputs each, but their sets differ by three inputs each way. Both baseline profiles
have the same differences here.

| Constructed input admitted by only one model | Actual whole-proposal PGU | Admitting model |
| --- | ---: | --- |
| Valid KZG, one call | 192,670,356 | Patched baseline |
| Valid KZG, eight calls | 406,546,722 | Patched baseline |
| BLS pairing K2, eight calls | 344,850,094 | Patched baseline |
| Full-width MULMOD, N11719 | 2,246,391,418 | Current node model |
| Full-width SDIV, N16792 | 1,705,946,106 | Current node model |
| Empty STATICCALL, N16384 | 282,251,534 | Current node model |

The current model's conservative KZG/BLS prices remain an availability limitation. Conversely,
the inherited Unzen operation weights still exclude the listed arithmetic/call workloads
in the patched baseline. These are valid constructed boundary inputs, not observations of
historical users being rejected. Keeping original history does not settle application-wide
availability, and this result does not justify blindly lowering the corresponding prices.

## Retrospective point preserving exactly the same history

For each score independently, take the smallest integer threshold that retains the same
nonempty set of historical blocks admitted by raw Unzen 100M, then apply it to the fixed
160 capacity inputs. Here that historical set is all 9,600 blocks. Raw Unzen receives the
same threshold adjustment as every other score. All selectors are present; no model gets
a smaller historical set. This is an opened-data operating-point diagnostic, not deployment
parameter selection or independent validation.

| Score | Smallest history-retaining threshold, in its own unit | Admitted /160 | Maximum admitted actual PGU | Actual >3.6B |
| --- | ---: | ---: | ---: | ---: |
| Raw Unzen | 29,566,495 score | 125 | 5,773,513,175 | 3 |
| Complete patched baseline, either profile | 1,436,433,094 PGU-equivalent | 143 | 1,145,206,963 | 0 |
| Current node model | 1,590,862,383 PGU | 141 | 1,145,206,963 | 0 |

Block 13,217,521 determines both complete-patch and current-node thresholds. The full facts
retain every determining block and every admitted/excluded canonical input. The patched
baseline is competitive at this descriptive point too. The data do not establish a unique
winner or certify any of these thresholds outside the selected history and pressure set.

## Residuals and limits

- Across 160 single-block actuals, node has 118 positive gaps, maximum 153,438,578 PGU;
  full proposal has none. Absolute protection at a chosen reference does not erase these
  literal pricing gaps or prove their causal origin.
- Across all 270 main actuals, node has 127 positive gaps, maximum 1,597,968,115 PGU;
  full proposal has three, maximum 1,261,582,685 PGU. The larger residuals remain attached
  to their whole multi-block inputs and are not converted into single-block gas claims.
- The same guards can be combined with a simpler operation-price baseline. This phase
  supports testing that simplification, while preserving demonstrated scalar, transaction-
  preparation and container protection. It does not support removing them based on the
  earlier mean-error table.
- Production ELF/SDK parity, active application traffic, untouched validation, GPU proving
  time, collector execution cost and activation policy remain separate work. Etna FI
  singleton trials are outside this Shasta dataset and remain pending their own execution
  and proof-acceptance stage.

## Verification and reproduction

The existing `targeted_unzen.py` CLI generated both reports from hash-bound canonical input,
native/actual/quote/header graphs. Its 36 focused tests pass. Independent direct integer/
Fraction verification checks all 322 inputs, 272 original actuals, 174 native receipts,
270 old quote-body/public parities, every staged price and ceiling, all admitted input sets,
exact and conservative-rounded inherited comparisons, all 9,600 historical decisions,
original 18/55 scales/WAPE/ranks, and the retrospective threshold identities.

The portable Atlas overlay and its three immutable source dependencies carry the complete
standard manifests, original/derived provenance, both profiles and reports. Transport/path
adaptations are explicit; numeric and public/program identities remain unchanged. Tools
remain in Raiko, fixtures in Atlas. The fixture's commands reproduce the reports using
relative paths; no local experiment directory is required.

- Overlay: [fixture-unzen-patched-baseline-tail-v1](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-patched-baseline-tail-v1),
  with [fixed metadata and composition instructions](https://github.com/taikoxyz/raiko2-atlas/blob/8bc11933e3faff7fc0710947e12b8a48f8f1b301/fixtures/benchmarks/unzen-patched-baseline-tail-v1/README.md),
  asset `fixture-data.tar.gz`, 200,225,972 encoded bytes, SHA256
  `2a3c5ad6d911057070c00f3b076f0389d44cfccdcf7b005b2013eb022771ab72`.
  Decoded archive: 939,857,920 bytes, 728 regular nonexecutable members, SHA256
  `01eaee4acffe0e079b0a3da676561e2b025dc32c649ce228454348c65ceb3303`.
- Fixed source dependencies:
  [frozen-model validation](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-frozen-model-validation-v1),
  [access-list preexecution](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-access-list-preexecution-v1),
  [access-list short pairs](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-access-list-short-pairs-v1).
  Their exact compressed/decoded hashes and composition commands are retained in the fixture
  metadata. They supply 244 existing inputs; the overlay adds 78 exact Hoodi inputs and the
  new quote/report graph without duplicating those dependencies.
- An independent consumer verified all four compressed/decoded archives and 4,924 inner
  files, then regenerated both portable reports byte-identically using the documented CLI
  commands from a fresh composed directory (about 93MiB peak RSS). This local composed-
  consumer check is distinct from published-asset transport verification.
- The original-experiment facts and independent numerical/source-contract audit JSON are
  retained as data, separately from the sanitized portable report identities. The supplementary
  retrospective point uses the explicit max-over-history rule above; the standard CLI reports
  supply its underlying scores and fixed-budget comparisons.

No price, admission limit, guest artifact, timer, deployment or PR merge is changed by this
research. Keep the implementation PRs Draft / do-not-merge.
