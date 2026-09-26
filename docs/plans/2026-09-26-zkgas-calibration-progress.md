# ZKGas Calibration Progress

## Status

In progress.

Last updated: 2026-09-26.

This document is the execution-status ledger for the ZKGas calibration work. It records what is
currently proved, what is actively being changed, and which gate opens the next layer. It does not
replace the [architecture design](2026-09-26-zkgas-calibration-design.md), the
[execution plan](2026-09-26-zkgas-calibration-execution-plan.md), or the operational details in the
[opcode experiment README](../../experiments/opcode-gas/README.md).

Update this file only at a milestone boundary. Do not use it as a command log or copy generated run
output into it.

## Canonical Objective

Build a high-precision estimator from host-computable execution and state features to backend-native
ZK proving cost. V1 measures SP1 normalized `proverGas`. A later backend may independently calibrate
the same workload decomposition in its own native cost unit.

The target decomposition is:

```text
proposal_cost =
    proposal_startup
  + sum(block_base
      + state_trie_cost
      + sum(tx_base
          + native_transfer_cost
          + opcode_cost
          + precompile_cost
          + call_create_wrapper_cost))
```

`tx gas` in this project means the per-operation execution ledger and actual raw EVM gas. It never
means transaction `gas_limit`, and final receipt `gasUsed` alone is not sufficient because executed
work may later revert or reset.

The state/trie term is a separate architectural layer, not a cost assigned blindly to each
`SLOAD`/`SSTORE`. The first block-level model may keep fixed witness/trie/hash work in `block_base`.
Split a variable state/trie function only when controlled block residuals show that the coarse model
is insufficient.

## Version Identity

Keep these version axes distinct:

- Taiko fork and production ZKGas schedule: Unzen and `UNZEN_ZK_GAS_SCHEDULE`;
- Ethereum upgrade terminology: Fusaka;
- EVM execution specification used by REVM: `SpecId::OSAKA`;
- proving-cost backend for V1: SP1 normalized `proverGas`.

Raiko2 maps `TaikoFork::Unzen` to `SpecId::OSAKA`. Every new controlled fixture must therefore use
Osaka execution semantics while continuing to bind the actual exported Unzen schedule identity.

## Layer Progress

| Layer | State | Exit gate |
| --- | --- | --- |
| Experiment framework | Core scope complete | Preserve its identity and replay invariants |
| Opcode core | In progress | Seal the bounded 101-plus-2 augmentation |
| Remaining operations | Pending | Seal execution coverage and event ownership |
| State/trie | Pending | Pass a coarse holdout or justify a split model |
| Transaction | Pending | Pass controlled fit and holdout |
| Block | Pending | Pass frozen block holdouts without proposal fitting |
| Proposal | Closed | Seal every lower layer before opening the corpus |
| Other ZKVM backends | Future | Measure natively or validate an explicit bridge |

## Sealed Baseline

The current immutable baseline is derivation `3e1d97c461cd2ef9a40e6a02`, built from historical
calibration run `09ebb08d76d3f461086b0cf4`.

- Core status: `supported_core_submodel`.
- Modeled named opcodes: 101 of 150.
- Explicitly unsupported named opcodes: 49.
- Dynamic model: supported with exact rank 14 of 14.
- Structured opcode families: `EXP`, `KECCAK256`, `MLOAD`, `MSTORE`, `MSTORE8`, and `MCOPY`.
- `ISZERO` is unmeasured because its historical relation depended on dispatch-only `NOT`.
- `CLZ` is absent because the historical opcode guest used Prague execution semantics.
- Candidate eligibility: false.
- Production schedule, runtime configuration, block limit, and Boundless configuration changed: no.
- Final-validation proposals opened: no.

The baseline is historical evidence. Never rewrite it or claim that its 101 coefficients were
measured under the Osaka guest.

## Active Milestone: Osaka Opcode Supplement

The active milestone is deliberately bounded:

1. Run the REVM opcode lab with `SpecId::OSAKA`.
2. Add `CLZ` to the controlled manifest and core inventory.
3. Give `ISZERO` and `CLZ` a one-opcode `SWAP1` control with matching stack shape and final stack
   height, so common dispatch cancels without depending on `NOT`.
