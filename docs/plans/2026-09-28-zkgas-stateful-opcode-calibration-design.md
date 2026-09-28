# Stateful Opcode Calibration Design

## Status

Approved on 2026-09-28. The executable plan is
`docs/plans/2026-09-28-zkgas-stateful-opcode-calibration-implementation-plan.md`.

This is a bounded follow-up to the first composite-proposal diagnostics. It measures `SLOAD` and
`SSTORE` in a stateful REVM laboratory before deciding whether either operation can be represented
by a fixed coefficient, a raw-gas relation, or a typed semantic function. It does not promote a
model, change the composite estimator, or reopen proposal validation.

## Decision

Run a new controlled SP1 campaign whose primary result is a per-scenario
`stateful_revm_execution_cost` report. Do not assume a model shape before seeing the sealed result.

The experiment compares three predeclared candidate shapes:

1. one fixed execution cost per opcode;
2. a fixed warm body plus a cold-access increment;
3. for `SSTORE`, a typed semantic-branch function plus a cold-access increment.

The raw EVM gas relation is reported as a diagnostic fourth view. It is not automatically selected
as the production shape because the raw-gas buckets mix access warmth and storage-transition
semantics.

No result from this campaign enters a proposal estimator until a separate promotion change adds the
required typed trace fields, seals a new operation registry and coverage ledger, and passes an
independent review.

## Why A New Stateful Lab Is Required

The current `revm-opcode-lab` input contains bytecode and count metadata but no initial storage or
access-list state. Every fixed microprogram executes against a fresh `BenchmarkDB`, which returns
zero for every storage slot. It therefore cannot express a nonzero original value, a pre-warmed
slot, or dirty and restore-original branches.

The current host and guest identities also disagree: the guest executes `SpecId::OSAKA`, while the
host workload identity and footprint trace still declare and execute Prague. The stateful campaign
must use Osaka on both sides before any formal sample is accepted. Historical sealed opcode evidence
is not rewritten or automatically re-sampled; the correction applies to this new campaign and later
artifacts.

## Cost Ownership Boundary

Whole-guest `proverGas` cannot split work that REVM performs inside one storage instruction. The
stateful operation term therefore owns all work that is inseparable at that boundary:

- opcode dispatch and stack handling;
- dynamic gas and refund branch evaluation;
- the in-memory account/storage lookup performed by REVM;
- per-execution journal bookkeeping and construction of REVM's in-memory result state.

The state/trie layer continues to own work absent from the laboratory:

- production `GuestInput` witness decoding and linkage validation;
- materializing the production witness-backed database and any external backing fetch;
- persistence of the final unique dirty account/storage set after transaction outcome is known;
- hashed post-state construction, trie updates, hashing, and final root computation.

This refines the earlier broad phrase “dirty-state tracking.” Per-execution REVM journal mutation is
part of the measurable stateful opcode execution. Persistence and trie work over the final committed
dirty set remain state/trie work. A later state model must not charge the journal work again.

Reverted transactions still pay for stateful opcode execution. Persistent dirty-state and final-trie
work depend on the final transaction/block outcome and must be measured separately.

## Alternatives Considered

### Warm No-op Constant Only

Measure a pre-warmed `SLOAD` and an `SSTORE` that writes the current value, then immediately promote
one fixed constant per opcode. This has the smallest code change but leaves cold access and every
mutating `SSTORE` branch untested. It is useful as the first row of the campaign, not as the whole
campaign.

### Controlled Stateful Scenario Matrix

Extend the lab with explicit prestate and warmth, run matched target/control pairs for a frozen
scenario matrix, and compare the predeclared model shapes. This is the selected approach. It answers
whether a simple approximation is adequate without mixing in production witness or final-trie work.

### Production-Guest State Instrumentation First

Add access, transition, committed-dirty, witness, and final-trie events to the production proposal
path before measuring storage operations. This can eventually split every state component but is a
much larger change and would combine operation calibration with the next architectural layer. It is
deferred unless the controlled campaign shows that the smaller model cannot meet its gates.

