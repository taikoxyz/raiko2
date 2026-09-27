# ZKGas Calibration Architecture Design

## Status

Accepted.

This is the authoritative design for the current ZKGas calibration work. Use the
[execution plan](2026-09-26-zkgas-calibration-execution-plan.md) for task order and the
[progress ledger](2026-09-26-zkgas-calibration-progress.md) for current evidence and the next gate.

The earlier multiplier-recalibration and June component documents are retained as historical
rationale. Where they conflict with this document, this document controls.

## Objective

Build a high-precision, host-computable estimator of backend-native ZK proving cost from the work a
proposal actually performs. V1 measures SP1 normalized `proverGas`; it does not choose a production
integer ZKGas scale or update the production schedule.

The estimator is hierarchical:

```text
proposal_cost =
    proposal_startup
  + sum(block_base
      + state_trie_cost
      + sum(tx_base
          + native_transfer_cost
          + sum(opcode_cost)
          + sum(precompile_cost)
          + sum(call_create_wrapper_cost)))
```

Each layer owns its work exactly once. Calibration starts with isolated operations and expands to
state/trie, transaction, block, and finally proposal scope. If a coarser upper-layer model passes its
frozen holdouts, the experiment does not split it merely to produce a more complicated table.

## Version Identity

The following are independent version axes and must all appear in provenance:

- Taiko fork: Unzen;
- production ZKGas schedule source: `UNZEN_ZK_GAS_SCHEDULE` from the pinned Alethia revision;
- Ethereum upgrade terminology: Fusaka;
- EVM execution specification: REVM `SpecId::OSAKA`;
- V1 proving-cost backend: SP1 normalized `ExecutionReport::gas()`;
- guest program identity: exact ELF/VK hashes;
- host execution identity: exact `guest-launcher` hash and dependency revisions.

Persist the symbolic axes together rather than inferring them later:

```json
{
  "version_identity": {
    "taiko_fork": "Unzen",
    "production_schedule": "UNZEN_ZK_GAS_SCHEDULE",
    "ethereum_upgrade": "Fusaka",
    "revm_spec_id": "OSAKA",
    "proving_backend": "sp1",
    "primary_metric": "proverGas"
  }
}
```

The Rust schedule exporter binds `taiko_fork` and `production_schedule` to the concrete
`UNZEN_ZK_GAS_SCHEDULE`. It obtains `revm_spec_id` from the same chain-spec mapping used by runtime
selection: `TaikoFork::Unzen -> SpecId::OSAKA` in `crates/primitives/src/chain_spec.rs`; it must not
duplicate that mapping as an exporter literal. `ethereum_upgrade = "Fusaka"` is an explicit
vocabulary label for the opcode set, not a value derived from the production schedule. Calibration
provenance adds the last two fields and includes the complete object in its content identity. A
missing or mismatched field fails run, supplement, augmentation, and replay validation.

Raiko2 maps `TaikoFork::Unzen` to `SpecId::OSAKA`. Controlled REVM fixtures therefore use Osaka
execution semantics while continuing to bind the actual Unzen production schedule. Changing any
version axis creates new evidence; it never silently relabels an older run.

## Cost Units

Use these terms consistently:

- `proverGas`: SP1's normalized proving-cost estimate and the sole V1 target;
- `raw_evm_gas`: gas charged by the interpreter or native precompile for one executed operation;
- `gas_limit`: an execution envelope only, never a workload count or proving-cost input;
- `receipt gasUsed`: a transaction result that is insufficient to reconstruct all attempted work;
- `sp1_instruction_count`: a diagnostic cycle proxy, never true SP1 zkcycles or proving cost;
- `risc0_user_cycles`: a different backend-native unit that V1 does not infer from SP1.

The estimator consumes an executed-operation ledger, not one scalar transaction gas value. Work
that executed and later reverted, reset, or exceeded the current ZKGas charge remains proving work.
Intrinsic or pre-validation failures and the unattempted tail after block truncation do not.

## Layer Ownership

### Operation Layer

The operation layer owns EVM interpreter execution, direct precompile bodies, and confirmed
CALL/CREATE wrapper events. It does not own final state-trie commit, transaction-envelope work,
Anchor/system execution, or proposal startup.

The fit keeps lab-body parameters and a cross-environment `body_scale` separate. Promotion applies
that scale exactly once and stores production-scaled parameters in the typed registry. Runtime and
higher-layer estimators consume those stored parameters directly; they do not apply `body_scale`
again. The registry therefore supports these two executable model shapes:

```text
static_opcode_cost(e) = common_dispatch
                      + stored_body_per_raw_gas[k] * raw_evm_gas(e)

structured_opcode_cost(e) = common_dispatch
                          + stored_f_k(context(e))

stored_body_per_raw_gas[k] = body_scale * fitted_lab_body_per_raw_gas[k]
stored_f_k                  = body_scale * fitted_lab_f_k
```

