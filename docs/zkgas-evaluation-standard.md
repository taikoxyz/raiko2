# zkGas Evaluation Standard

Status: **Draft, non-binding.** This document proposes a model-independent evaluation contract. It
does not become a production gate until the predicates, corpora, and promotion process described
below are reviewed and adopted. Until then, campaign-specific accepted gates remain authoritative.

This standard evaluates whether zkGas is a usable workload meter. It deliberately does not specify
the pricing formula, selectors, tariffs, or implementation. Those belong to the pricing-model
specification. A candidate evaluation covers three separate objects:

1. the **workload meter**, which observes execution and assigns a score;
2. the **backend profile**, which relates that score to one exact prover program's native workload;
3. the **capacity policy**, which selects an admission limit for an explicit proving-capacity
   assumption.

An accurate backend profile cannot repair missing workload accounting. A conservative limit cannot
repair a route whose cost decreases or disappears under composition. Mean proposal error is a
diagnostic, not a substitute for either property.

## Evidence Contract

### Evidence lanes

Keep three evidence lanes distinct:

- **Controlled boundary and adversarial evidence** tests safety through isolated workload families,
  parameter extremes, mixed interactions, saturation workloads, and fixed-score searches for the
  greatest actual proving work.
- **Natural historical evidence** tests workload recognition, compatibility, overpricing, and drift
  on an untouched, time-ordered corpus.
- **Runtime conformance evidence** tests deterministic feature collection, ownership, fail-closed
  routing, integer arithmetic, rollback, and candidate-prefix behavior.

A row may support multiple diagnostics, but any row used to select features, coefficients, tariffs,
discounts, reserves, thresholds, or limits is not untouched validation data. Corpus identities,
partitions, metrics, thresholds, and aggregation rules must be frozen before validation truth is
opened.

Before a campaign opens results, it defines eligibility and representativeness predicates,
including:

- minimum sample and time-range support;
- required fork, route, workload-family, and size buckets;
- named Anchor, system, application, and precompile-heavy cases;
- whether the corpus is exhaustive over a declared interval or is a predeclared sample; and
- the must-retain compatibility corpus, selected from protocol-valid history and named product
  requirements rather than from a candidate's score.

Every replayable eligible row in an exhaustive range remains included. Missing trustworthy truth or
an unreplayable eligible row makes the affected gate insufficient; it is not silently excluded.

Each aggregate binds its numerator, denominator, sample count, block or time range, inclusion and
exclusion rules, failed or censored rows, truth provenance, and quantile and tie conventions.
Natural-corpus rates and correlations include a declared confidence or resampling interval when the
sample supports one. Controlled and adversarial maxima are exact observed bounds, not confidence
claims.

### Controlled-workload semantic evidence

Numerical replay and source hashes do not establish that a controlled program executes its
declared workload. For operand-, branch-, size- or terminal-sensitive families, the proposed
evidence contract also records:

- the intended operands, branch and expected result or terminal condition;
- independently decoded actual operands, including order, width and signedness;
- focused native confirmation of the relevant result, success/failure and completed counts;
- fitting and validation partitions bound to the qualified program/input identities; and
- explicit limits for zero-only, symmetric, early-return and intentional failure controls.

A successful receipt, a zero result, matching opcode totals or small fit residuals can all coexist
with the wrong operands. Fit and holdout rows produced by the same defective generator do not
independently validate its semantics. Use an independent operand/result oracle appropriate to the
claimed family; this need not require a full proof or a full execution trace for every row.
Preserve rejected or mislabeled evidence and identify which claims it cannot support. Deliberate
conservative tariffs remain distinguishable from measured normal-path costs.

The [BLOCKHASH calibration erratum](research/2026-10-31-calibration-semantics.md) supplies a concrete
case. This addition remains part of the draft standard, not an adopted production gate.

### Observation surfaces

Every score, metric, predicate, and limit binds the exact observation surface and execution stage
that produces it. In the current architecture, relevant surfaces include the node's mining-visible
block-local score and the prover host's full-replay score enriched with proposal input, exact
block-end resource, MPT, payload, and blob observations. These are distinct measurements even when
they share a model identity.

For the current SP1 workload campaign, retain three quantities for each identical proposal input:
the complete proposal estimate `Q_proposal`, the node-observable mining estimate for each block
`Q_node,b` and its proposal aggregate `Q_node`, and actual proposal SP1 PGU `A` under the
named frozen program. When both estimates have the declared PGU semantics, report `A / Q_proposal`
and `A / Q_node`, their independent underpricing flags (`A > Q`), and absolute shortfalls. Keep
small-input and zero-repetition controls in both comparisons. A full proposal quote covering `A`
does not mean the mining estimate covers it. Comparing a node sum with proposal actual is an
aggregate observability diagnostic; it does not assign actual PGU to any individual block.

