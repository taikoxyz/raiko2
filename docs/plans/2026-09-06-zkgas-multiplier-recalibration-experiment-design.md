# ZKGas Multiplier Recalibration Experiment Design

## Status

Approved for implementation as an offline experiment design. This document defines how measurement
runs are made and reported. It does not authorize or implement a production zk gas schedule or
Boundless quote-model change.

The earlier SP1 opcode lab, coverage inventory, and workload damage documents remain useful component
designs. This document is the authoritative contract for a complete multiplier recalibration run.

## Goal

Produce one high-precision SP1-native zkGas cost model in normalized `proverGas` units. Its required
V1 components are proposal-startup fixed cost, per-block base cost, per-transaction base cost,
native-value-transfer cost, opcode multipliers, and precompile multipliers. Freeze every candidate
value before opening proposal results, then use a fixed Mainnet/Hoodi proposal corpus only as the
final validation of that frozen model.

Per-operation pricing is a fixed architecture decision, not a hypothesis reopened by this experiment.
The experiment calibrates the numeric cost of each measurable operation/scenario and controlled
non-opcode workload, emits candidate backend-native multiplier artifacts, and tests whether those
frozen tables predict real proposals at their explicitly reported coverage. It does not decide
whether one global EVM-gas multiplier should replace the per-operation schedule.

The experiment must answer two primary questions:

1. What marginal SP1 `proverGas` cost is attributable to each required opcode, precompile, and
   controlled fixed/base workload?
2. Without fitting or selecting any value on proposal rows, how accurately does the completely
   frozen SP1 cost model predict directly measured `proverGas` totals on Mainnet and Hoodi proposals?

The same runs also produce an independent, non-gating V1 sidecar report for the relationship between
SP1 total instruction count and `proverGas`. The report calls the former
`sp1_instruction_count` or a cycle proxy, never true SP1 zkcycles: it excludes syscall, deferred
syscall, and system-chip work that contributes to `proverGas`. The sidecar freezes its eligible keys,
model, and thresholds before proposal output is visible, fits only on controlled workloads, and uses
the proposal corpus only for validation. Its result cannot block or promote the `proverGas`
candidate.

The result is an SP1-native high-precision opcode/precompile multiplier table, an explicit
fixed/base-cost table, proposal-validation evidence at explicitly reported coverage, and the separate
SP1 instruction-count-to-`proverGas` bridge report. A developer
separately decides how to encode those backend-native results into a production alethia-reth/REVM
schedule. A later experiment may independently measure RISC0 opcode/user-cycle costs regardless of
the SP1 bridge conclusion. Only direct reuse of the frozen SP1 scalar transport requires
`bridge_conclusion = supported`. Production promotion and Boundless quote calibration remain separate
work.

## Non-Goals

This experiment does not:

- change the production zk gas multiplier table;
- change the production block zk gas limit;
- change Boundless quoting or any online prover path;
- add runtime SDK, ELF, image, fork, network, or model-version checks;
- choose the protocol integer encoding or production block-budget scale for backend-native costs;
- mix an SP1-native cost vector with the current intrinsic charge, spawn estimates, or 100M cap;
- automatically promote a candidate table or conversion model;
- automatically open or merge an alethia-reth change;
- submit SP1 or RISC0 proofs to a network prover;
- infer RISC0 costs from SP1 measurements without executing the same controlled cases in RISC0;
- execute or calibrate RISC0 proposal workload in V1;
- use already-observed RISC0 proposal cycles as an execution-free Boundless quote input;
- call SP1 instruction count true SP1 zkcycles or RISC0 user cycles;
- require SP1 bridge completeness or stability to seal or validate the V1 candidate;
- fit or validate a RISC0/Boundless bridge in V1;
- fit, calibrate, select, normalize, or repair any candidate value from proposal observations.

The current 100M per-block zk gas cap is not an experimental fitting constraint. It may be included
as report context, but synthetic generation and coefficient fitting must not reject or truncate a
case because it would exceed that cap.

## Fixed Experiment Contract

The experiment campaign has one immutable calibration run followed by one immutable validation run.
Together they bind four frozen inputs across one irreversible validation barrier:

Unless explicitly qualified otherwise, references below to proposal results or output in a
calibration or sealing barrier mean `final_validation` rows. The separately labeled
`integration_smoke` rows defined below may run earlier and never participate in calibration, bridge
fitting, candidate construction, or final validation.

1. **Implementation provenance**: one clean `implementation_revision` captured before controlled
   measurement, the Cargo-pinned alethia-reth revision, the exported Unzen schedule hash, guest
   ELF/VK hashes, and relevant SDK versions from that checkout. Candidate, bridge, and validation
   provenance all retain that same revision until the run is complete.
2. **Measurement backend**: SP1 normalized software `proverGas` is the sole V1 candidate and
   validation metric. SP1 total instruction count is recorded alongside it as a non-gating secondary
   bridge input. RISC0 is not executed for V1 measurements; its guest is rebuilt and identity-checked
   only because the shared Alethia revision update can change both guest artifacts.
3. **Controlled suite**: exact manifest, generated-case metadata, and fixture hashes used for opcode,
   precompile, proposal-startup, block-base, transaction-base, and native-transfer measurement. Every
   candidate value comes only from this suite. Optional witness/input and blob/KZG cases are
   diagnostic unless a later reviewed manifest explicitly makes them part of a new candidate.
4. **Proposal validation corpus**: an explicit, saved set of Mainnet and Hoodi proposal GuestInputs,
   identified by network, proposal ID, repository-relative fixture path, and SHA256. It supplies no
   coefficient, scale, feature choice, threshold, or model selection.