## Stateful Input Contract

Add an optional, strongly typed storage contract to `OpcodeLabInput`. The exact Rust representation
may use enums, but its serialized meaning must be equivalent to:

```text
storage = {
    measurement_opcode: 0x54 | 0x55,
    lane: target | control,
    slot: 32-byte canonical hex,
    original_value: 32-byte canonical hex,
    access: cold | warm,
    operation: load { expected_value: 32-byte canonical hex }
             | store {
                   current_value: 32-byte canonical hex,
                   new_value: 32-byte canonical hex
               }
}
```

`measurement_opcode` names the stateful operation under study. The existing top-level `opcode`
continues to name the opcode whose count/raw gas the concrete lane declares: it equals
`measurement_opcode` in the target lane and the matched reference opcode in the control lane. The
structured `operation` describes the target semantic scenario in both lanes so the pair carries
identical prestate and access-list state. Validation is lane-aware: the target must execute the
declared state operation at the measured site, while the control substitutes the sealed reference
there. A dirty/restore control still executes the identical declared prefix `SSTORE` at the same
position as the target; it may not execute any other state opcode. Clean controls execute no state
opcode. This avoids pretending that the control's measured site performs the state transition
while preserving the prefix work that must cancel in a dirty relation.

The primary matrix uses values `0`, `1`, and `2` because the required semantic classes depend on
zero and equality relationships. The wire contract nevertheless carries complete 256-bit values,
and the campaign includes frozen high-limb diagnostics described below. The validator must reject
an operation that disagrees with the top-level opcode, bytecode, or expected transition.

Each fixed microprogram receives a newly constructed but identical in-memory database. It contains
the benchmark caller, the fixed contract bytecode, and the declared initial slot value. A warm case
places the slot in the transaction access list; a cold case does not. Every sweep member uses the
same number of microprograms, the same transaction envelope, the same gas limit, the same serialized
field widths, and the same database-construction path. Only the fixed number of active target slots
changes.

Every storage slot and new-value operand uses a canonical `PUSH32`, including low values `0`, `1`,
and `2`. Low-value and high-limb variants must have identical opcode counts, bytecode length,
operand positions, and signed reference ledger; only the declared 32-byte immediate spans may
differ. Generation persists the ordered immediate spans and a program-shape hash computed after
zeroing those spans. Admission compares the masked programs byte-for-byte and rejects any other
byte difference. This keeps the high-limb diagnostic from measuring a `PUSH0`/`PUSH1` versus
`PUSH32` setup difference or an unrelated program-shape change.

The guest and host footprint tracer must share one database and `TxEnv` constructor. Both must
execute Osaka. A fixture is invalid unless the host trace confirms the declared target count, raw
gas, access class, and storage transition for every active target microprogram.

The measured guest must not add an observer, walk the returned state, or commit per-operation state
details: that instrumentation would change the ELF and the `proverGas` being measured. Instead:

- the measured guest validates the canonical input/program contract and uses the shared constructor;
- a host-native trace pass validates the actual opcode, branch, raw gas, access class, and result;
- test-only or separately built non-measured semantic checks inspect the loaded value and returned
  in-memory state for every canonical case;
- provenance binds the shared source, formal guest ELF, host trace binary, and exact input hash.

Formal sampling is blocked unless all four checks agree. Semantic-check output is never substituted
for the formal guest's `proverGas`.

The laboratory intentionally does not contain a production Merkle witness or compute a final trie
root. Its result must be named `stateful_revm_execution_cost`, not “complete SSTORE cost” or
“state/trie cost.”

## Frozen Scenario Matrix

### SLOAD

| Scenario | Original value | Access |
| --- | ---: | --- |
| `sload_cold_zero` | `0` | cold |
| `sload_cold_nonzero` | `1` | cold |
| `sload_warm_zero` | `0` | warm |
| `sload_warm_nonzero` | `1` | warm |