Preserve the producer's aggregation and rounding. The current quote API exposes
`block_local_known_pgu = ceil(sum(block_known_scaled) / scale)` alongside
`sum_of_block_known_pgu = sum(ceil(block_known_scaled / scale))`. Retain the individual block
estimates and both totals for multi-block proposals; name the denominator actually used. The
current controlled single-block cases have equal totals. Rounding does not allocate actual
proposal PGU to blocks.

The current priority is finding gaps in the block-mining-aware model. Matched increments help
distinguish fixed costs from repeatable scaling deficits, but do not replace either full ratio or
erase controls with large actual/node ratios. Adding a proposal-aware estimate and allocating its
cost back to mining remains an unresolved design option, not an assumed bridge or a correction
applied to these observations.

Prove-only offline completion cannot establish the safety of node admission. A node admission claim
must use the candidate-prefix score computable from node-observable inputs, or a separately
specified and enforced conservative bridge from the node surface to the prove surface whose
capacity containment is itself evaluated. In every threshold formula, model score `Q` and limit `L`
refer to the same named surface. Truth, score, limit, and predicate scope must not be silently joined
across surfaces.

### Units and comparability

Comparisons require the same scope, truth unit, backend and prover-program identity, fork, and
execution stage. Proposal truth compares with proposal predictions, block truth with block
predictions, and transaction admission with the corresponding candidate-prefix score. A
proposal-average PGU value is not per-block truth.

Raw scores in different units are never described as overquotes or underquotes. In particular,
**raw Unzen header difficulty must never be reported as overquoting or underquoting SP1 PGU**.
Different-unit scores may be compared using rank and threshold-sweep metrics. A single positive
scalar fitted on a frozen training partition may also provide an out-of-time workload-shape
diagnostic, but the calibrated value:

- must be labeled as a calibrated diagnostic;
- must bind the calibration artifact and training partition; and
- is not direct safety evidence and does not give the raw score PGU semantics.

## Required Scorecard

These eight areas are the minimum stable reporting surface. A campaign may add diagnostics without
changing the meaning of these metrics.

### 1. Coverage and observability

Report reachable route counts by `measured_model`, `conservative_envelope`, `tariff`, `unsupported`,
and `fail_closed` status. Also report silent-unpriced routes, missing selectors, out-of-domain and
fallback counts, feature-join and ownership failures, and coverage across opcode, precompile,
memory, call/create, state/MPT, payload, transaction, block, and proposal-once work.

The proposed hard gate is zero silent-unpriced routes. Every reachable route has an explicit price
or an observable fail-closed outcome. A high coverage percentage does not excuse a missing
repeatable or high-damage route.

### 2. Workload recognition

On untouched natural data, report:

- Pearson correlation on raw values, which is dimensionless and invariant to a positive unit-scale
  change, plus direct `log1p` Pearson only when prediction and truth share a unit;
- Spearman rank correlation and Kendall rank concordance;
- workload amplification across predeclared positive low, middle, and high truth buckets as the
  dimensionless ratio-of-ratios `(Q_high / Q_low) / (A_high / A_low)`, with positive denominators;
  and
- direct out-of-time WAPE only when prediction and truth share a unit.

For different-unit scores, direct `log1p` Pearson, WAPE, and raw `Q / A` bucket ratios are forbidden.
A campaign may report them only after applying exactly one positive scalar fitted on a frozen
training partition, and must label each result as a calibrated out-of-time workload-shape diagnostic
rather than direct error or safety evidence. Raw-value Pearson, Spearman, Kendall, and the
ratio-of-ratios amplification above remain allowed scale-invariant diagnostics. In particular, raw
Unzen difficulty versus SP1 PGU WAPE or raw `Q / A` amplification is not a valid metric.

Rank metrics test ordering. Linear and log correlation, bucket amplification, and time-split WAPE
test preservation of magnitude. A model can rank common proposals well while compressing the
expensive tail, so no single correlation or average-error metric is a promotion gate by itself.

### 3. Underpricing and capacity safety

When prediction and truth share a backend-native unit, report underquote count, minimum
`prediction / actual`, maximum `actual / prediction`, p95 and p99 **`actual / prediction`** ratios,
and worst absolute backend-native-workload shortfall. Tail quantiles do not replace the observed
controlled or adversarial maximum. A positive actual with zero prediction is a hard infinite
underpricing case, reported separately and included in the underquote count rather than silently
dropped from ratio statistics. Zero-actual rows are counted separately; they are never treated as a
passing ratio or silently mixed into ratio quantiles.

For model score `Q`, actual backend workload `A`, backend capacity `C`, and candidate score limit
`L`, report separately for natural and adversarial corpora:

```text
dangerous_admit_count(L, C) = count(A > C and Q <= L)
dangerous_admit_rate(L, C)  = dangerous_admit_count / count(A > C)
over_capacity_count(C)      = count(A > C)
admitted_count(L)           = count(Q <= L)
max_admitted_actual(L)      = max(A where Q <= L)
max_admitted_excess(L, C)   = max(0, max_admitted_actual(L) - C)
```

When `admitted_count(L) = 0`, `max_admitted_actual` and `max_admitted_excess` are not applicable;
they are never serialized as zero or treated as passing.

If a workload author also consumes an independent positive resource `E`, such as EVM gas,
calldata, blob bytes, or an execution fee, report maximum `A / E` among admitted workloads and the
minimum `E` observed or structurally required to exceed `C`. Report zero-cost cases separately and
do not hide independent attacker costs inside an undocumented scalar.

Threshold sweeping remains valid for models with different score units: sweep each model's own `L`
and compare capacity containment. The proposed gate requires zero observed dangerous admits in the
frozen controlled and adversarial domain and no direct underquote in a declared matched-unit safety
gate. The containment corpus must include at least one reachable `A > C` case and exercise the
selected threshold on both sides, unless a reviewed structural bound proves every reachable case in
the declared domain has `A <= C`. A zero denominator is not applicable, not a passing zero.

Finite zero counts cover only the declared domain. Complete routing, bounded dimensions, monotonic
pricing, and fail-closed behavior establish the structural boundary.

#### Gas-bounded priority and pricing complexity

For the current SP1 research worklist, apply the same severity dimension to every pricing family:
opcodes, precompiles, memory, witness/MPT, payload and transaction/block/proposal-level terms.
Use a30,000,000 charged-EVM-gas block budget and the positive absolute underpricing in matched
SP1 PGU units:

```text
node_shortfall(x)     = max(0, actual_SP1_PGU(x) - node_estimate_PGU(x))
proposal_shortfall(x) = max(0, actual_SP1_PGU(x) - full_proposal_estimate_PGU(x))
worst_observed_node_shortfall_30m = max(node_shortfall(x) over tested valid blocks with gas_used <= 30,000,000)
```

Keep the campaign's honest minimal-input/witness eligibility rule. If no eligible block was tested,
this maximum is not applicable rather than zero. A block already exceeding the shortfall threshold
below30M remains relevant; exact saturation is not an eligibility requirement.

For block-aware work, the node shortfall controls the priority; report the full-proposal shortfall
separately. More than2,000,000,000PGU of shortfall within this budget is high priority;
at or below2,000,000,000 is low priority in the tested domain and may remain a documented TODO.
The threshold concerns the missing workload, not total actual PGU or a ratio. High ratios remain
useful search diagnostics, including controls, without automatically requiring a complex fix.
Record the exact model, program, input/assembly, gas usage and any node admission limit/filter.
A gas-only experiment does not establish that a candidate also passes a separately selected filter.

Use receipt charged gas, including intrinsic, floor, refund and system/Anchor effects. Keep those
effects and any gross-execution-gas diagnostic explicit. Controlled single-block proposals provide
matched whole-proposal actuals for this block-budget diagnostic. Multi-block context studies retain
their actual whole-proposal results and block counts separately; do not divide proposal PGU into
invented block truth. Count proposal-once startup once. State whether a reported30M result is a
saturation measurement, an extrapolation or a structural bound. A short-sample extrapolation can
support provisional prioritization, but cannot certify the30M maximum or become a measured result.

Evaluate freely composed transactions and mixed workload families under the same total gas budget.
Independent low-priority findings do not establish a low-priority combined block. Keep baseline
shortfalls, growth shortfalls, maximum actual workload and original literal ratios visible alongside
this severity dimension. This worklist threshold changes neither admission limits nor the separate
proposal-capacity/deadline policy.

Pricing precision is traded against implementation, runtime and maintenance cost. This applies to
new fixes and to simplifying existing rules: an opcode special case, witness selector or
level-aware tariff can be replaced by a simpler candidate when gas-bounded composition tests and
normal-workload compatibility support accepting the remaining low-priority discrepancy. Record
the removed branches/selectors, price changes and observed residuals; retain old data and model
identities. Prefer an explicit small residual/TODO to complexity whose measured benefit does not
justify its cost. The present engineering focus remains block-mining-aware workload pricing;
proposal-aware allocation remains undecided. See the
[initial application and numerical evidence](research/2026-11-02-gas-bounded-pricing-priority.md).

### 4. Normal-workload availability

For the same `(L, C)` pairs, report:

```text
false_reject_count(L, C) = count(A <= C and Q > L)
false_reject_rate(L, C)  = false_reject_count / count(A <= C)
```