The runner must refuse to mix rows whose provenance does not match the relevant manifest. A different
SDK, dependency revision, ELF, schedule, or controlled manifest creates a new calibration ID and
candidate digest. A different candidate digest or proposal corpus creates a new validation ID; no
command overwrites or silently extends an existing report. Before any proposal result may be opened,
the calibration run separately seals (a) the complete `proverGas` candidate table, fixed/base costs,
prediction formula, coverage rules, and acceptance thresholds and (b) the sidecar bridge manifest,
controlled-only fit, thresholds, and digest. Neither seal depends on the other succeeding. A value
changed after that
barrier creates a new calibration ID and requires a newly versioned validation corpus of previously
unopened GuestInputs; the old proposal rows become regression-only evidence.

Generated experiment data does not redefine `implementation_revision`. From the start of controlled
measurement through final verification, `HEAD` remains that revision and no implementation,
dependency, controlled-manifest, or guest-artifact file may change. Candidate, corpus-manifest,
validation, and report outputs may exist only in their declared generated paths and remain
uncommitted until validation finishes. They are committed afterward as experiment evidence while
continuing to name the revision that actually executed them. This ordering prevents a corpus-data
commit from being mistaken for a different implementation during candidate/validation provenance
checks.

These checks protect the integrity of offline experimental data only. They are not copied into the
runtime or used to decide whether production may use an installed schedule.

## SP1 Units, Candidate Tables, And Final Validation

Use these exact controlled-measurement quantities:

- `p`: SP1 normalized `ExecutionReport::gas()` in `proverGas` units;
- `s`: SP1 `ExecutionReport::total_instruction_count()`, named `sp1_instruction_count` in artifacts;
  it is a secondary cycle proxy and is not RISC0 user cycles;
- `r(k)`: raw EVM gas for one controlled operation represented by measurement key `k`;
- `g_p(k)`: marginal `proverGas / operation` fitted only from controlled case `k`;
- `c_p(k) = g_p(k) / r(k)`: proving-gas multiplier in `proverGas / raw EVM gas`;
- `o_p(q)`: controlled proving-gas cost for a non-opcode workload dimension `q`;
- `g_s(k)` and `o_s(q)`: optional secondary marginal instruction-count observations for the same
  controlled workload identities. They remain in the sampling artifact and never enter V1 candidate
  acceptance, normalization, prediction, or validation.

The controlled manifest freezes `normalization_reference_key = "opcode:0x01"` (ADD) before any
measurement. The reference must pass every required case and the `proverGas` gates. Emit the
dimensionless candidate view without rounding:

```text
m_p(k) = c_p(k) / c_p(opcode:0x01)
```

Preserve raw `c_p` alongside the relative multiplier. ADD normalization makes the SP1 vector shape
reviewable; it does not define the production integer zkGas scale. If the reference key or a required
fixed/base-cost dependency is unavailable, the candidate cannot be sealed and proposals are not
executed.

The run manifest pins the SP1 SDK and records that `p` is returned by its normalized API. In SP1
6.3.0 that API performs the version-compatibility `raw_gas * 10 / 191` integer normalization. The
private raw-gas field is neither read nor inverted by this experiment. Do not call `cycle_tracker`
the SP1 total: it contains only explicitly marked regions. `p` is the only primary measured output.
Preserve `s`, syscall, memory, wall-time, and region tracker rows as secondary observations or
diagnostics without allowing them to veto a valid `proverGas` measurement.

A measurement key is an immutable event-match tuple: opcode or precompile identity plus only scenario
dimensions that the frozen proposal trace can resolve exactly. Its controlled-manifest entry declares
a nonempty `required_case_ids` set before measurement. Several measurement keys may map to one
production schedule key only when their event matches are disjoint; for example, a linked
`ChargeAttempt` can resolve CALL/CREATE `spawned=true|false`. Warm/cold state or other context absent
from the event schema cannot be guessed from a free-text synthetic scenario name.

The manifest validator derives a structured operation identity from all three sources: the parsed
production schedule key, the proposal `event_match`, and every referenced required or diagnostic
case. Their component and opcode/precompile address must be identical. Each case also declares an
execution basis: opcode cases use `interpreter_raw_gas`, while precompile cases use `native_gas`.
Every CALL/CREATE measurement key and case declares the same structured `spawned` boolean. The
controlled trace must prove that the generated case actually exercised the declared identity,
execution basis, and spawned context. The free-text `scenario` field is descriptive only and can
never establish identity, context, or basis.

The candidate artifacts contain every accepted `c_p(k)` and `m_p(k)` at full decimal precision in a
schedule-shaped map with explicit missing/rejected entries. Together with the accepted `o_p(q)`
values, these form the experiment's only V1 candidate. They are not rounded to the current
integer zkGas representation and do not choose a production block-budget scale.

Write the simultaneously observed `(s, p)` values, controlled `g_s/o_s` slopes, repeat ranges, and
workload identities to separately hashed controlled/proposal cycle-cost sample files. These files and
the bridge report are content-addressed independently from the candidate.

The controlled manifest freezes a nonempty ordered `T_bridge = bridge_key_ids` before measurement.
It must contain the ADD normalization reference, all four `Q_formula` fixed/base keys, at least one
additional opcode key, and at least one precompile key. Every member is selected by exact workload
identity, never by its observed result. Members are either declared opcode/precompile measurement
keys or one of the four required `Q_formula` keys; diagnostic-only overhead keys are excluded. For a
controlled operation or overhead key `t`, define same-unit marginals and the sidecar model:

```text
a_p(t) = g_p(t) for opcode/precompile keys, otherwise o_p(t)
a_s(t) = g_s(t) for opcode/precompile keys, otherwise o_s(t)
rho(t) = a_p(t) / a_s(t)
kappa_sp1 = median(rho(t) for t in T_bridge)
p_hat_bridge_controlled(t) = kappa_sp1 * a_s(t)
APE(predicted, observed) = abs(predicted - observed) / observed
```

For an even number of keys, the median is the exact arithmetic mean of the two middle sorted values.
The model is deliberately a through-origin scalar: V1 tests whether instruction count alone is a
useful cost proxy rather than adding features that can hide syscall/system-chip divergence. Report
every `rho(t)`, signed and absolute residual, MAPE, maximum absolute percentage error, and results by
operation/fixed-base/precompile stratum.

