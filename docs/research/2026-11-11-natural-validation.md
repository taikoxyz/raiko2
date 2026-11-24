# Frozen-model natural proposal validation

The frozen `80160649…` model was validated against ten recent Mainnet and ten recent
Hoodi proposals without changing prices. Together with the
[single-block capacity validation](2026-11-11-frozen-model-capacity.md), these results
support the measured capacity-control benefit while preserving important precision
and data-diversity limits. They do not establish universal superiority over Unzen.

## Selection and actuals

Selection used fresh live data on November 11. Mainnet contains proposals 48283–48292:
the ten newest eligible distinct GuestInput URIs within a freshly captured first
100-row production API page. Global completeness across unseen pages is not claimed.
Hoodi contains the ten completed proposal IDs immediately before the captured chain
head, 89873–89882. No failed input is replaced and no selection uses PGU or cycles.

The request/proposal selection and historical comparison rule were sealed before
the selected actuals were opened. Canonical inputs, the genuine current-model quotes,
all 5,738 headers and their hashes, full proposal boundaries, public commitments and
per-block Unzen difficulty were checked. Every selected input is distinct from the
previous 192-input development/regression panel, and its first block postdates the
candidate source commit. The set contains 1,920 Mainnet and 3,818 Hoodi blocks.

Ten Mainnet actuals were newly executed once. Ten Hoodi actuals were reused from the
unchanged collector after exact input/program/public-output qualification. Nine of
those collector actuals predate selection; the tenth, proposal 89882, was collected
after selection. This is not a claim that all twenty observations were future blind
samples. There were no failed or retried actual executions.

Actual is whole-proposal execute-only PGU from research ELF `b76baf2e…` and
SDK/executor/runner 6.3.0, exactly `floor(raw ExecutionReport.gas * 10 / 191)`.
The source requests' production PGUs are not used as this research-program truth.
No guest, observer, SDK or tariff was rebuilt or changed, and no production-program
or GPU proving-time equivalence is claimed.

## Literal PGU observations

Node is `sum(ceil(block_known_scaled / 10^18))`; full is the separate complete
proposal quote. Neither subtotal is rescaled in this table.

| Metric | Mainnet, 10 proposals | Hoodi, 10 proposals |
| --- | ---: | ---: |
| Actual sum, PGU | 10,824,275,772 | 17,608,922,100 |
| Node sum, PGU | 10,304,816,283 | 17,946,156,107 |
| Full quote sum, PGU | 23,734,210,529 | 36,813,060,934 |
| Node sum / actual sum | 0.952010x | 1.019151x |
| Full sum / actual sum | 2.192683x | 2.090591x |
| Node underquotes | 6/10 | 0/10 |
| Largest positive node gap, PGU | 178,245,194 | 0 |
| Full underquotes | 0/10 | 0/10 |

The Mainnet node deficits remain findings. Full-quote coverage does not erase them,
and a proposal's actual PGU cannot identify an individual block's actual workload.
These results preserve the approximately 2.1–2.2x full-quote overestimate rather than
claiming the model has native unit accuracy near one on both surfaces.

All 5,738 existing block quotes pass the separate raw Unzen 100M condition and the
illustrative new-node 3.6B PGU reference. The maximum node block quote is 137,491,063
PGU. This is existing-packing sensitivity, not a transaction repacking simulation,
transaction refusal rate or measurement of per-block actual PGU.

## Unit-correct comparison with Unzen

Raw Unzen is a distinct score. Its raw value is never treated as PGU or assigned a
literal PGU-underquote label. To compare workload shape, use the already frozen
historical training sets separately by chain: Mainnet proposals 44900/45309 and
Hoodi's original eighteen training proposals. For each of node, full and Unzen,
fix `k = sum(training actual) / sum(training score)`, with no intercept; a constant
baseline is the historical mean actual. No new observation changes any coefficient.
These diagnostic scalars are not admission tariffs or a suggested limit adjustment.

| Training-scaled WAPE | Mainnet | Hoodi |
| --- | ---: | ---: |
| Node | 20.830451% | 5.274391% |
| Full | 19.527709% | 0.565166% |
| Unzen | 75.864035% | 0.734703% |
| Constant historical mean | 98.100568% | 6.822997% |

| Raw-score Spearman against actual | Mainnet | Hoodi |
| --- | ---: | ---: |
| Node | 0.721212 | 0.951515 |
| Full | 0.951515 | 0.830303 |
| Unzen | 0.734694 | 0.950992 |

Unzen remains competitive here: it slightly outranks node on Mainnet and has lower
training-scaled error than node on Hoodi. Full has the lowest normalized error on
Hoodi, but the difference from Unzen is small in absolute percentage points.
Mainnet's much lower new-model scaled error must be interpreted with its two-input
historical training set and transfer bias; it is not evidence of a universal
multiplicative improvement in physical accuracy.

Traffic diversity is limited. The sets contain only 1,934 Mainnet transactions in
1,920 blocks and 3,892 Hoodi transactions in 3,818 blocks. Within-chain actual
max/min ratios are only 1.012021 and 1.017239. In contrast, Mainnet node quotes span
1.289001x and full quotes span 1.128314x. Thus the new quotes still move more than
actual workload in this quiet Mainnet cohort. Correlations over ten nearly constant
observations cannot establish robust identification across applications. Broader
application traffic remains a precision/availability follow-up; it must not be
replaced by repeatedly adding quiet proposals.

## Evidence and remaining work

The frozen selection, bounded-page clarification, input/header seal, diagnostic
protocol, raw actuals, current quotes and exact per-block data are retained. Their
entrypoints are `NATURAL-SELECTION.json`, `NATURAL-SELECTION-QUALIFICATION.json`,
`NATURAL-INPUTS-SEAL.json`, `NATURAL-DIAGNOSTIC-PROTOCOL.json`,
`NATURAL-ACTUALS.json` and `NATURAL-METRICS.json`. Existing Raiko quote/compare/report
tools remain the execution interfaces; Atlas owns fixture data and integrity only.

Independent checks cover selection replay, canonical input and public-header joins,
program/unit/public-output compatibility, resource receipts and numerical metrics.
New actuals used one sampler under the existing shared worker lock, the 6GiB sampled
process-tree PSS guard and exact receipt-owned shared-memory cleanup. The collector
and its timer remain unchanged. No runtime mode, admission limit, deployment or
pricing implementation changes follow from this validation.

The measured single-block capacity frontier remains stronger than the old cap on
the selected high-workload cases. Open limitations remain the quiet natural cohort,
the Mainnet node deficits, full-quote conservatism, constructed KZG/BLS overquotes,
the retained multi-block residuals and exact production-target/fork validation.
The later EVM-gas-only saturation TODO remains deferred.
