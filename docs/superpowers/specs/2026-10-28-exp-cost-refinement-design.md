# EXP per-event pricing refinement

Continue block-aware workload evaluation before increasing workload complexity.
Rank actual SP1 PGU divided by new node zkGas. Full proposal quotes isolate the
shared proving overhead during calibration; they do not replace the node metric.
Unzen values remain in their own cost-model units.

The frozen proposal ELF executes EXP with an exponent-dependent loop, inline
integer arithmetic and byte-wise zero comparisons. Byte length alone hides
exponent shape. New dense-width and amplified samples reproduce a scalable
deficit: at sixteen blocks, the L4/L5 cases cost about 1.303B/1.545B actual PGU
against about 0.961B/1.179B complete quotes under parent model80459. These are
local SDK6.3.0 measurements, not a claim of current production SDK parity.

Use the existing typed exponent byte length L, bounded to 0..32. Preserve the
original accepted curve and charge each normal executed EXP:

`max(original_EXP_event_price(L), K * (10 + 50 * L))`.

Apply the maximum before accumulation, retaining accepted-only route composition.
A maximum over aggregate route candidates could dilute the winning candidate when
different lengths occur together. This change adds no new instrumentation or raw
gas selector, preserves the original dispatch/curve coefficients, and leaves
feature-error pricing and all non-EXP routes unchanged.

Authenticate immutable parent model80459 and a sealed manifest of 29 native/SP1
observations. Only the 21 observations with complete-price deficits constrain the
new floor. Solve the complete deficit per executed EXP, normalize by `10+50L`,
take the maximum, multiply by 5/4, and round upward to an integer. The other eight
observations validate the preserved original curve and do not force an unnecessary
new floor. This yields K=24; all 29 projected complete quotes cover their actuals.
The successor and table generators produce the model and accepted content hash.

Validate six fresh shape/mixed-length cases excluded from fitting, the retained
simple boundaries, and both fixed 90-proposal mainnet cohorts against model80459.
Preserve inputs, actual observations, Unzen scores, selectors and component
ownership. Discuss a material historical shift using the existing refinement
thresholds before publishing. Independently review source and verify behavior.

Next finite coverage explores substantial work before caught failure, fresh child
memory and returndata, then state interactions. Successful isolated and mixed LOG,
BN254 MUL and SHA256 controls test candidate composition; arithmetic dilution by
itself is not evidence of actual underpricing. None of these finite tests proves
coverage of every EVM input or introduces a production admission limit.