If any `T_bridge` member lacks finite positive `a_p/a_s`, emit `insufficient_data`. Otherwise label
the controlled result `stable_controlled` only when every included key has absolute percentage error
`<= 10%`; label it `not_stable_controlled` otherwise. Never drop a failed key or change the model,
eligibility, or threshold after seeing controlled or proposal output. Every bridge result remains
non-gating for the `proverGas` candidate.

Before reading any proposal result, seal the complete forward prediction formula:

```text
p_hat(j) = sum(execution_raw_evm_gas(e) * c_p(resolve_measurement_key(e)))
         + sum(controlled_feature(j, q) * o_p(q) for q in Q_formula)

APE_main(j) = abs(p_hat(j) - p(j)) / p(j)
```

`Q_formula` is exactly `[proposal_startup, block_base, tx_base, native_value_transfer]` in canonical
order; `controlled_feature(j, proposal_startup) = 1`. Every member and its transitive
dependencies must be accepted or the candidate cannot seal. Diagnostic overhead keys are never added
to the formula after results are visible. Every fixed cost and coefficient in these formulas comes
from the controlled suite. There is no
proposal-fitted intercept, residual model, global rescale, feature selection, calibration split, or
model selection. The proposal trace only supplies the counts and byte quantities consumed by the
already-frozen formula.

Execution and current-schedule charging are separate observations. Include executed work from
committed transactions, attempted transactions before reset, and system execution even when the
subsequent current-schedule charge returns `limit_exceeded` or `arithmetic_overflow`. Exclude only
work that never began, such as intrinsic/pre-validation failure and the unattempted tail after block
truncation. Keep missing or unmeasured keys in explicit coverage fields instead of assigning them a
current-schedule value or a proposal-fitted value.

Define operation-count coverage as measured executed opcode/precompile events divided by all executed
opcode/precompile events. Define execution-raw-gas coverage using the analogous sums of interpreter
opcode gas and native precompile gas. Report both per proposal and as ratio-of-sums per network; a
zero denominator is unavailable, not 100%, and prevents a positive classification. V1 declares no
arbitrary minimum coverage percentage. Its positive classification is therefore
`candidate_table_validated_at_reported_coverage`, not “all operation costs validated.” Emit it only
if every proposal has finite positive actual `p(j)`, finite `p_hat(j)`, and `APE_main(j) <= 0.10`,
and every coverage value is reported with a positive denominator. Per-proposal error, per-network and
combined MAPE, maximum error, and the 10% pass/fail decision all use this formula with 50-digit
`Decimal` arithmetic. MAPE is the arithmetic mean of proposal APE values, not a ratio of aggregate
gas totals. Otherwise emit `candidate_table_not_validated` with the exact failed rows and components.

Independently validate the already-sealed sidecar bridge on the same proposal rows:

```text
p_hat_bridge(j) = kappa_sp1 * sp1_instruction_count(j)
```

Proposal rows never change `T_bridge`, `kappa_sp1`, the through-origin model, or the 10% threshold.
When the controlled bridge is complete, report proposal MAPE, maximum absolute percentage error,
underprediction count, maximum underprediction, and network splits. Label the proposal bridge
`validated_on_proposals` only when every proposal has finite positive observed/predicted values and
absolute percentage error `<= 10%`; otherwise label it `not_validated_on_proposals`. If the controlled
bridge is `insufficient_data`, or any proposal lacks a finite positive instruction count, emit
`not_evaluable_on_proposals` with exact reasons. Report controlled and proposal statuses independently.
This sidecar label never changes the primary candidate classification.

Emit one final `bridge_conclusion` after both statuses are known:

```text
inconclusive   if controlled_status == insufficient_data
not_supported  if controlled_status == not_stable_controlled
inconclusive   if proposal_status == not_evaluable_on_proposals
supported      if controlled_status == stable_controlled
               and proposal_status == validated_on_proposals
not_supported  otherwise
```

The order above is precedence order. `APE` always uses the finite positive directly measured
`proverGas` value as `observed`; controlled rows use `a_p(t)` and proposal rows use `p(j)`. `MAPE` is
the exact arithmetic mean of the declared rows' APE values. `inconclusive` means the controlled
evidence was insufficient, or the controlled relationship was stable but proposal evaluation lacked
data. `not_stable_controlled` always produces `not_supported`, even when proposal instruction count is
missing. Only direct use of this scalar transport requires `supported`; independent RISC0 measurement,
backend-specific tables, and cross-backend comparison remain allowed for every conclusion. No bridge
conclusion changes the primary candidate.

A failed validation may guide the design of a new controlled case, but no proposal residual may alter
the current run. Any changed coefficient, feature, mapping, normalization, or threshold creates a new
run and must use a newly versioned corpus containing previously unopened GuestInputs before receiving
a positive final label. The fixed V1 60-row corpus is never reused to validate a revised candidate;
reuse is labeled regression-only evidence.

## Workload Decomposition

Proposal workload is validated in SP1 `proverGas` with this decomposition:

```text
predicted_sp1_prover_gas =
    startup_fixed_cost
  + block_count * block_base_cost
  + started_transaction_count * tx_base_cost
  + native_value_transfer_count * native_value_transfer_cost
  + sum(operation_raw_evm_gas_i * controlled_cost_per_raw_evm_gas_i)
```

`Q_formula` therefore contains the four required V1 non-operation identities
`proposal_startup`, `block_base`, `tx_base`, and `native_value_transfer`. The startup feature count is
exactly one per proposal guest invocation and is not added a second time. Opcode and precompile work
is represented only by the operation sum.

Their frozen subtraction DAG is:

```text
tx_base                -> []
native_value_transfer  -> [tx_base]
block_base              -> [tx_base, native_value_transfer]
proposal_startup        -> [block_base]
```