### SSTORE Clean Branches

| Scenario | Original/current/new | Access |
| --- | --- | --- |
| `sstore_noop_zero` | `0 / 0 / 0` | warm and cold |
| `sstore_noop_nonzero` | `1 / 1 / 1` | warm and cold |
| `sstore_set` | `0 / 0 / 1` | warm and cold |
| `sstore_clear` | `1 / 1 / 0` | warm and cold |
| `sstore_reset_nonzero` | `1 / 1 / 2` | warm and cold |

### SSTORE Dirty Branch Diagnostics

| Scenario | Prefix and measured transition | Access at measured operation |
| --- | --- | --- |
| `sstore_dirty_rewrite` | `0 -> 1`, then `1 -> 2` | warm |
| `sstore_restore_zero` | `0 -> 1`, then `1 -> 0` | warm |
| `sstore_dirty_rewrite_nonzero` | `1 -> 2`, then `2 -> 0` | warm |
| `sstore_restore_nonzero` | `1 -> 2`, then `2 -> 1` | warm |

Dirty and restore-original branches are necessarily warm because the prefix has already accessed the
slot. The prefix is identical in the target and matched-control lanes and is verified separately;
only the second operation contributes to the reported relation.

### High-limb Diagnostics

Define `H = 2^255`, `H2 = 2^255 + 1`, and high storage slot `Q = 2^255`. Run target/control rows at
counts `0` and `64`, with three repeats, for these diagnostic cases:

- warm and cold `SLOAD` with original value `H`;
- warm and cold `SSTORE` no-op `H -> H`;
- warm and cold set `0 -> H`;
- warm and cold clear `H -> 0`;
- warm and cold reset `H -> H2`;
- dirty rewrite `0 -> H -> H2` and restore original `H -> H2 -> H`;
- repeat warm/cold `SLOAD`, no-op `SSTORE`, set, clear, and reset on slot `Q` rather than slot `0`.

These rows fit no new parameter. They test the corresponding low-value and low-slot model at an
otherwise untouched U256 shape. Every applicable high-limb marginal-signal APE must be at most
`10%`, and low/high variants of the same semantic class must differ by at most `10%`, for that model
shape to be eligible. Failure means the value- or slot-shape dependency needs a versioned typed
model; it is not averaged into the low-value coefficient.

For a high-limb diagnostic `h`, reconstruct its observed absolute per-event cost from its structural
zero and checkpoint rows:

```text
b_h = (median(delta_h(64)) - median(delta_h(0))) / 64
absolute_h = b_h - r_h
consistency_error_h = abs(absolute_h - model_absolute_cost_h)
                      / abs(model_absolute_cost_h)
```

A zero model denominator fails closed. Its model APE uses the same relative reconstruction formula
as the ordinary checkpoint below.

## Sweep, Controls, And Gates

For each scenario, freeze these counts before formal sampling:

- fit: `1, 2, 4, 8, 16`;
- untouched holdout: `32`;
- extrapolation checkpoint: `64`;
- repeats: three identical executions per row.

Count zero remains a structural control but is not part of the slope fit. Target and control use the
same fixed microprogram count and layout. The reference program must preserve stack shape, bytecode
length, transaction count, prestate construction, access list, and any dirty-branch prefix. It must
not execute a storage opcode in place of the measured target. Every non-storage opcode in the
target/control difference must resolve to an exact accepted model in the sealed opcode registry.

For each scenario, freeze a signed per-slot opcode ledger `L_s` from the host trace:

```text
L_s[k] = target_count_per_active_slot[k] - control_count_per_active_slot[k]
L_s[state_opcode] = 1
```

Let `r_s` be the signed cost of every non-storage ledger entry, reconstructed from the exact sealed
registry artifact:

```text
r_s = sum(L_s[k] * sealed_absolute_cost[k] for k != state_opcode)
```

