# Block-aware arithmetic and copy cost refinement

The user authorized fixing block-visible pricing deficits while preserving the empirical
mainnet proposal conclusion (roughly 2x conservative quotes, rare underquotes). Stop for
discussion if a candidate materially changes that conclusion. Shared proposal budgeting
and guest/backend upgrades are outside this change.

## Selected approach

Use data-derived conservative prices for the dimensions already visible during execution.
Add per-normal-event structural floors to DIV, SDIV, MOD, SMOD and ADDMOD. Their fitted
accepted models and feature-error routes remain intact, composed by the existing maximum.
Raise only MCOPY's accepted `copy_words` parameter through an authenticated generator;
retain additive dispatch/constant/memory-growth charges. This covers observed direction and
relative-alignment variability without introducing new input selectors or operand classes.
An operand/alignment-sensitive model would add instrumentation and calibration complexity;
independent global discounts would also alter unrelated paths. Those approaches are not
needed for the observed block-local deficits.

Fit arithmetic with exact fractions: parent event price plus complete-proposal deficit per
observed target event; take at least the parent price, multiply the maximum by 5/4, and round
up to a 100-PGU grid. For MCOPY, apply the same deficit calculation per copied 32-byte word,
with the existing word parameter as the minimum, 5/4 headroom and an 8-PGU grid. Zero-word
rows validate controls without division. Fit cases are validated retained observations; exclude
reserved holdouts and deduplicate canonical inputs per route. Whole-proposal quotes isolate
non-operation costs during calibration; node subtotal remains the admission-facing metric.

Use an immutable authenticated model0a3 parent snapshot and compact hash-bound evidence.
The current deterministic successor generator is the single producer of the runtime JSON,
generated Rust and accepted hash. Retain the prior MULMOD 1600-PGU floor, payload ownership,
Unzen/default modes, feature-error rules, resource/MPT/startup/blob tariffs and all unrelated
operation routes. No hand-edited generated coefficients or historical artifact rewrite.

## Validation and decision gate

Re-quote retained boundary inputs and two fixed 90-proposal/17,280-block mainnet cohorts (180 proposals/34,560 blocks)
with both model0a3 and the candidate. Verify canonical input, public output, gas/Unzen scores,
non-price selectors and component ownership remain unchanged. Report per-proposal and
per-block price deltas, complete quote/actual distributions, weighted ratios, and underquote
counts, keeping external actuals in their own SP1 identities. Existing profile/ELF mismatch
limits remain explicit; do not retag historical calibration identities or treat Unzen scores
as PGU.

As an internal triage rule, a >5% change in median or workload-weighted mainnet quote, or
>20% price uplift on an individual historical proposal, requires discussion before publication.
Also stop on a qualitatively different underquote/overquote distribution or normal transaction
compatibility regression. These are review triggers, not production acceptance limits.

Seal the fit before evaluating fresh prospective arithmetic/copy checks; keep all previously
reserved cases out of fitting. Add generator scope/provenance/rounding failures and runtime
price, zero length, memory growth, feature-error and aggregation regressions. Independent
adversarial review and independent behavioral verification precede pushing #260 and updating
its exact revision in #284. Preserve both Draft/do-not-merge; no activation or deployment.
Use coherent native caches and bounded checks; no guest or dependency rebuild solely for
re-quoting measurements. Record checks that cannot finish under the disk guard.