Transitive subtraction is deduplicated, so `tx_base` is removed exactly once from a block or startup
residual even though it is also a dependency of native transfer.

`tx_base` and `native_value_transfer` are deliberately distinct. One `tx_base` unit is counted at the
frozen `TransactionStart` boundary; its controlled coefficient is the common per-started-transaction
residual after modeled block, transfer, opcode, and precompile work is removed. It does not claim that
all of that work occurs before the event. `native_value_transfer` is an additional exclusive residual
for a non-create transaction whose recipient executes no code and whose positive native value is applied.
It excludes `tx_base`, block work, and all observed opcode/precompile work. The trace must emit this
classification from structured execution/state facts; calldata shape or a free-text scenario name is
not sufficient. Signer-recovery failures and transactions after the truncation boundary do not
silently receive either cost; any guest work they caused remains explicitly unmeasured until a later
controlled component models it.

`block_base` is the exclusive per-successful-block residual after transaction, native-transfer,
opcode, and precompile contributions are subtracted. It includes the controlled block-loop/header and
block-finalization work that remains under that definition; it must not absorb a variable component
that the trace declares separately. `proposal_startup` is not fitted as an ordinary operation slope.
It is the repeat-stable residual of a frozen minimal proposal panel after all block, transaction,
transfer, opcode, and precompile terms have been subtracted. It is never a proposal-corpus intercept.

This is a controlled-measurement and validation decomposition, not a new runtime metering formula.
Every term is calibrated before proposal execution. Proposal observations may test the sum but may
not fit, rescale, select, or repair any term.

The one-dimensional opcode multiplier table cannot represent every source of zkVM work. The
experiment therefore keeps non-opcode effects visible instead of forcing them into opcode
coefficients. At minimum, proposal validation records:

- proposal and block count;
- input, started, committed, attempted, and unattempted transaction counts;
- structured native-value-transfer count;
- opcode counts and raw EVM gas contribution by opcode;
- precompile calls, native gas, and input-size features when available;
- guest input and witness size features;
- blob or KZG-related features when present;
- observed SP1 `proverGas` and SP1 instruction count;
- the frozen-candidate prediction, explicit unmeasured-work coverage, and residuals.

The controlled suite contains paired fixtures for proposal startup, block base, transaction base,
native value transfer, and every other explicit non-opcode component admitted to the forward
formula. Witness/input bytes and nodes, blob payload bytes, and KZG work are collected as diagnostics
in V1; promoting any of them to a required formula term requires a reviewed new manifest and fresh
validation corpus. Physical features overlap: adding blocks, transactions, or transfers can add
serialization, state, and operation work. The manifest therefore freezes a residualization DAG before
measurement. Each overhead key declares
`subtract_keys`; target/control deltas record every induced operation and overhead-feature delta, then
subtract every already-accepted descendant cost exactly once before fitting the exclusive residual:

```text
delta_p_exclusive(q) = p_target - p_control
                     - sum(delta_operation_gas(k) * c_p(k))
                     - sum(delta_feature(t) * o_p(t) for t in subtract_closure(q))

delta_s_sample(q) = s_target - s_control
                  - sum(delta_operation_count(k) * g_s(k))
                  - sum(delta_feature(t) * o_s(t) for t in subtract_closure(q))
```

Only `delta_p_exclusive` and `o_p` participate in V1 candidate acceptance and prediction. The `s`
calculation is stored for the independently sealed V1 bridge when its dependencies are available;
failure leaves a marked secondary sample and may make the bridge `insufficient_data`, but it never
invalidates a valid `proverGas` component.

`subtract_closure(q)` is the deduplicated transitive closure, so a descendant reachable through
several paths is subtracted once. The DAG is acyclic, children are measured before parents, and every
changed child dimension must be either listed in `subtract_keys` or explicitly bundled into the
parent's exclusive unit while that child is omitted from `Q_formula`. Every transitive subtract child
of a required parent is itself
required and present in `Q_formula`; only a bundled child may remain outside the formula. A key whose
induced deltas cannot be completely accounted for is
`confounded_overhead` and cannot enter the formula. This prevents double-counting stdin/witness/blob
bytes or block/transaction execution. Blob bytes and KZG invocations remain distinct only if a
controlled fixture and this accounting prove their independent deltas; otherwise the manifest uses
one explicit bundled blob unit or leaves the unsupported dimension unmeasured.

Each overhead entry freezes `unit`, `formula_role = required|diagnostic`, `subtract_keys`,
`bundled_keys`, and one exact positive rational `bundled_ratio(child_per_parent)` for each bundled
child. Every target/control trace must enumerate its operation deltas and all changed
overhead features. A changed feature is valid only when it is the target, a transitive subtract key,
or a declared bundled key. Bundled keys cannot appear elsewhere in `Q_formula`. The manifest is
rejected for cycles, ambiguous ownership, or an ordering in which a parent is measured before a
subtracted child. Any nonzero induced operation delta that does not resolve to an accepted operation
key with the matching execution basis makes the overhead case `confounded_overhead`. These rules, not
a claim that overlapping raw features remain physically constant, define the non-overlapping
attribution basis.

For every bundled child, each controlled target/control delta and each proposal feature row must
satisfy the frozen ratio exactly in integer arithmetic. Equivalently, the parent is a compound feature
whose internal child composition is fixed by the manifest. A missing or different ratio is
`bundle_relation_mismatch`; that parent contributes no prediction and a required parent makes final
validation fail. The experiment never extrapolates a fixture-specific bundled ratio.

The candidate artifacts remain in backend-native high-precision units. This experiment does not round
them into the current alethia-reth integer schedule because the production block-budget scale,
intrinsic charge, spawn estimates, and cap have not yet been selected together.

## Measurement Layers

### 1. Active Schedule Export

The experiment reads the current Cargo-pinned
`alethia_reth_evm::zk_gas::unzen::UNZEN_ZK_GAS_SCHEDULE` through the existing xtask exporter. Python
must not contain another hand-maintained copy of the production table.

