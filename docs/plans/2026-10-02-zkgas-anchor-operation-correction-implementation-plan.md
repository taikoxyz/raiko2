# Anchor Operation Ownership Correction Implementation Plan

## Status

In progress.

## Purpose

Correct the implementation that wrongly treated all Anchor execution as an opaque `block_base`
charge. The approved order is:

1. route Anchor EVM execution through ordinary operation pricing;
2. calibrate `BLOCKHASH` and close the remaining material Anchor operation gaps;
3. refit `block_base` after subtracting the corrected operation sum;
4. only then interpret any remaining block/proposal residual.

This plan does not open the final 60-row proposal corpus and does not modify the production Unzen
schedule, block limit, runtime configuration, or Boundless configuration.

## Root Cause And Evidence Status

The ownership error first entered the consolidated design in commit `dad31dbb`, was frozen into
the coverage manifest and tests in `28729c18`, and was consumed by the composite estimator in
`5f9fff4b`. The trace already identifies Anchor operations as `phase=transaction` rows joined by
`tx_index`; the estimator deliberately discarded them after joining to `is_anchor=true`.

The following evidence remains reusable:

- controlled opcode coefficients and typed opcode functions;
- stateful storage execution models;
- context-operation approximations;
- non-Anchor `tx_base` and native-transfer evidence, subject to exact replay under the corrected
  ownership declaration;
- the two integration-smoke inputs and raw traces as diagnostic inputs only.

The following evidence is invalid for promotion under the corrected decomposition:

- operation-coverage manifests whose execution selector is `non_anchor_started_transaction`;
- higher-layer and block-base fits that absorbed Anchor operation work;
- composite estimators built from those ownership and fixed-cost artifacts;
- APE or coverage conclusions produced by those estimators.

Do not rewrite or delete historical content-addressed artifacts. New artifacts must cite the
invalidated predecessor and carry new identities.

## Task 1: Correct Operation Ownership

**Primary files:**

- `experiments/opcode-gas/opcode_gas.py`
- `bin/guest-launcher/src/controlled_workload.rs`
- `experiments/opcode-gas/tests/test_operation_coverage.py`
- `bin/guest-launcher/tests/controlled_workload.rs`

Write failing tests first for these invariants:

- an Anchor transaction receives no `tx_base` and remains an Anchor envelope;
- every valid `phase=transaction` operation joined to that Anchor matches execution coverage;
- a `phase=system` operation remains block-owned;
- an unattempted transaction cannot contribute an operation;
- a confirmed Anchor CALL/CREATE wrapper is charged once through the wrapper model;
- child execution inside Anchor uses ordinary operation coverage with zero extra child charge;
- corrected operation coverage cannot be sealed into a composite estimator until Task 4 has
  refitted the higher-layer costs.

Replace `non_anchor_started_transaction` with a started-transaction execution selector. Keep the
non-Anchor predicate only on transaction-envelope and native-transfer features. Replace ownership
metadata such as `transaction_non_anchor_only` / `anchor_operation_ownership=block_base` with exact
values that distinguish transaction execution from the Anchor envelope.

Generate a new operation-coverage artifact from source. Do not edit a sealed JSON file by hand.
Do not publish a corrected composite schema in this task: the old `block_base` absorbed Anchor work,
so combining it with corrected operation ownership would double-charge Anchor execution. Task 5
introduces the next composite schema only after Task 4 seals the replacement higher-layer fit.

## Task 2: Add Controlled `BLOCKHASH` Calibration

Use the production proposal guest, not the small opcode-lab ELF, so the target executes through the
same REVM and witness-backed ancestor-hash path as an Anchor transaction.

Add a controlled context workload whose target and control have identical loop, stack, bytecode
length, gas limit, transaction envelope, block envelope, and non-target operation counts:

```text
target:  NUMBER; PUSH offset; SUB; BLOCKHASH; POP
control: NUMBER; PUSH offset; SUB; ISZERO;    POP
```

Predeclare these three semantic classes against the controlled fixture's complete 256-header
window:

- `recent_ancestor_hit_1`, using `NUMBER - 1`;
- `recent_ancestor_hit_256`, using `NUMBER - 256`;
- `out_of_range_zero`, using the current block number.

Use fit counts `0, 1, 2, 4, 8, 16, 32` and the fixed extrapolation checkpoint `64`, with three exact
repeats per row. The target/control raw-gas and operation ledgers must match the declared signed
delta exactly. Solve the target event cost using the already sealed `ISZERO` control cost. Require
positive signal, stable repeats, fit/checkpoint APE at most 10%, and exact production guest/launcher
identity.

Because the current production trace does not encode the requested block number or hit/miss result,
the first candidate is a conservative static approximation applied to `opcode:0x40`: use the
maximum accepted per-event cost across the required semantic classes. Record both class estimates
and the selected maximum. If a required class fails, `BLOCKHASH` stays unmeasured; do not infer it
from the proposal smokes.

## Task 3: Close Remaining Material Anchor Gaps

After routing Anchor operations through the corrected evaluator, produce an exact per-key gap
histogram from the existing diagnostic traces. The smoke traces may prioritize work but may not fit
coefficients.

For each remaining material Anchor key, choose one explicit outcome:

- reuse a sealed model when the event schema and execution semantics match;
- run a controlled target/control calibration;
- use a documented conservative approximation with a materiality bound;
- keep it unmeasured and block the new block fit.

Direct precompiles and confirmed wrappers remain separate operation families. Do not hide either in
`block_base`. The operation gate closes only when every operation in the controlled higher-layer
fixtures is either priced or intentionally absent by construction.

## Task 4: Refit Higher-Layer Costs

Re-run higher-layer controlled observations with Anchor operation units included in
`absolute_operation_pricing_units`. Preserve these feature meanings:

- `tx_base`: started non-Anchor transaction envelope count;
- `native_value_transfer`: committed non-Anchor native-transfer count;
- `block_base`: block-owned residual after all priced transaction-phase operations, including
  Anchor operations, are subtracted;
- `proposal_startup`: one per guest invocation after lower-layer subtraction.

Do not reuse the old `block_base` value. Seal a new derivation and rerun the existing frozen
controlled block fit/holdout split without changing membership after results are visible. A failed
holdout may open a new state/trie experiment, but it may not be repaired by restoring Anchor work to
the block offset.

## Task 5: Rebuild And Validate The Composite Candidate

Build a new estimator from the corrected operation coverage, `BLOCKHASH` artifact, remaining
operation models, and refitted higher-layer derivation. It must fail closed on predecessor ownership
metadata.

Validation order:

1. focused Rust and Python ownership tests;
2. exact artifact replay and mutation tests;
3. production SP1 guest build and estimator replay at its bound implementation revision;
4. independent adversarial review;
5. independent behavioral verification;
6. rerun Hoodi `80907` and Mainnet `39339` as integration smokes only after the candidate is sealed.

The smoke results may accept or reject the frozen candidate. They may not change a coefficient,
coverage rule, approximation, or threshold. The final 60-row corpus remains closed.

## Exit Gate

This correction is complete only when:

- Anchor operations contribute to the same per-key operation sum as ordinary transactions;
- Anchor envelope and true system work remain block-owned without double charge;
- `BLOCKHASH` has controlled production-guest evidence or remains an explicit blocker;
- no controlled block row has an unpriced operation;
- a new block fit and holdout pass under the corrected decomposition;
- a new composite estimator replays exactly and both independent review roles close their findings;
- the two integration smokes run only after freeze and report their actual coverage and APE.