The historical higher-layer derivation preserves its original extra-scale residualization only for
byte-exact replay. Any new composite estimator must reproject its sealed raw rows through the typed,
production-scaled registry semantics above.

Use the static shape when one per-key body coefficient passes controlled relations and holdouts. Use
a typed `f_k` only when measured evidence rejects the static shape. The current structured families
are:

- `EXP`: conservative small-exponent bucket plus a larger-exponent function;
- `KECCAK256`: zero-length branch, permutation count, and memory term;
- `MLOAD`, `MSTORE`, and `MSTORE8`: opcode body plus shared memory-growth function;
- `MCOPY`: fixed term, copied-word term, and shared memory-growth function.

`NOT` and `JUMPDEST` currently use explicit dispatch-only approximations. Their frequency-dependent
error remains visible in block validation and cannot be hidden in a fixed offset.

Direct precompiles use their native-gas or declared semantic features. For `CALL`/`CREATE`, a
confirmed spawn replaces the pending opcode raw-gas row with one fixed wrapper row; the two rows are
never both charged. Forwarded child gas is never priced as wrapper work because executed child
operations are counted through ordinary operation coverage. `child_execution` is a derived grouping
with zero additional charge.

### State And Trie Layer

State/trie cost is cross-cutting rather than an opcode suffix. An `SLOAD` or `SSTORE` may trigger a
lookup or dirty-state change, but it does not necessarily perform a full leaf-to-root hash traversal
at that point.

The working decomposition is:

- witness/state input validation;
- lookups against the already materialized witness database;
- dirty account/storage tracking;
- final trie update, hashing, and root computation.

The first V1 block model may keep fixed witness/trie/Merkle/hash work inside `block_base`. Only a
failed frozen coarse-model holdout may open a new candidate with explicit features such as witness
nodes, unique account/storage accesses, dirty nodes, or hash/update counts. Proposal residuals may
not select those features.

### Transaction Layer

`tx_base` is charged once when the executor starts a non-Anchor candidate transaction. A committed,
successful, non-create native value transfer with no executable recipient or precompile dispatch
adds `native_transfer_cost`.

Native transfer and EVM execution are distinct paths. A native transfer is an EOA-to-EOA value
transfer with no operation trace. A contract call instead enters the CALL/CREATE wrapper and
opcode/precompile model. Rust's `TxKind::Call(address)` only distinguishes a non-creation
transaction; it does not by itself mean that the recipient has executable code. The two paths must
never share one coefficient or use the opcode sum as a substitute for native balance-transfer work.

Opcode work from an attempted transaction remains in the operation sum even if the transaction later
reverts. State commit work follows the state/trie outcome rather than being folded back into an
isolated `SSTORE` coefficient.

### Block Layer

`block_base` owns pre-transaction system work, the Anchor transaction, block context, and the coarse
fixed state/trie/hash baseline until controlled evidence justifies a separate variable term. Those
events cannot also enter the non-Anchor operation sum.

### Proposal Layer

`proposal_startup` occurs exactly once per proposal guest invocation. It is distinct from per-block
and per-transaction bases. Proposal rows validate a completely frozen lower-layer model and never
fit an intercept, scale, coefficient, feature, fallback, or threshold.

## Controlled Operation Measurement

The primary operation lane is the reviewed, revm-backed SP1 opcode lab executed with the SP1
gas-estimator engine. It is an experiment guest, not the production proposal guest. The estimator
still executes the guest ELF; it avoids proof generation and heavy profiling. Synthetic
direct-interpreter results are diagnostics only, while higher-layer calibration uses the production
proposal guest.

Every formal relation binds:

- target and control bytecode bytes;
- target/control opcode counts and signed raw-gas maps;
- operand profile, stack shape, memory state, fixed gas limit, padding, and suffix;
- exact GuestInput bytes and hashes;
- manifest, schedule, implementation, launcher, and guest identities;
- three repeated observations and the adaptive-count decision ledger.

Target and control must keep non-target executed work constant. An early `STOP`, a changed final
stack shape, a changed memory footprint, or metadata-only relabeling is invalid evidence.

For slope-based relations, fit the first passing frozen count prefix and then execute its
predeclared larger checkpoint. The checkpoint does not participate in fitting. A failed signal,
determinism, rank, residual, extrapolation, or provenance gate remains explicit and cannot be
repaired by a proposal row.

## Historical Baseline And Osaka Supplement

The sealed historical baseline models 101 named opcodes. Its raw measurements used the Prague REVM
opcode-lab configuration, while its production controlled-block evidence used the production guest.
The baseline remains immutable.