The exported schedule is the exact current-charge reconciliation oracle and coverage inventory. Its
normalized content hash is stored in the run manifest; it is not converted into a PGU table.

### 2. SP1 Synthetic Microbenchmarks

The primary opcode lane is the revm-backed `revm-opcode-lab`, not the experiment-only mini
interpreter. Direct precompile body measurements use `precompile-lab`. The mini interpreter remains
a fast smoke signal only.

For each supported scenario, generate a matched baseline and an adaptive count sweep. Variants must
keep setup, environment, calldata shape, and cleanup as constant as practical while changing the
target feature count.

The synthetic manifest freezes every measurement key's production-schedule identity, exact proposal
event match, nonempty required case set, and any explicitly diagnostic-only cases. A measurement key
enters the SP1 vector only when every required case completes and passes all isolation, signal, and
fit gates, and the resulting required-case coefficients differ by at most 5%. Its coefficient is the
exact arithmetic mean of those required coefficients. A missing, rejected, or confounded required
case makes the complete measurement key unmeasured. Diagnostic-only cases remain visible but never
define, rescue, or veto `c(k)`.

Reject the manifest before measurement when a referenced case, event match, and production schedule
key do not describe the same opcode or precompile address, or when their structured execution context
or basis disagrees. A valid case ID is not sufficient evidence of a valid association.

The final calibration manifest materializes these sets explicitly before results exist.
Schedule-driven include flags may generate a smoke suite, but they cannot remain as hidden expansion
rules in the frozen calibration manifest. Changing a required set, diagnostic assignment, or event
match creates a new manifest and run identity.

Proposal events may use a successful measurement key only through its exact frozen event match. No
match, more than one match, or missing linked context is unmeasured coverage. A successful case must
never be generalized to a failed scenario that the proposal event schema cannot distinguish.

Fit the primary marginal relationship and record the secondary one from the same runs:

```text
sp1_prover_gas = intercept_p + slope_p * target_feature_count
sp1_instruction_count = intercept_s + slope_s * target_feature_count
```

The sweep expands until `proverGas` has enough dynamic range for a stable fit or reaches a declared
case limit. Only `p` must pass the predeclared determinism, signal, slope-error, R2, and residual
gates for V1 candidate inclusion. Apply the same diagnostics to `s`, but store a failed secondary fit
as unavailable instead of rejecting a valid `p` result. Each result records sample counts, both
slopes and intercepts when available, fit diagnostics, observed ranges, and why either metric was
skipped or rejected. Low-quality primary fits remain visible and are not silently included in the
candidate table.

Every slope-based controlled case also has one frozen out-of-fit checkpoint. The predeclared mapping
from the maximum count in the first passing prefix to the checkpoint is
`4->8, 16->32, 64->128, 256->512, 1024->2048`. Run that larger count only after a prefix passes; it
never participates in fitting, prefix selection, normalization, or coefficient calculation. Repeat
the same isolation and target/control requirements and require:

```text
observed_checkpoint_delta_p = response_p(checkpoint_count) - response_p(0)
predicted_checkpoint_delta_p = slope_p * checkpoint_count
APE_checkpoint_p = abs(predicted_checkpoint_delta_p - observed_checkpoint_delta_p) /
                   observed_checkpoint_delta_p
APE_checkpoint_p <= 0.10
```

The observed checkpoint delta must be finite and positive. Using the delta prevents startup/intercept
cost from hiding a bad slope. An unavailable, confounded, or failed
primary checkpoint rejects that case without trying a later prefix. Apply the same checkpoint as a
non-gating diagnostic to the fitted instruction-count response; failure makes its secondary slope
unavailable. This rule also applies to every opcode, precompile, or non-opcode controlled cost derived
from a slope. The fixed `proposal_startup` residual is not slope-derived and is exempt.

State- and environment-dependent opcodes, CALL/CREATE families, halting behavior, and unsupported
entries use explicit scenarios or explicit unsupported classifications. Precompiles with
argument-dependent cost use deterministic input families and preserve the input dimensions in raw
results.

All SP1 runs use local execute mode. They never request a proof and never call a remote prover.

### 3. Controlled Non-Opcode Measurements

Run the controlled overhead fixtures through the production `sp1-shasta-proposal` guest without
access to corpus proposal observations. These are constructed synthetic GuestInputs, not Mainnet or
Hoodi proposal samples. Each fixture has a target and matched control, a predeclared unit, an adaptive
sweep, and an isolation trace proving that only the target plus its declared transitive subtract/
bundled closure changed. At minimum the
manifest separately covers proposal startup, block base, transaction base, and native value transfer.
Witness bytes/nodes, stdin or serialization bytes, blob payload bytes, and KZG invocations are V1
diagnostics. A required dimension that cannot be isolated is rejected and remains explicitly
unmeasured; it is never recovered through proposal regression.

Block, transaction, transfer, and any diagnostic variable-count dimension use matched count sweeps.
Their slope acceptance includes the same frozen out-of-fit checkpoint and 10% APE gate as opcode and
precompile slopes.
Proposal startup instead uses the fixed residual of a frozen minimal-proposal panel: subtract every
accepted variable contribution, require three identical `p` repeats per fixture and at most 5%
dispersion between the residuals of the predeclared panel members, then take their exact arithmetic
mean. A single proposal row or an intercept fitted on the final corpus is invalid.

The accepted primary output is `o_p(q)` plus its repeat/fit or fixed-residual evidence. Store `o_s(q)`
when its secondary diagnostics pass. Freeze the primary costs with the opcode/precompile table before
any proposal result is opened; keep the secondary rows in the separate cycle-cost sample artifact and
use them only through the independently sealed, non-gating bridge sidecar.

### 4. Fixed Proposal Feature Extraction

For every saved proposal GuestInput in the corpus, execute the same proposal workload through an
instrumented host/revm path to extract opcode, precompile, block, started-transaction, structured
native-value-transfer, witness, input, and blob features.