The fitted target-control slope `b_s` is a relative relation, not the absolute storage cost. Recover
and report the latter as:

```text
absolute_stateful_cost_s = b_s - r_s
```

The artifact binds `L_s`, every referenced model ID and parameter, and the registry artifact hash.
Missing, unsupported, structured-without-exact-input, or caller-resealed reference entries are
fatal.

An individual scenario is accepted only when all of the following pass:

- exact guest-input, ELF, launcher, manifest, host-trace, and Osaka identity checks;
- exact target/control footprint and declared storage-transition checks;
- positive signal above the existing controlled-run noise floor;
- `R² >= 0.995` on fit rows;
- relative slope standard error `<= 5%`;
- maximum fit residual divided by marginal signal `<= 2%`;
- count-zero/intercept residual divided by marginal signal `<= 2%`;
- repeat spread `<= 5%` of the fitted per-event cost;
- holdout APE `<= 10%`;
- checkpoint APE `<= 10%`.

For each scenario, define the matched-control signal

```text
delta_s(n) = prover_gas_target_s(n) - prover_gas_control_s(n)
```

Every fit has a free per-scenario intercept `a_s`. For fit counts `F = {1, 2, 4, 8, 16}`, define:

```text
signal_s = abs(b_s) * (max(F) - min(F))
max_fit_residual_s = max(abs(delta_s(n) - (a_s + b_s * n)) for n in F)
residual_signal_ratio_s = max_fit_residual_s / signal_s
count_zero_ratio_s = abs(delta_s(0) - a_s) / signal_s
```

Zero signal makes all three ratios invalid and rejects the scenario. Holdout and checkpoint APE use
only marginal signal, so fixed guest or lane overhead cannot make a nonlinear relation appear
accurate. A candidate absolute model cost is first converted back through the sealed reference
ledger; this validates the complete absolute reconstruction rather than an uncorrected delta:

```text
observed_marginal_s(n) = delta_s(n) - a_s
predicted_relative_cost_s = model_absolute_cost_s + r_s
predicted_marginal_s(n) = n * predicted_relative_cost_s
APE_s(n) = abs(predicted_marginal_s(n) - observed_marginal_s(n))
           / abs(observed_marginal_s(n))
```

Pair target and control executions by repeat index before calculating `delta`. Define repeat
stability on per-event marginal observations:

```text
z_s(n, repeat) = (delta_s(n, repeat) - median(delta_s(0, *))) / n
repeat_spread_s = max_over_n(
    (max_over_repeat(z_s(n, repeat)) - min_over_repeat(z_s(n, repeat)))
    / abs(absolute_stateful_cost_s)
)
```

Use every nonzero fit, holdout, and checkpoint count. A zero absolute-cost denominator fails
closed.

All arithmetic used for fitting, reporting, and threshold decisions is `Decimal`; binary `float` is
not a canonical intermediate.

The campaign has no result-driven adaptive expansion. A failed gate remains a failed or unmeasured
scenario and requires a versioned follow-up design; it is not repaired after opening the result.

## Predeclared Model Comparison

Evaluate the accepted slopes without changing them:

```text
M_fixed:
    SLOAD  = one per-event constant
    SSTORE = one per-event constant

M_access:
    SLOAD  = warm_body[SLOAD] + cold_extra[SLOAD]
    SSTORE = warm_body[SSTORE] + cold_extra[SSTORE]

M_typed:
    SLOAD  = warm_body[SLOAD] + cold_extra[SLOAD]
    SSTORE = branch[noop, set, clear, reset, dirty_rewrite, restore_original]
             + cold_extra[SSTORE] where applicable

M_raw_gas_diagnostic:
    cost = alpha + beta * interpreter_raw_gas
```