The current Osaka change does not trigger a full historical resample. Reuse is allowed only through
this bounded procedure:

1. Build and bind the Osaka opcode guest.
2. Replay these exact non-fitting compatibility relations with their historical programs, counts,
   and raw-gas maps:
   - `opcode:0x01:canonical`;
   - `opcode:0x02:canonical`;
   - `opcode:0x10:canonical`;
   - `opcode:0x1b:canonical`;
   - `opcode:0x57:canonical`;
   - `opcode:0x19:canonical`;
   - `opcode:0x5b:canonical`;
   - `opcode:0x0a:exp-bytes-1`;
   - `opcode:0x20:input-32`;
   - `opcode:0x51:offset-0x00`;
   - `opcode:0x5e:copy-32`.
3. For each nonzero historical slope, compute
   `drift_APE = abs(osaka_slope - historical_slope) / abs(historical_slope)`. Require matching
   relation identity and raw-gas maps, the same slope sign, per-relation drift APE at most 10%, and
   arithmetic-mean drift APE at most 5% across all eleven relations.
4. Treat the canary only as evidence that unchanged baseline coefficients remain reusable. It does
   not refit, rescale, or replace any historical coefficient.
5. If the canary fails, stop. Record the drift and design a separately reviewed recalibration; do
   not automatically launch a full resample.

After the canary passes, sample only:

- `ISZERO` (`0x15`) against a one-opcode `SWAP1` control;
- `CLZ` (`0x1e`) against a one-opcode `SWAP1` control under Osaka.

Both pairs use the same two-item stack shape, execute one target/control opcode per repetition, and
finish with the same stack height. Their equations are:

```text
3 * mu(ISZERO) - 3 * mu(SWAP1) = d_iszero
5 * mu(CLZ)    - 3 * mu(SWAP1) = d_clz
```

The augmented artifact reuses all 101 baseline models unchanged and adds the two solved keys only
after both relations pass the standard adaptive and checkpoint gates. It binds the complete baseline
derivation, Osaka guest identity, canary artifact, supplemental raw rows, decisions, and relation
artifact. It states explicitly that 101 coefficients were reused rather than remeasured.

## Remaining Operation Coverage

After the Osaka supplement, maintain two orthogonal ledgers.

The execution-coverage ledger classifies each named opcode and precompile body exactly once as:

- static raw-gas model;
- structured opcode function;
- direct precompile model;
- inactive or unreachable under the frozen fork;
- explicitly unsupported.

The side-effect/event ledger separately classifies each emitted work component exactly once. Its
owners include operation wrapper, state/trie, transaction, and block. Examples include confirmed
CALL/CREATE spawn wrappers, account or storage access, dirty-state updates, final trie work,
transaction envelope processing, and Anchor/system work.

An opcode identity may therefore have one execution model and emit separately owned events. `SSTORE`
keeps its interpreter execution in the operation ledger while dirty-state/final-trie work belongs to
the state/trie ledger. For `CALL`/`CREATE`, a confirmed spawn replaces the pending opcode raw-gas row
with one fixed wrapper row. Executed child operations keep their ordinary execution rows;
`child_execution` is a derived grouping with zero additional charge. Exact-one ownership applies to
each charged work component or derived grouping, never to the whole opcode identity.

The inventory is complete when no active execution key or declared side-effect event is
unclassified. Unsupported execution keys remain visible in proposal coverage; they never receive a
current-schedule multiplier disguised as measured cost.

## Controlled Higher-Layer Calibration

Higher-layer fixtures use the production proposal guest and freeze all rows before opening results.
They first test the smallest model:

- one proposal-startup value;
- one block-base value;
- one started-non-Anchor-transaction value;
- one native-transfer value;
- the frozen operation registry;
- no independent variable state/trie term.

Fit and holdout rows must keep ownership unambiguous. If the coarse block holdout fails, inspect the
controlled residual and create a new, predeclared state/trie feature matrix. Never select a feature
from final proposal residuals.

### Declared Native-Transfer Approximation

The canonical production-guest run `999b91b91fd693899d09fa53` showed that native-transfer cost is
positive but is not identifiable as a high-precision constant with the current matched control.
Changing transaction value changes the signed transaction hash and therefore the SP1 signature-
recovery workload. A successful positive transfer also changes sender and recipient balances and
the final trie. These effects are real proving work, but their combined residual is immaterial at
the whole-guest scale and must not block the next calibration layer.

V1 therefore declares a conservative native-transfer approximation:

```text
native_transfer_cost = 5017 proverGas per committed native EOA transfer
native_transfer_materiality_budget = 0.002
```