Feature extraction and backend measurement are joined by the SHA256 of the exact GuestInput bytes,
not merely by proposal ID. A proposal ID is human-readable provenance; the hash is the experiment
identity.

The extractor must account for every block and transaction in the proposal. It must report
unattributed work instead of dropping it when a feature is unavailable.

### 5. Fixed Proposal SP1 Execution

Execute each exact GuestInput locally with the SP1 proposal guest and record actual `proverGas` plus
the available instruction, syscall, cycle-tracker, and timing diagnostics. The measured SP1 guest
uses the same Alethia revision as the host trace, while the host observer feature remains disabled in
the guest path.

V1 does not execute the RISC0 guest. If the shared Alethia revision changes, build both guests and
record both artifact identities to prove the repository remains release-consistent; that build check
does not turn RISC0 into a measurement backend for this experiment.

The runner is finite and resumable. Completed rows are content-addressed by run identity and case
identity. On restart it verifies an existing row before skipping it; it does not append duplicate or
mixed-provenance observations.

### 6. Candidate Tables And Proposal-Only Validation

Construct high-precision SP1 proving-gas multipliers from accepted controlled fits. Add the required
controlled fixed/base costs, then seal their canonical digest before proposal execution. Retain full
precision and do not translate them to the production integer schedule representation in this
experiment. Write the secondary SP1 instruction-count observations, diagnostic overhead cost table,
and controlled bridge result to independently content-addressed sidecar artifacts; none is a
candidate dependency. Freeze the bridge manifest, controlled-only `kappa_sp1`, statuses, and digest
before proposal execution.

Apply the exact frozen forward formula to the proposal ledger without fitting any proposal value.
Report at least:

- signed and absolute error per proposal;
- MAPE and maximum absolute percentage error;
- underprediction count and maximum underprediction;
- results split by Mainnet and Hoodi;
- operation-count and execution-raw-gas coverage plus explicit unmeasured components;
- the frozen proving-gas prediction;
- the frozen bridge key set, controlled ratios/model/status, proposal-only bridge predictions and
  validation status, and final `bridge_conclusion`;
- the standalone diagnostic overhead cost table with every accepted/rejected witness/input/blob/KZG
  result and its fit evidence;
- the proposals and components responsible for the largest residuals;
- required/diagnostic case outcomes, unresolved event matches, coverage gaps, and coefficients
  excluded for poor fit or out-of-fit checkpoint quality;
- the exact candidate digest, frozen formula, coefficients, validation corpus identity, and final
  validation classification.

The report does not need to enforce zero underprediction. It must expose underprediction and the
chosen accuracy tradeoff so developers can decide whether the SP1-native tables are supported by the
corpus. The 10% validation budget never hides larger errors or automatically promotes a table.

## Fixed Proposal Corpus

The proposal corpus is defined by a checked-in manifest. Every row contains:

```text
network
proposal_id
fixture_path
guest_input_sha256
workload_id
notes
```

The manifest also has one top-level archive identity:

```text
archive_uri
archive_generation
archive_size_bytes
archive_sha256
```

`network` is descriptive and is used for per-network metrics; it does not select network-specific
SP1 coefficients. Every row has the single role `final_validation`; no proposal row is a fit,
calibration, normalization, feature-selection, or model-selection input.

Large GuestInput fixtures remain ignored locally and are packed into one deterministic archive. The
archive is published under a content-addressed `gs://` object name with create-only semantics. A
positive GCS generation, size, and SHA256 are recorded only after an exact-generation download
matches the local archive. A URI without that readback is `local_unpublished`, not frozen. A report
is invalid if the exact object generation cannot be retrieved, its bytes do not match, or an unpacked
fixture does not match its row identity.

The corpus is intentionally selected and finite. The runner does not discover recent proposals from
RPC, replace failures with new proposal IDs, or repurpose a validation proposal after observing its
result. `prepare-corpus` passes the fixed IDs to
`scripts/regression/stress_shasta_proposal.py` with `--proposal-ids`, `--discover-only`, and
`--proposal-out`; it does not substitute a different recent-proposal discovery path.

Task 2 may execute one temporary post-Unzen proposal per network solely to smoke-test the trace/SP1
integration. Freeze each row's `purpose = "integration_smoke"` before execution and require its
`(network, proposal_id)` to be outside the predetermined 60-row `final_validation` selection. Smoke
rows never enter the final corpus manifest, controlled calibration, candidate, bridge fit, or final
report, and they cannot later be relabeled `final_validation`.

## Artifact Layout

```text
experiments/opcode-gas/
  manifests/
    sp1-calibration-v1.toml
    proposals/<corpus-id>.json
  runs/<calibration-id>/
    experiment.json
    raw/
      controlled-prover-gas.jsonl
    samples/
      controlled-cycle-cost-samples.json
      controlled-cycle-cost-samples.sha256
    diagnostics/
      sp1-diagnostic-overheads.json
      sp1-diagnostic-overheads.sha256
    bridge/
      bridge-manifest.json
      controlled-bridge.json
      bridge-root.json
      bridge.sha256
    costs/
      sp1-opcode-precompile-costs.json
      sp1-controlled-overheads.json
    candidate/
      candidate-manifest.json
      sp1-native-multipliers.json
      controlled-overheads.json
      candidate.sha256
  validations/<validation-id>/
      candidate-ref.json
      bridge-ref.json
      proposal-manifest.json
      raw/
        proposal-features.jsonl
        sp1-proposal-runs.jsonl
      samples/
        proposal-cycle-cost-samples.json
        proposal-cycle-cost-samples.sha256
      proposal-validation.json
      bridge-validation.json
      report.json
      report.md
```