Fit every frozen shape jointly on fit rows using the complete relative prediction
`model_absolute_cost_s + r_s`. Each scenario has its own nuisance intercept; the per-event
parameters are shared exactly as shown above. Zero/nonzero variants of the same branch do not
receive separate production parameters. The report calculates all four shapes before it opens the
holdout, checkpoint, or high-limb decisions. A simpler shape is eligible only if every required
scenario passes its own measurement gates and the model's maximum marginal-signal holdout,
checkpoint, and applicable high-limb APE are all at most `10%`. Choose the least complex eligible
shape in the fixed order `M_fixed`, `M_access`, `M_typed`. `M_raw_gas_diagnostic` is never
automatically promoted.

Cold increments remain per-opcode in this first experiment; equality across SLOAD and SSTORE is
reported as a diagnostic and is not imposed. A later state-side event model may share them only if a
separate reviewed experiment supports that constraint.

If no candidate passes, SLOAD and/or SSTORE remain unsupported in the operation registry. The raw
rows and rejection reasons are still sealed as useful evidence.

## Proposal Trace And Promotion Boundary

The current proposal trace exposes only opcode, interpreter raw gas, transaction/frame identity, and
a static-raw-gas model input for `SLOAD`/`SSTORE`. It cannot reliably reconstruct access warmth or
the original/current/new semantic class. Raw-gas buckets are useful diagnostics, not a semantic
source of truth.

Therefore this campaign does not edit the proposal trace or estimator. After the result is sealed, a
separate reviewed promotion design must choose one of these paths:

- a fixed per-event model, if `M_fixed` passes;
- a base plus independently emitted storage-access event, if `M_access` passes;
- typed opcode execution fields and separately owned persistent-state events, if only `M_typed`
  passes;
- no promotion, if none passes.

A typed promotion must capture warmth and original/current/new relationships at execution time. It
must not infer them from final raw gas. Persistent unique-dirty and final-trie events must be
derived after transaction outcome is known, not emitted speculatively by the opcode inspector.

## Result Artifact

Seal a new artifact without modifying historical derivations. It records:

- implementation revision and clean-tree status;
- SP1 SDK, guest ELF/VK, launcher, REVM, Osaka, and Unzen schedule identities;
- manifest and every generated `OpcodeLabInput` hash;
- target/control traces, raw gas, normalized `proverGas`, instruction count, and repeat index;
- signed target/control reference ledgers, sealed reference costs, relative slopes, and
  reconstructed absolute stateful costs;
- per-scenario fit, holdout, checkpoint, and gate decisions;
- all four model reports and the least-complex eligible result;
- an explicit ownership statement for included REVM journal work and excluded witness/trie work;
- `candidate_eligible = false` for the existing composite estimator.

## Blind Replay

Only after the controlled artifact and any later promotion artifact are sealed may Mainnet proposals
`23077` and `7857` be replayed. They remain `ad_hoc` diagnostics. Their coverage and residuals may
motivate another experiment but cannot change a storage coefficient, model shape, feature, or gate.

The frozen Hoodi and Mainnet integration smoke rows and the final 60-proposal corpus remain closed.

## Implementation Sequence After Approval

1. Fix the new campaign's host/guest Osaka identity and add a shared stateful DB/transaction
   builder.
2. Add the strongly typed storage input and fail-closed validation tests.
3. Add canonical target/control generation and trace verification for the frozen scenario matrix.
4. Add the independent stateful manifest, runner, model comparison, and seal/replay verifier.
5. Build the SP1 guest, run focused and complete experiment tests, and obtain independent review.
6. Run the formal controlled campaign and publish the sealed result.
7. Decide and review proposal-trace promotion only from the sealed controlled result.

## Exit Criteria

This design is complete when it produces reproducible evidence answering:

- whether SLOAD is adequately represented by one constant or needs a cold-access increment;
- whether SSTORE is adequately represented by one constant, needs access separation, or needs a
  semantic branch function;
- which measured cost belongs to stateful REVM execution and which work remains for the state/trie
  layer;
- whether either opcode is ready for a separately reviewed operation-registry promotion.