A zero denominator is not applicable and is always accompanied by the count. Also report score
utilization and remaining headroom at p50, p90, p95, p99, p99.9, and maximum; rejected historical
blocks and candidate prefixes; largest single-transaction share; and Anchor, system-transaction,
and precompile-heavy compatibility. Keep count-weighted and workload-weighted results separate.
When prediction and truth share a unit, report natural-workload `prediction / actual` at p50, p90,
p95, p99, and maximum. Zero-actual rows are counted and reported separately rather than included in
these ratios or treated as passing. A zero prediction with positive actual is the hard infinite
underpricing case defined above, not evidence of availability.

The proposed compatibility gate requires every item in the frozen must-retain corpus to remain
admissible at the selected limit. Whole-block replay is useful evidence, but transaction-level
availability additionally requires candidate-prefix replay with production-equivalent rollback and
replacement behavior.

### 5. Monotonicity, composition, and gaming resistance

Report monotonicity violations for every cost-bearing dimension, prefix-monotonicity violations,
ownership and double-charge failures, isolated-versus-mixed residuals, and block-local versus
proposal-once decomposition residuals. Increasing a priced dimension must not reduce the score.
Splitting, batching, reverting, nesting calls, warming state, or moving work across transaction or
block boundaries must not make equivalent proving work materially cheaper unless the backend
actually performs less work.

The proposed hard gate is zero unexplained monotonicity, ownership, or composition violations in the
declared domain. Large conservative composition residuals remain availability findings even when
they are safe.

### 6. Stability and portability

Report rolling and time-split recognition, safety, and availability metrics. Break out worst
residuals by fork, workload family, application class, route, and relevant size bucket. Report
feature-distribution drift and change points rather than relying on an aggregate daily ratio.

A fork, backend, prover program, feature schema, or cost-bearing runtime change must pass an
explicit compatibility gate or create a new profile. A stable aggregate mean cannot authorize old
coefficients when a subgroup or tail has drifted.

### 7. Backend capacity and deadline validity

For the recommended prover configuration, report end-to-end proof latency, effective
backend-native-workload throughput, and deadline misses by workload family and concurrency level.
The capacity claim explicitly states whether setup, witness transfer, proving, and aggregation are
included. Execute-only workload must not be presented as proof-deadline measurement.

Freeze the scope of the claim. A block-local policy validates only block-local containment unless a
proposal-total budget is separately enforced. Applying proposal capacity independently to every
block does not prove proposal deadline containment.

Capacity selection reports hardware and software identity, proposal interval, reserved utilization,
queue and concurrency assumptions, throughput p5/p50/p95, and a capacity sensitivity range. Peak or
vendor headline throughput is not a production limit. If equal backend-native workload has
materially different wall time across workload families, use separate profiles or recalibrate the
native cost model.

### 8. Runtime and operational fitness

Report exact-repeat determinism, estimator CPU time, peak memory, proposer critical-path latency,
typed failure counts, and version and provenance completeness. Deployment-specific resource
thresholds are frozen before production validation. Missing or malformed inputs fail closed and
remain observable; they never produce a zero or legacy charge.

## Model Selection and Limit Selection

Model selection and limit selection are separate decisions. Each applicable scorecard dimension
must have a campaign-specific pass predicate frozen before validation truth is opened. Missing
predicates or required truth yield insufficient evidence, not a pass.

Candidate selection first rejects models that fail any applicable frozen predicate, including
coverage, recognition, matched and capacity safety, availability, composition, stability, backend
deadline validity, runtime fitness, reproducibility, or must-retain compatibility. Among remaining
candidates, compare workload recognition and the complete `dangerous_admit` versus `false_reject`
frontier. Safety is lexicographically prior: minimize dangerous admission and worst admitted excess
before minimizing false rejection and conservative overpricing.

Do not select a model because its raw score is larger than another raw score or because its mean
`prediction / actual` ratio is larger. Do not select a limit by multiplying an average ratio. Select
`L` against an explicit capacity `C`, same-granularity controlled and adversarial truth, and the
frozen compatibility corpus.

## Reproducibility and Promotion Status

Every report or promotion packet binds enough identity to reproduce and audit its claims:

- fork and network range, corpus manifest, partitions, and inclusion rules;
- workload-meter implementation and feature-schema identity;
- pricing/profile artifact identity and rounding policy;
- backend, prover program, ELF/VK, launcher, and proof-mode identity;
- hardware and software identity for capacity claims;
- truth provenance, collector version, failed rows, and generated report artifacts; and
- frozen predicates and the exact results for all eight scorecard areas.

A report labels each applicable area `pass`, `fail`, or `insufficient`, and distinguishes observed
evidence from structural bounds and assumptions. A model is not promoted merely because a natural
corpus has a favorable average or no observed underquote. Promotion requires all adopted applicable
predicates to pass, no unresolved material failure, and an explicit decision binding the selected
model profile and capacity policy.

While this document remains draft, reports may implement its metrics and terminology for review,
but must not claim compliance or use the draft alone as production authorization.