`candidate-manifest.json` is the canonical root of the candidate. It contains the SHA256 and schema
identity of every normalized primary raw and cost file, the accepted/rejected status of every
controlled key, `Q_formula`, the residualization DAG, normalization anchor, exact prediction formula,
thresholds, the frozen prefix-to-checkpoint mapping, and review-only provenance. The hashed primary
artifacts contain the checkpoint count, observed and predicted delta, APE, and status for every
candidate-referenced slope-derived cost. Diagnostic slope costs store equivalent checkpoint evidence
under their independent artifact digest. `candidate.sha256` is the SHA256 of the canonical compact
bytes of that manifest, so every formula input is transitively covered. A formula-referenced key
without an accepted artifact, or an accepted slope-derived cost without a passing checkpoint,
prevents sealing.

`controlled-prover-gas.jsonl` is a primary-only projection and contains no instruction-count field.
The controlled and proposal cycle-cost sample files each have their own digest and contain paired
`sp1_instruction_count`/`proverGas` observations plus workload identities and diagnostics. They are
not referenced by `candidate-manifest.json`; missing secondary rows or any bridge conclusion
cannot change the V1 candidate digest or status. Proposal samples remain under the immutable
validation ID and never write back into the calibration directory.

`bridge-manifest.json` freezes `T_bridge`, the through-origin median model, 10% controlled/proposal
thresholds, strata, and missing-data rules before measurement. `controlled-bridge.json` stores either
the controlled fit and diagnostics or `insufficient_data`. `bridge-root.json` is the canonical root
containing the manifest, controlled-sample, and controlled-result hashes plus their schema identities;
`bridge.sha256` is the SHA256 of that root's canonical compact bytes. `bridge-ref.json` binds that
digest before proposal output is visible. Neither bridge manifest nor bridge result is referenced by
`candidate-manifest.json`.

`bridge-validation.json` stores every proposal prediction/APE, the controlled and proposal statuses,
and the final `supported|inconclusive|not_supported` conclusion. It references the sealed bridge root
and proposal sample digest but never changes either one.

`sp1-diagnostic-overheads.json` contains full-precision `o_p/o_s`, fit evidence, sample identities,
and accepted/rejected reasons for every witness/input/blob/KZG diagnostic case. Its independent digest
is not a candidate or bridge dependency unless a future reviewed experiment explicitly declares it.

Every row carries two different identities. `workload_id` identifies backend-independent logical
work; `execution_row_id` identifies one backend execution of that work. A controlled fixture first
serializes a canonical `workload_spec` containing its schema version, measurement/overhead key,
case ID, target count, target/control lane, complete state/environment/input parameters, and expected
operation/feature deltas. It excludes backend, calibration/validation ID, repeat index, SDK/ELF,
backend input encoding, and results. Define:

```text
controlled_workload_id = sha256(canonical_json({
  "kind": "controlled",
  "workload_spec": workload_spec
}))
proposal_workload_id = sha256(canonical_json({
  "guest_input_sha256": guest_input_sha256,
  "kind": "proposal"
}))

controlled_execution_row_id = sha256(canonical_json({
  "backend": backend,
  "backend_input_sha256": case_input_sha256,
  "kind": "controlled_execution",
  "repeat_index": repeat_index,
  "run_id": calibration_id,
  "workload_id": controlled_workload_id
}))
proposal_execution_row_id = sha256(canonical_json({
  "backend": backend,
  "backend_input_sha256": guest_input_sha256,
  "kind": "proposal_execution",
  "repeat_index": repeat_index,
  "run_id": validation_id,
  "workload_id": proposal_workload_id
}))
```

Every persisted execution row stores its recomputed `workload_id` and `execution_row_id`; controlled
rows also store the canonical `workload_spec`. Reject a row whose stored identity does not match its
canonical source fields. A future cross-backend comparison joins on `workload_id`, never
`execution_row_id`, a free-text case label, or proposal ID alone.

The candidate is content-addressed and immutable before validation. `candidate-ref.json` contains the
exact candidate digest, and validation commands verify the manifest plus every referenced file before
writing only below their validation directory. The
checked-in fixed report is self-contained enough to review without rerunning the SP1 guest. It
includes provenance, fixture/archive identities, controlled fit evidence, the `proverGas` candidate
table and fixed/base costs, proposal-level results, exact validation errors, and separately labeled
bridge/diagnostic results.

Every candidate JSON carries `review_only = true` and `production_write = false`. No experiment
command accepts a production schedule, Boundless configuration, guest artifact, image, or deployment
path as an output target.

Large transient launcher output and build artifacts remain outside Git. The committed raw JSONL is
the compact normalized observation stream, not terminal logs or generated binaries.

Calibration and validation IDs are content-derived or otherwise collision-resistant. Generating into
an existing directory is rejected unless an explicit resume proves that all immutable inputs match.

## Proposed Stable Workflow

