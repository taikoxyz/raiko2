# Patched-Unzen workload-shape checkpoint

On this opened cohort, a simple scalar-aware ECMUL reserve added to Unzen has slightly
lower normalized prediction error than the current complete model. This does not establish
that either model safely bounds workload at an admission limit, or that the other model's
additional protection components can be removed.

## Population and method

The original Hoodi selection contains 18 training proposals, 55 evaluation proposals and
three separate boundaries. All 76 fresh native quotes completed and retain the predecessor's
numerical801 node/full/Unzen values and public outputs. Actuals are reused execute-only
observations from research b76 / SP1 SDK6.3; no new SP1 run or guest build was needed.

Both baseline profiles and the input selection were sealed before calculating these results.
The original linear access-list profile is retained alongside a pair-aware alternative
whose linear key reserve is explicitly zero. Neither is selected using the resulting error.
The scalar coefficient572936 is an identified calibration prior from numerical801, not a
coefficient fitted on these 55 observations. Nevertheless, this is an already opened
development/regression cohort, not untouched validation.

Unzen scores retain their own unit. Each predictor receives one multiplier determined only
from the 18 training proposals before computing evaluation WAPE. The patched baseline starts
with `k*Unzen`, where `k=33858680912/8514368607`, adds `572936*W` for successful ECMUL calls,
and rounds block scores upward before summing. `W` is the observed sum of reduced-scalar
bit length plus popcount, independent of selected quote prices. This is a PGU-equivalent
counterfactual, not an identified marginal PGU conversion or production tariff.

## Results on the same 55 proposals

| Predictor | Training-scaled WAPE | Spearman |
| --- | ---: | ---: |
| Original Unzen | 6.283565% | 0.512799 |
| Unzen plus scalar-aware ECMUL reserve | 2.468748% | 0.973338 |
| Above plus container reserves | 2.489145% | 0.972183 |
| Current model, literal node score | 2.793784% | 0.959596 |
| Current model, full proposal score | 2.683576% | 0.965152 |
| Constant training-mean predictor | 6.063455% | Undefined: no rank variance |

Access-list linear/pair stages do not change the scores on this cohort, in either profile.
These proposals therefore provide no discriminatory evidence for those repairs. The small
increase after container reserves is a shape result, not evidence that the reserves are
unnecessary for concentrated-key inputs.

Native PGU accounting remains separate: the current node sum is1.074392x actual and full
is2.102738x, with zero positive gaps on either surface in these 55 proposals. No raw-Unzen
PGU ratio or raw-Unzen PGU-underquote count is reported. A fitted shape multiplier must not
be substituted for the native safety or availability checks.

## Interpretation and unresolved capacity evidence

This result does not support the claim that replacing all Unzen pricing with the complete
model is necessary for average accuracy on this cohort. A few targeted repairs remain a
serious candidate. Choosing between them requires comparing observed tail deficits and
historical/application compatibility, as well as prediction error and implementation cost.
More routes or coefficients alone do not establish an advantage.

The wider322-input native batch is incomplete. Three concentrated-storage inputs reached
the1200-second native quote limit: write-prefix24, its budget neighbor and the transition
prefix case. They are recorded failures, not pricing passes or SP1 failures. This batch's
host executable used unoptimized dev compilation; the previous successful producer used
dev opt-level1. That configuration difference is established; its complete causal effect
has not been verified at the same source. No new price is derived from the timeouts.

Consequently the patched-Unzen single-block capacity frontier and historical packing
comparison remain unresolved. The first76 shape report is deliberately separate from the
incomplete capacity batch, and every input uses the quote/actual-only manifest path without
inventing constructor receipts or per-block actual PGU.

## Reproduction and verification

The retained report bundle contains `SELECTION.json`, `MANIFEST.json`, both profiles,
`COMMANDS.json`, `REPORT-linear.json`, `REPORT-pair-aware.json`, `HEADLINE-CHECK.json` and
`SUMMARY.json`. SUMMARY SHA-256 is
`ec94871eba9eb943614b0738dbe2d8ed49ca7b6227e439429fc2942b272dc238`.
Raw artifacts retain their existing hashes; this checkpoint does not claim that the new
bundle has already been published to Atlas.

Run either profile through the checked-in interface:

```sh
~/.venv/bin/python bin/zkgas/bench/targeted_unzen.py \
  --manifest "$MANIFEST" --profile "$PROFILE" \
  --max-source-bytes 1073741824 --output "$REPORT"
```

The strict tool checks canonical input/report/header/public/program identities and resource
bounds. Independent direct arithmetic over the 76 quotes and actuals reproduces every
reported WAPE and training scale exactly; all14 displayed stage/common rows match. The
original Unzen/node/full/constant headline and ranks also match the retained earlier metrics.
No price, admission, runtime activation, SDK, ELF or deployment changes are made here.