4. Complete focused Python and Rust tests, build the SP1 guest artifacts, and independently review
   the complete change.
5. Execute a small compatibility canary for representative sealed baseline families. The canary is
   a reuse check, not coefficient fitting and not a full resample.
6. Sample only the `ISZERO` and `CLZ` formal relations under the Osaka guest.
7. Seal an augmented, non-candidate artifact that binds both the immutable 101-opcode baseline and
   the two supplemental relations. Its provenance must state that the old coefficients were reused,
   not remeasured.

Expected coverage after this milestone is 103 of 150 only if both supplemental relations pass all
predeclared gates and the compatibility canary shows no material baseline drift. Failure preserves
the 101-opcode baseline and records the failed supplement; it does not trigger an automatic full
resample.

Current worktree state at the last update:

- Task 1 landed in `3ea7a8d4` with review fix `c6b64e23`;
- the bounded Osaka/CLZ/SWAP1 inputs and structured Unzen/Fusaka/Osaka calibration identity are
  implemented;
- root verification passed 345 Python tests with one skip, 15 SP1 guest library tests, 24 chain-spec
  tests, and four schedule-exporter tests;
- the SP1 artifact provenance check is current; two forced builds were deterministic;
- the REVM lab ELF/VK changed as expected, while the opcode-lab VK and all its `PT_LOAD` segments
  remained byte-identical despite deterministic non-loaded ELF metadata churn;
- independent review found one missing Prague-negative CLZ regression, verified its fix, and ended
  with both spec-compliance and correctness verdicts passing;
- no sampling or proposal execution ran, and no production schedule or configuration changed.

## Next Gate

Implement Task 2's historical-schema validator, exact compatibility-canary subset, bounded
ISZERO/CLZ supplement runner, and create-only augmentation sealer. Complete its full test suite and
independent review before running any sampling. Proposal execution remains closed.

## Next Milestones

### 1. Close Operation-Layer Coverage

Inventory the remaining opcode/precompile execution keys separately from their emitted side-effect
events. Do not assume all remaining execution keys need independent scalar measurements. The valid
execution outcomes are:

- a static raw-gas relation;
- a typed opcode-specific cost function;
- a precompile or fixed wrapper-event model;
- unreachable or inactive under the frozen fork identity;
- explicitly unsupported with measured coverage reported later.

Maintain a separate exact-owner ledger for wrapper, state/trie, transaction, and block events. An
SSTORE or CALL may have an operation execution model and separately owned side effects; never assign
the whole opcode to one higher layer. Seal both ledgers before fitting transaction or block residuals.

### 2. Calibrate State/Trie And Transaction Costs

Start with the smallest controlled feature set: fixed block state work, started non-Anchor
transaction count, and native value-transfer count. Keep opcode execution and final trie work in
different ownership domains. Add unique-access, witness-node, dirty-state, or hash/update features
only if the frozen coarse model fails.

### 3. Calibrate Block And Proposal Costs

Fit block base and holdouts from controlled production-guest fixtures. After every lower component,
formula, threshold, and coverage rule is sealed, open the fixed proposal corpus exactly once for
final validation. Proposal residuals may motivate a new experiment but may not repair the current
candidate.

### 4. Extend To Other Backends

Keep SP1 `proverGas`, SP1 instruction count, and RISC0 cycles as different units. A future RISC0 run
executes the same controlled workload identities and may produce a backend-specific table. Only an
independently validated bridge may transport a cost vector between backends.

## Current Non-Goals

Until the active milestone is sealed, do not:

- resample all 101 historical opcode coefficients;
- run the final proposal corpus;
- update the production multiplier table or integer scale;
- change the production block ZKGas limit;
- update Boundless quoting;
- treat SP1 instruction count as proving cost;
- attribute state finalization or revert behavior to an isolated opcode coefficient.

## Update Checklist

At each milestone update, record:

- exact immutable run or derivation IDs;
- implementation revision and guest ELF identity;
- supported, unsupported, and unmeasured coverage counts;
- validation commands that actually passed;
- independent review outcome;
- whether any proposal result was opened;
- whether any production code, schedule, or configuration changed;
- the single next gate and any material blocker.