The finished tooling exposes separate, composable steps rather than one command with hidden
side-effects:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-calibration ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-controlled --backend sp1 ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-controlled-costs ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py build-candidate ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py build-sp1-bridge ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-corpus ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py publish-corpus ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-validation ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py trace-proposals ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-proposals --backend sp1 ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py validate-proposals ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py validate-sp1-bridge ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py report ...
```

Exact flags are defined in the implementation plan. Controlled steps read one immutable
`experiment.json`. Validation steps read only the sealed candidate digest, independently sealed bridge
digest, and immutable proposal manifest; they fail on missing, mismatched, duplicate, or
writable-backreference inputs. An `insufficient_data` bridge is still a valid sealed sidecar and does
not prevent primary proposal validation.

Python commands use the user-managed `~/.venv`. The implementation must not create another virtual
environment or install packages implicitly.

## Failure And Resume Semantics

- Invalid controlled or proposal fixtures fail before measurement.
- Guest execution failures produce explicit failed rows and a non-zero command exit.
- NaN, infinity, malformed metrics, duplicate case identities, and numeric overflow reject report
  generation.
- A partial run remains resumable, but it cannot be reported as complete.
- Resume validates environment, manifest, candidate and bridge digests, workload identities, fixture
  hashes, and existing result schemas before reuse.
- Report generation lists missing and failed cases and refuses a final status unless the declared
  corpus is complete.
- External RPC availability is not required after the proposal fixtures have been saved. GCS is
  required only to publish or retrieve the exact frozen archive generation.

## Production Boundary And Promotion

Experiment calibration, validation, and report commands never modify the production schedule,
runtime configuration, or
Boundless configuration. Task 2's upstream, feature-gated Alethia observer is offline instrumentation
and must not change metering behavior when disabled. Guest artifacts change only through the normal
reviewed `just build-guest` workflow when the shared pinned Alethia revision changes. The experiment
emits a high-precision SP1-native `proverGas` multiplier table, fixed/base costs, and validation
evidence, not a production table write. It also emits a separate non-gating cycle-cost sampling
artifact, a frozen SP1 instruction-count/`proverGas` bridge report, and a diagnostic overhead cost
table. None of those sidecars changes the candidate or its validation result.

There is deliberately no automatic promotion command. If the report supports an update, a developer
chooses the production integer scale and recalibrates intrinsic, spawn, and cap values consistently in
a separate design. A separate reviewed change may then update the Alethia schedule. Changing the
Boundless quote model first requires its own RISC0-native calibration evidence.

Deployment owners decide when a new schedule is suitable for a particular release. Production code
does not inspect this experiment's run ID, SDK version, ELF hash, proposal corpus, or report before
using the schedule compiled into that release.

## Verification

The implementation is complete when:

- the active schedule export round-trips without a Python pricing-table copy;
- the full inventory has no unclassified opcode or precompile entries;
- controlled generation is deterministic for a fixed manifest;
- every measurement key freezes a nonempty required case set and an unambiguous proposal event match;
- every required/diagnostic case, event match, and production schedule key has the same structured
  component, opcode/address, execution context, and raw-gas basis;
- one failed required case excludes the complete measurement key and moves matching proposal work to
  unmeasured coverage;
- `Q_formula` contains exactly proposal startup, block base, started-transaction base, and native
  value transfer, and the trace derives the latter two from structured execution events;
- startup is accepted only as a repeat-stable minimal-panel residual, never as an ordinary operation
  slope or proposal-corpus intercept;
- every accepted opcode, precompile, and required overhead key passes the `proverGas` gates;
- every slope-derived controlled cost passes its frozen out-of-fit checkpoint at APE `<= 0.10`, and
  that checkpoint never enters its fit;
- failed or missing instruction-count diagnostics are preserved in the secondary sample and bridge
  artifacts, produce `insufficient_data` or `not_evaluable_on_proposals` when required, and do not
  change primary candidate acceptance;
- every required overhead residual subtracts its deduplicated transitive closure exactly once, every
  induced operation resolves to an accepted key, and every bundled ratio matches in controlled and
  proposal rows;
- the candidate contains only the SP1-native `proverGas` multiplier table and required controlled
  fixed/base costs under one canonical root manifest and immutable digest;
- the exact bridge key set, through-origin equal-key median model, 10% controlled/proposal thresholds,
  controlled result, and digest are frozen before proposal output is visible;
- paired instruction-count/`proverGas` observations and bridge results are stored under separate
  digests, with no candidate dependency;
- every bridge APE uses `abs(predicted-observed)/observed`, and the final bridge conclusion follows
  the frozen `supported|inconclusive|not_supported` truth table;
- backend-independent `workload_id` and backend-execution-specific `execution_row_id` are distinct,
  and future cross-backend joins use only the former;
- the candidate is frozen before proposal output is readable and validation cannot write it back;
- a run cannot mix SDK, ELF, schedule, manifest, or fixture provenance;
- resume does not duplicate completed cases;
- SP1 execution is local execute-only, and V1 performs no RISC0 proposal execution;
- fixed Mainnet and Hoodi trace/SP1 rows join by GuestInput hash and public output;
- the report recomputes the `proverGas` prediction only from the frozen candidate and proposal ledger;
- the report reproduces every primary proposal-validation metric from sealed normalized
  observations, reproduces the separate raw paired sampling artifact, and applies the sealed bridge
  formula to proposals without refitting or selection;
- the diagnostic overhead cost table reports witness/input/blob/KZG observations and exclusion
  reasons without adding those dimensions to `Q_formula`, the candidate, or the bridge;
- executed operation work remains in the prediction after a later charge failure, while intrinsic/
  pre-validation failures and unattempted work enter neither prediction;
- no proposal row participates in fitting, normalization, feature selection, model selection, or
  candidate repair;
- operation-count and execution-raw-gas coverage are reported per proposal and network, and the final
  label claims only frozen-candidate accuracy at that reported coverage;
- missing, modified, failed, or duplicate fixtures prevent a final report;
- no experiment command modifies the production table or runtime configuration;
- tests cover deterministic artifacts, provenance mismatch, resume, report recomputation, candidate
  immutability, proposal-only validation, and incomplete-corpus failures.

## Future Extensions

- Run identical controlled fixture identities in RISC0 execute mode when an independent RISC0 cost
  vector is needed, regardless of the SP1 bridge conclusion. Record RISC0-native user-cycle costs
  independently; SP1 success is not evidence that RISC0 shares the same fixed or variable overhead.
- Reuse the frozen SP1 scalar for direct transport only when `bridge_conclusion = supported`.
  Otherwise compare backend-specific tables or a separately justified cross-backend envelope.
- Validate any frozen RISC0/Boundless quote model only after RISC0-native controlled measurements and
  bridge assumptions have been sealed. If transport is unstable, retain backend-specific tables;
  selecting a per-key maximum remains a separate policy.
- Choose and version a protocol conversion scale, then jointly recalibrate intrinsic, spawn, and cap
  values before proposing an integer zkGas schedule.
- Add multi-dimensional state, memory, CALL/CREATE, and precompile scenarios if the production
  schedule evolves beyond one multiplier per entry.
- Add more fixed proposal corpora as new immutable runs rather than changing historical reports.