`5017` is the maximum observed `delta_prover_gas / transfer_count` across the frozen nonzero counts
`1, 2, 4, 8, 16, 32, 64, 128` in that run. It is an upper-bound approximation, not an accepted OLS
coefficient. A new campaign validates this already frozen value; it must not select a new maximum or
refit the value after opening its rows.

For every frozen nonzero native-transfer count `n`, validate:

```text
native_delta(n) = target_prover_gas(n) - control_prover_gas(n)

native_materiality(n) =
    abs(5017 * n - native_delta(n)) / target_prover_gas(n)

max(native_materiality(n)) <= 0.002
```

Identity, exact-repeat, successful-execution, lane, ownership, and positive-signal checks remain
mandatory. The materiality rule replaces only the requirement that this small mixed signal pass the
ordinary two-percent relation-residual and checkpoint-relative-error gates. The source campaign's
largest observed whole-guest materiality is at count 128: `219576 / 181155333`, or approximately
`0.00121209`, below the frozen `0.002` budget.

Artifacts retain `native_value_transfer` as an independent transaction-layer term and record its
status as `declared_approximation`. They record `tx_base`, `block_base`, and `proposal_startup` as
ordinary accepted measurements only when those terms pass their original gates. A dependency blocks
a case only when that dependency has a nonzero delta in the case equation. Consequently, the native
approximation does not block block-base or proposal-startup fixtures whose native-transfer delta is
zero.

The fixed model status is `accepted_with_declared_approximation` only after all three ordinary fixed
terms pass and the native materiality rule passes. State holdouts may then open. Their pairwise
predictions cancel the native approximation when the two lanes have equal native-transfer counts;
signature recovery and trie differences remain visible in the observed higher-layer residual. A
passing coarse holdout permits continued use of the approximation. A failed holdout motivates a
new predeclared transaction or state/trie model rather than a result-time change to `5017` or the
`0.002` budget.

This approximation does not by itself open the final proposal corpus or authorize production-table
changes. All lower-layer artifacts, holdouts, formulas, coverage statements, and approximation
evidence must still be sealed first.

## Candidate And Proposal Validation

Before final validation, seal:

- the typed operation registry and explicit coverage gaps;
- state/trie ownership and any accepted variable model;
- transaction, block, and proposal fixed costs;
- the exact forward formula;
- per-layer coverage definitions;
- numeric thresholds and corpus identity;
- all implementation, schedule, guest, launcher, and artifact hashes.

For proposal `j`:

```text
p_hat(j) = proposal_startup
         + sum(block_base + state_trie_cost(block))
         + sum(tx_base + native_transfer_cost(tx))
         + sum(resolve_opcode_model(event))
         + sum(resolve_precompile_model(event))
         + sum(resolve_wrapper_model(event))

APE(j) = abs(p_hat(j) - observed_prover_gas(j)) / observed_prover_gas(j)
```

Use `Decimal` arithmetic for canonical observations, predictions, errors, and serialized numeric
values. Proposal rows report operation-count, raw-gas, wrapper-event, and state-feature coverage
separately. No arbitrary coverage threshold is invented after results are visible; conclusions name
the reported coverage.

The fixed proposal corpus opens only after the candidate is sealed. Validation may produce pass,
failure, or insufficient-coverage evidence, but it cannot write to the calibration root or change a
candidate value.

## Backend Bridge Boundary

Record SP1 instruction count alongside proverGas as an independently hashed diagnostic. A controlled
instruction-count-to-proverGas bridge may be evaluated, but it never gates or modifies the SP1
candidate.

RISC0 and future ZKVMs require either:

- independent execution of the same controlled workload identities to produce a backend-specific
  model; or
- a separately frozen and validated transport model.

RISC0 cycles are never relabeled as SP1 proverGas, and a failed SP1 instruction-count bridge does not
prevent independent RISC0 calibration.

## Production Boundary

All calibration artifacts are review-only and backend-native. This work does not:

- write Alethia's production multiplier table;
- choose integer rounding or a common multi-backend scale;
- change intrinsic charges, spawn estimates, or the block ZKGas limit;
- change Boundless quoting;
- publish or submit a network proof;
- promote a model automatically after validation.

Production promotion is a separate reviewed change after the experiment is complete.

## Completion Criteria

The experiment is complete only when:

- all active opcode/precompile executions are classified and every measured model has replayable
  evidence;
- all declared side-effect events have exactly one layer owner with no whole-opcode ownership
  shortcut;
- the state/trie ownership boundary is explicit and its chosen coarse or split model passes holdouts;
- transaction, block, and proposal fixed costs pass controlled fit and holdout gates, except for a
  separately declared approximation that passes its frozen whole-model materiality budget;
- the complete candidate and coverage rules are sealed before proposal output is opened;
- final proposal predictions and coverage are independently reproducible;
- no production schedule or configuration was changed by the experiment.
