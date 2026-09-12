# SP1 ZKGas Multiplier Recalibration Implementation Plan

## Status

Ready for implementation. This is the authoritative plan for the offline V1 experiment. V1 removes
every proposal-derived normalization, coefficient, residual fit, feature-selection, and
model-selection path. It calibrates one SP1 `proverGas` candidate on controlled fixtures:
opcode/precompile multipliers plus proposal-startup, block-base, transaction-base, and
native-value-transfer costs. It seals that candidate and uses proposals only for final forward
validation. SP1 instruction count is sampled alongside `proverGas` and evaluated by an independently
sealed, non-gating V1 bridge sidecar. Production integer encoding, cross-backend/RISC0 bridge
validation, and table installation remain separate work.

> **For agentic workers:** Use `superpowers:executing-plans` for this plan and
> `superpowers:test-driven-development` for each behavioral change. Task 2 changes consensus
> metering code even though the observer is opt-in, so it also requires an independent adversarial
> review and an independent behavioral verification pass.

**Goal:** Reuse the existing experiment foundation to produce a reproducible SP1-native `proverGas`
multiplier table plus explicit startup, block, transaction, and native-transfer costs, then validate
the completely frozen candidate on one Mainnet/Hoodi proposal corpus while independently fitting and
validating a non-gating SP1 instruction-count-to-`proverGas` sidecar.

**Architecture:** First run isolated opcode/precompile and non-opcode controlled suites, using
normalized SP1 `proverGas` as the sole V1 candidate metric. Build one schedule-shaped native and
ADD-normalized multiplier table plus four required fixed/base costs, then seal a content digest. SP1
total instruction count is captured from the same executions. Before proposal output is visible,
freeze an independent through-origin median bridge, eligibility, and 10% thresholds; it never gates
the candidate. Only after sealing both roots, measure each proposal with the normal SP1 guest and trace the
identical GuestInput in a separate host-native pass; validation applies the frozen `proverGas`
formula and separately validates the frozen bridge without fitting or selecting any proposal-derived
value. A later experiment may measure RISC0 costs independently under every bridge conclusion; only
direct reuse of the SP1 scalar transport requires `bridge_conclusion = supported`.

**Tech stack:** Rust/revm for authoritative tracing, the existing `guest-launcher`, Python 3.11
standard library via `~/.venv`, local SP1 execute mode, GCS generation-pinned corpus archives, and
JSON/JSONL/Markdown artifacts.

**Spec:**
`docs/plans/2026-09-06-zkgas-multiplier-recalibration-experiment-design.md`

## Global Constraints

- SP1 normalized `ExecutionReport::gas()` is the sole V1 candidate and validation metric.
- `Q_formula` is exactly `proposal_startup`, `block_base`, `tx_base`, and
  `native_value_transfer`; opcode and precompile terms remain separate.
- The V1 SP1 bridge freezes its exact key set, through-origin median model, and 10% thresholds before
  proposal output; it has an independent digest and status.
- SP1 bridge inputs and results cannot change candidate acceptance, digest, prediction, or validation.
- V1 performs no RISC0 proposal execution or cross-backend bridge fit.
- Proposal rows validate a sealed candidate only; they never fit, rescale, select, or repair it.
- Every slope-derived controlled cost must pass the frozen, non-fitting out-of-fit checkpoint at APE
  `<= 0.10`; fixed `proposal_startup` is exempt.
- Experiment commands never modify the production schedule, runtime, Boundless configuration, or
  generated guest artifacts.
- The measured SP1 guest never enables the host-only Alethia observer.

Unless explicitly qualified otherwise, references below to proposal results or output in a
calibration or sealing barrier mean `final_validation` rows. The separately labeled
`integration_smoke` rows may run earlier and never participate in calibration, bridge fitting,
candidate construction, or final validation.

---

## Existing Foundation To Reuse

Do not rebuild these components:

- `xtask export-unzen-zk-gas-schedule` and its Cargo-pinned Alethia source;
- `experiments/opcode-gas/opcode_gas.py` generation, batching, fitting, damage, and inventory helpers;
- `sp1-revm-opcode-lab`, `sp1-precompile-lab`, and their existing fixture templates;
- `guest-launcher` local SP1 execute mode and `Sp1ExecutionMetadata`;
- the existing opcode experiment test suite.

The current `[0, 1, 2, 4]` variants remain a smoke prefix, not accepted calibration data by default.
State/environment-dependent opcodes, CALL/CREATE cases without an isolated scenario, halting, CLZ,
p256, and any measurement key with an incomplete or inconsistent required case set are omitted from
the candidate table and remain explicit coverage gaps. Their current schedule values are preserved
only as trace metadata, never relabeled as measured costs.

## Fixed Measurement And Modeling Contract

### Units And Identities

Use these names consistently in code and artifacts:

- `x`: synthetic target operation count;
- `p`: directly observed SP1 `ExecutionReport::gas()`, exposed as `extra_data.sp1.gas`, in
  `proverGas` units;
- `s`: SP1 `ExecutionReport::total_instruction_count()`, exposed as
  `extra_data.sp1.total_instruction_count`, serialized as `sp1_instruction_count`, and treated as a
  secondary cycle proxy rather than RISC0 user cycles;
- `r`: raw EVM gas charged for one synthetic target operation;
- `g_p(k)` and `g_s(k)`: controlled marginal `proverGas / operation` and secondary
  `SP1 instruction count / operation`;
- `c_p(k) = g_p(k)/r(k)`: the V1 backend-native proving-gas multiplier per raw EVM gas;
- `m_p(k)`: the corresponding dimensionless multiplier normalized to
  `normalization_reference_key = "opcode:0x01"`;
- `o_p(q)`: controlled proving-gas cost per declared non-opcode unit `q`;
- `c_s(k) = g_s(k)/r(k)` and `o_s(q)`: optional secondary instruction-count slopes stored only in
  the cycle-cost sampling artifact;
- `q_current(e)`: current-schedule zkGas charge for one frozen ledger event `e`, used only for
  exact trace reconciliation and coverage;
- `fixture_sha256`: SHA256 of the saved JSON file bytes;
- `guest_input_sha256`: SHA256 of `bincode::serialize(GuestInput)`, which is the exact byte vector
  inserted into the first proposal SP1 stdin buffer;
- `case_input_sha256`: SHA256 of the bincode bytes inserted into the first synthetic-lab SP1 stdin
  buffer;
- `workload_id`: backend-independent logical workload identity; it excludes run ID, backend,
  repeat, SDK/ELF, backend input encoding, and measured results;
- `execution_row_id`: identity of one backend execution; it includes `workload_id`, run ID, backend,
  repeat, and the backend-specific input hash.

`p` is the sole V1 candidate and validation target. `s` is captured from the same executions for the
independently sealed, non-gating V1 bridge sidecar and cannot reject, rescue, normalize, or validate
a candidate value. The run
identity records the exact SP1 SDK version and the fact
that `p` comes from the normalized `ExecutionReport::gas()` API; for SP1 6.3.0 this API applies the
SDK's version-compatibility `raw_gas * 10 / 191` integer normalization. The experiment never reads,
inverts, or republishes the private raw-gas field. SP1 `cycle_tracker` rows contain only explicitly
marked regions and are diagnostics. SP1 syscall counts, memory statistics, and wall time are also
diagnostics only.

The existing RISC0 corpus files provide only the
fixed proposal-ID pool and pre-measurement block/zkGas stratification fields; their historical
`actual_mcycles` values are ignored. Every selected GuestInput is re-executed with the final SP1 guest.
V1 performs no RISC0 proposal execution.

The controlled manifest freezes ADD (`opcode:0x01`) as the dimensionless normalization reference.
The ADD key must pass the primary `proverGas` gates. Emit `m_p(k)=c_p(k)/c_p(ADD)` without rounding
while preserving raw `c_p`. This reference exposes the SP1 vector shape and is not the protocol
integer scale. Do not combine these values with the
current intrinsic charge, spawn estimates, failsafe value, or 100M block cap.

Persist paired controlled and proposal `(sp1_instruction_count, proverGas)` observations, workload
identity, and `c_s/o_s` diagnostics under digests independent from the candidate. The controlled
manifest freezes a nonempty ordered `bridge_key_ids` before measurement. It must contain ADD, all
four `Q_formula` keys, at least one additional opcode, and at least one precompile. Every entry must
be a declared opcode/precompile measurement key or one of the four required formula keys;
diagnostic-only overheads are excluded. Define:

```text
a_p(t) = g_p(t) for opcode/precompile keys, otherwise o_p(t)
a_s(t) = g_s(t) for opcode/precompile keys, otherwise o_s(t)
rho(t) = a_p(t) / a_s(t)
kappa_sp1 = median(rho(t) for t in bridge_key_ids)
p_hat_bridge_controlled(t) = kappa_sp1 * a_s(t)
p_hat_bridge_proposal(j) = kappa_sp1 * sp1_instruction_count(j)
```

Use the exact arithmetic mean of the two middle sorted ratios for an even key count. This is a
through-origin scalar by design: adding an intercept or workload features would hide the
syscall/system-chip gap V1 is measuring. Every key is selected by frozen identity; never drop one
after seeing its result. Missing/non-positive data produces `insufficient_data`. Otherwise emit
`stable_controlled` only when every controlled absolute percentage error is `<= 10%`, and
`not_stable_controlled` otherwise. For any finite controlled `kappa_sp1`, separately emit
`validated_on_proposals` only when every proposal absolute percentage error is `<= 10%`, and
`not_validated_on_proposals` otherwise. Missing/non-positive proposal instruction count produces
`not_evaluable_on_proposals`. Controlled or proposal failure never changes the primary candidate
status.

Use this exact error definition for every controlled and proposal bridge row:

```text
APE(predicted, observed) = abs(predicted - observed) / observed
```

`observed` must be finite and positive. Controlled rows use `a_p(t)`; proposal rows use directly
measured `p(j)`. `MAPE` is the exact arithmetic mean of the declared rows' APE values. After both
statuses are available, emit one final bridge conclusion using this precedence-ordered truth table:

```text
inconclusive   if controlled_status == insufficient_data
not_supported  if controlled_status == not_stable_controlled
inconclusive   if proposal_status == not_evaluable_on_proposals
supported      if controlled_status == stable_controlled
               and proposal_status == validated_on_proposals
not_supported  otherwise
```

`not_stable_controlled` therefore remains `not_supported` even when proposal instruction count is
missing. Only direct adoption of this scalar transport requires `supported`; independent RISC0
measurement, backend-specific tables, and cross-backend comparison are allowed for every conclusion.
No conclusion blocks, rescues, or modifies the primary SP1 candidate.

Use `decimal.localcontext(prec=50, rounding=ROUND_HALF_EVEN)` for slopes, coefficients, secondary
sample diagnostics, predictions, and metrics. Parse integer observations directly into `Decimal`; never route
canonical values through binary `float`. Serialize canonical numeric fields as normalized decimal
strings, reject non-finite values, and compare every quality threshold in `Decimal` arithmetic.

### Complete Schedule Identity

Extend the schedule export to contain:

```text
schema_version
fork
alethia_revision
block_limit
tx_intrinsic_zk_gas
failsafe_multiplier
spawn_estimates.{call,callcode,delegatecall,staticcall,create,create2}
opcodes[0..255].{opcode,multiplier,source=explicit|failsafe}
precompiles[].{address,multiplier}
unlisted_precompile_semantics=failsafe
```

The normalized schedule hash is SHA256 over canonical compact JSON with lexicographically sorted
object keys and fixed array order. Export all 256 opcode entries, including `0`, and preserve the
distinction between an explicit value and the `65535` failsafe. Python must not contain another
production schedule table.

### Frozen Workload Ledger

Alethia owns one feature-gated `execution-observer` interface. The interface is observational and
infallible:

```rust
pub trait ExecutionObserver: Send + Sync {
    fn on_event(&self, event: ExecutionEvent);
}

pub type SharedExecutionObserver = Arc<dyn ExecutionObserver>;
```

The feature is disabled by default. Normal constructors keep the existing no-observer path;
feature-gated `*_with_observer` constructors accept `Option<SharedExecutionObserver>`. Observer
callbacks return `()` and cannot alter an execution result. A callback panic fails the host tracing
process; it is never caught and converted into an execution decision.

`operation_id` is unique and monotonically increasing within a block. Every opcode/precompile
`ChargeAttempt` must reference exactly one earlier `OperationExecuted`; transaction intrinsic charges
have no `operation_id`. The observer may distinguish checked-arithmetic overflow from an ordinary
budget exceedance in its event, but the existing consensus-facing execution result remains
`ZkGasOutcome::LimitExceeded` in both cases.

`ExecutionEvent` is an owned, serializable enum with this minimum schema:

```text
BlockStart { block_index, block_number, expected_difficulty, block_limit, recovered_tx_count }
PhaseStart { phase=pre_execution_system|transactions }
PhaseEnd { phase }
TransactionStart {
  tx_index, tx_hash, is_anchor,
  execution_class=native_value_transfer|contract_call|contract_create|no_code_no_value|other
}
OperationExecuted {
  operation_id, phase, tx_index?,
  component=opcode{opcode,interpreter_raw_gas}|
            precompile{address,native_gas}
}
ChargeAttempt {
  operation_id?,
  phase, tx_index?,
  component=tx_intrinsic{amount}|
            opcode{opcode,spawned}|
            precompile{address},
  charge_raw_gas?,
  raw_gas_source=intrinsic_fixed|interpreter_delta|spawn_estimate|precompile_native,
  multiplier?,
  requested_current_zkgas?,
  outcome=applied|limit_exceeded|arithmetic_overflow
}
TransactionEnd {
  tx_index,
  disposition=committed_success|committed_revert|filtered_zero_signer|
              filtered_invalid|filtered_block_gas_limit|filtered_zkgas_limit|fatal,
  observed_current_zkgas,
  committed_current_zkgas
}
BlockStop { reason=complete|zk_gas_truncated|fatal, first_unattempted_tx_index? }
BlockEnd { finalized_current_zkgas }
```

Raiko2 separately emits derivation events from `build_derived_block` for each manifest transaction:

```text
DerivationTransaction { manifest_index, tx_hash, outcome=recovered|signer_recovery_failed }
```

This is the only raiko2-owned admission observation: signer recovery happens before Alethia receives
the recovered candidate. Raiko2 must not copy Alethia's zero-signer, invalid-transaction, block-gas,
zkGas-limit, commit, or reset state machine.

The host collector maintains transaction buffers with these exact rules:

1. Pre-execution system-call execution and charge attempts go directly to the `system` ledger and
   never to difficulty.
2. `TransactionStart` opens an empty buffer. An `OperationExecuted` event is appended after the
   opcode/precompile body executes and before its current-schedule charge result is known. The
   event records only execution facts known at that point. Any later corresponding `ChargeAttempt`
   uses the same `operation_id`; an intrinsic charge has no operation.
3. `committed_success` and EVM `committed_revert` move the buffer to `committed`; both contribute to
   `header.difficulty` because both transactions committed at the block level.
4. Every `filtered_*` disposition moves the buffer to `attempted`; Alethia then resets its in-flight
   meter. Already-emitted operation events remain attempted work even when their charge returned
   `limit_exceeded` or `arithmetic_overflow`. Intrinsic or pre-execution validation failure can have
   no operation events because EVM execution never began.
5. `zk_gas_truncated` records the first unattempted transaction. The tail is classified as
   `unattempted_after_truncation` and contributes no operation work because the guest did not execute
   it.
6. A fatal block/proposal discards the complete trace from final validation and writes an explicit failed
   row. Partial traces are never accepted.

`started_transaction_count` is the number of `TransactionStart` events. A
`native_value_transfer_count` increment requires `execution_class=native_value_transfer`, positive
native value, no recipient code execution, and `committed_success`. The executor derives the class
from structured transaction/state facts at its existing execution boundary. A free-text fixture name,
empty calldata alone, or a transaction that never reaches `TransactionStart` cannot create this
feature. These definitions are shared by controlled traces and proposal extraction.

For every successful block, the non-vacuous oracle is:

```text
sum(applied current-schedule ChargeAttempt values in committed buffers)
    == finalized_current_zkgas
    == u64::try_from(block.header.difficulty), rejecting out-of-range values
block.header.difficulty > 0
```

All-zero equality fails corpus validation. Attempted and system ledgers are not added to difficulty,
but remain in the frozen workload because the SP1 guest performed that work.

For a frozen `ChargeAttempt` event `a` under the active schedule:

```text
q_current(a) = a.component.amount
    if a.component = tx_intrinsic
q_current(a) = a.charge_raw_gas * a.multiplier
    if a.component = opcode or precompile

C_current(p) = sum q_current(a) over applied committed ChargeAttempt events
A_current(p) = sum q_current(a) over attempted ChargeAttempt events, including a limit trigger
S_current(p) = sum q_current(a) over pre-execution system ChargeAttempt events
```

For an opcode `OperationExecuted`, `interpreter_raw_gas` is the interpreter's actual step gas. For a
precompile it is `native_gas`. CALL/CREATE execution is emitted at `step_end` before dispatch has
resolved `spawned`; the later linked `ChargeAttempt` records that decision. The charge fields record
the independent current-schedule basis: ordinary opcodes use `interpreter_delta`, spawned
CALL/CREATE uses the fixed `spawn_estimate`, and precompiles use `precompile_native`. Its `multiplier`
is the effective value selected by Alethia, including precompile fallback. The collector may verify
these fields against the exported schedule but must not recompute a different selection rule.

Every multiplication is checked as `u64`; overflow invalidates the report. The current ledger proves
that feature extraction follows the production transaction/filter/reset semantics and records which
accepted candidate keys occur in each validation network. It is not converted into a prediction by
mixing current intrinsic/spawn/cap values with measured costs. Cycle and proving-gas execution terms
derive from `OperationExecuted`, never from a charge outcome. The exact committed, attempted, system,
and unattempted classifications are frozen before final validation.

### Controlled Acceptance And SP1 Candidate Tables

The controlled manifest defines the candidate boundary explicitly. Add
`normalization_reference_key = "opcode:0x01"`, a `measurement_keys` array, and controlled overhead
keys with this canonical shape:

```toml
[[measurement_keys]]
id = "opcode:0x01"
production_schedule_key = "opcode:0x01"
event_match = { component = "opcode", opcode = "0x01" }
required_case_ids = ["add"]
diagnostic_case_ids = []

[[cases]]
name = "add"
kind = "opcode"
opcode = "0x01"
scenario = "arithmetic"
execution_basis = "interpreter_raw_gas"
template = "stack_binary"
target_raw_gas = 3

[[overhead_keys]]
id = "block_base"
unit = "block"
formula_role = "required"
subtract_keys = ["tx_base", "native_value_transfer"]
bundled_keys = []
bundled_ratios = {}
required_case_ids = ["block_target_control"]
diagnostic_case_ids = []

[[overhead_cases]]
name = "block_target_control"
overhead_key_id = "block_base"
target_template = "controlled_proposal_block_target"
control_template = "controlled_proposal_block_control"
expected_changed_feature_keys = ["block_base", "tx_base", "native_value_transfer"]
```

The reviewed V1 manifest must define exactly these four required overhead identities:

```text
proposal_startup       unit=proposal
block_base             unit=block
tx_base                unit=started_transaction
native_value_transfer  unit=native_value_transfer
```

Witness/input/blob/KZG identities may be present only as `diagnostic` in V1. They cannot enter
`Q_formula` or become required after results are visible.

Freeze this exact subtraction DAG:

```text
tx_base                -> []
native_value_transfer  -> [tx_base]
block_base              -> [tx_base, native_value_transfer]
proposal_startup        -> [block_base]
```

The existing transitive-closure rule deduplicates `tx_base` when measuring block base or startup.

Materialize these required controlled identities rather than relying on free-text scenario expansion:

```text
proposal_startup:
  [startup_minimal_no_candidate_tx, startup_minimal_one_no_code_tx]
block_base:
  [block_base_one_vs_two_minimal_blocks]
tx_base:
  [tx_base_no_code_no_value, tx_base_minimal_contract_call]
native_value_transfer:
  [native_transfer_positive_vs_zero]
```

The two `tx_base` cases must agree within the primary 5% required-case gate after subtracting their
resolved operation work. The native-transfer target/control pair differs in positive native value but
has the same non-create recipient with no executable code and otherwise matched state/environment.
The block case subtracts every induced child and operation delta. The startup panel uses the fixed
residual rule, not a count sweep.

Extend the Python manifest model with these typed fields:

```text
CaseSpec.execution_basis: interpreter_raw_gas|native_gas
CaseSpec.spawned: bool?                         # CALL/CREATE only
MeasurementKeySpec.id: str
MeasurementKeySpec.production_schedule_key: str
MeasurementKeySpec.event_match: EventMatchSpec
MeasurementKeySpec.required_case_ids: tuple[str, ...]
MeasurementKeySpec.diagnostic_case_ids: tuple[str, ...]
EventMatchSpec.component: opcode|precompile
EventMatchSpec.opcode: int?                     # opcode only, 0..255
EventMatchSpec.address: int?                    # precompile only, nonnegative
EventMatchSpec.spawned: bool?                   # CALL/CREATE only
Manifest.measurement_keys: tuple[MeasurementKeySpec, ...]
Manifest.normalization_reference_key: str
Manifest.bridge_key_ids: tuple[str, ...]
Manifest.bridge_model: through_origin_equal_key_median
Manifest.bridge_controlled_max_ape: Decimal          # exactly 0.10 in V1
Manifest.bridge_proposal_max_ape: Decimal            # exactly 0.10 in V1
OverheadCaseSpec.name: str
OverheadCaseSpec.overhead_key_id: str
OverheadCaseSpec.target_template: str
OverheadCaseSpec.control_template: str
OverheadCaseSpec.expected_changed_feature_keys: tuple[str, ...]
OverheadKeySpec.id: str
OverheadKeySpec.unit: proposal|block|started_transaction|native_value_transfer|
                      witness_byte|witness_node|stdin_byte|blob_byte|kzg_invocation
OverheadKeySpec.formula_role: required|diagnostic
OverheadKeySpec.subtract_keys: tuple[str, ...]
OverheadKeySpec.bundled_keys: tuple[str, ...]
OverheadKeySpec.bundled_ratios: dict[str, ExactPositiveRational]
OverheadKeySpec.required_case_ids: tuple[str, ...]
OverheadKeySpec.diagnostic_case_ids: tuple[str, ...]
Manifest.overhead_keys: tuple[OverheadKeySpec, ...]
Manifest.overhead_cases: tuple[OverheadCaseSpec, ...]
```

Precompile matches use `component = "precompile"` and canonical lowercase `address`; opcode matches
use canonical lowercase `opcode`. Final `CaseSpec` rows materialize `kind`, exactly one of
`opcode|address`, `execution_basis`, and optional `spawned`; the existing `scenario` string remains
descriptive only. Opcode cases require `execution_basis = "interpreter_raw_gas"`; precompile cases
require `execution_basis = "native_gas"`. `spawned` is the only V1 scenario discriminator read from a
linked `ChargeAttempt`, and every CALL/CREATE measurement key and case must declare it. Other
opcodes and all precompiles must omit it. A non-CALL/CREATE measurement key without `spawned` matches
from `OperationExecuted` alone. Do not add warm/cold, state shape, input-size bucket, or another
discriminator unless Task 2's frozen event schema contains the exact field and the proposal collector
validates it.

Reject the manifest unless measurement-key and overhead-key IDs are unique, every
`production_schedule_key` exists in the exported schedule, every required set is nonempty, and every
referenced case ID exists in the matching operation or overhead case namespace. Every case ID appears
exactly once across its namespace's required and diagnostic sets and never in both. Each overhead
case's `overhead_key_id` must own that case. Event matches must be pairwise disjoint and contain only
schema-observable fields. The normalization reference must resolve to the ADD measurement key and
must not be diagnostic-only.
Require `Q_formula` to equal exactly
`[proposal_startup, block_base, tx_base, native_value_transfer]` in canonical order for V1. Reject a
manifest that omits, renames, duplicates, or adds another required overhead key. This fixes the V1
cost-model boundary before measurement while leaving additional physical features visible as
diagnostics.

Require `bridge_key_ids` to be nonempty, unique, and selected only from declared measurement keys or
the four required `Q_formula` overhead keys; diagnostic-only overhead keys are forbidden. It must
include the normalization reference, every `Q_formula` key, at least one other opcode, and at least
one precompile. Require the exact V1 model and thresholds above. Missing or rejected bridge inputs
produce a sealed `insufficient_data` bridge report rather than failing candidate sealing; bridge keys
are never silently removed or replaced.

Reject an overhead graph unless every `subtract_keys` and `bundled_keys` reference exists, the sets
are disjoint, the subtraction graph is acyclic, and a deterministic topological order measures every
child before its parent. Define `Q_formula` as exactly the keys marked `formula_role = required` before
any result exists. A bundled key must be diagnostic and cannot occur in `Q_formula` or be owned by two
parents. Every transitive subtract child of a required key must also have `formula_role = required`
and occur in `Q_formula`. Every required formula key and all its transitive subtract keys must pass
the `proverGas` gates before candidate sealing; results never add or remove a formula key. Each
overhead case's declared
changed-feature set must equal the target plus only its transitive subtract/bundled closure, and the
native controlled trace must match that declaration exactly.

Require `bundled_ratios` to have exactly the same keys as `bundled_keys`; numerator and denominator
are positive canonical integers. Reject a manifest with a zero/noncanonical ratio, a bundled child
reachable through a subtraction path, or inconsistent ownership. Materialize every deduplicated
`subtract_closure` as a lexicographically sorted key set and include it in the manifest hash.
For every controlled target/control delta, require
`delta_child * denominator == delta_parent * numerator`. The proposal extractor enforces the same
integer equality on absolute feature counts before using the compound parent feature.

Parse `production_schedule_key` into `(component, identifier)` rather than comparing an opaque string.
For each measurement key, require that tuple to equal the `event_match` component plus opcode/address.
For every referenced required or diagnostic case, require its `kind` and opcode/address to equal the
same tuple. Require the case's `execution_basis` to match its component, and for CALL/CREATE require
`case.spawned == event_match.spawned`; a missing value is not a wildcard. Reject structured context
on components that do not support it. The native controlled trace must also assert that generated
execution actually matches the declared opcode/address, execution basis, and spawned context. Never
use the free-text `scenario` field in these checks. Freeze the validated mapping in the
controlled-manifest hash before any run.

The existing `sp1-smoke.toml` include flags remain smoke-only conveniences. The final
`sp1-calibration-v1.toml` materializes every generated case, measurement key, overhead key, event
match, unit, and required/diagnostic assignment explicitly. It must be reviewed and hashed before
collecting any controlled or proposal result; result-dependent regeneration is forbidden.

Define `resolve_measurement_key(e)` over one `OperationExecuted` plus its optional linked
`ChargeAttempt`. It returns a key only when exactly one frozen `event_match` succeeds. A match that
requires `spawned` fails when the linked charge context is absent. Zero or multiple matches classify
the operation as unmeasured; report `no_measurement_key` or `ambiguous_measurement_key` rather than
falling back to the production schedule key.

For every exact `(case, target_count, lane=target|control)` run three local SP1 execute repeats.
Require identical `case_input_sha256`, exit code, public values, and `proverGas` across all three
repeats. Define `repeat_noise_p = max(p)-min(p)`; nonzero primary noise rejects the case as
non-deterministic instead of averaging it away. Also record
`repeat_noise_s = max(s)-min(s)`. Nonzero or missing secondary instruction-count data marks that
cycle-cost sample unavailable but does not reject an otherwise valid `proverGas` case.

An opcode slope is eligible only when a host-native REVM trace of the exact synthetic bytecode proves
that, across variants, the target's executed raw gas is exactly `x*r` while all non-target executed
opcode/precompile counts and raw gas, bytecode length, calldata length, transaction envelope, and
setup state remain fixed. Keep the static bytecode layout fixed and vary only the immediate or input
that selects how many target operations execute. A current template that grows helper opcodes or code
length with `x` is `confounded_template`, not a calibration result; omit it from the candidate table
until it has an isolated scenario.

Each direct-precompile case has a paired control lane in the same synthetic ELF. The control runs the
same loop, branching, input handling, and output folding over deterministic fixture output with the
same `gas_used` and output length, but does not invoke the precompile. Define the fitted response as:

```text
z_p(x) = p_target(x)                           for an isolation-proven opcode case
z_p(x) = p_target(x) - p_control(x)            for a direct-precompile case
z_s(x) = s_target(x)                           for an isolation-proven opcode case
z_s(x) = s_target(x) - s_control(x)            for a direct-precompile case
```

This prevents growing helper bytecode and per-iteration lab overhead from being assigned to the
target multiplier. A missing or mismatched control rejects the precompile case.
`response_repeat_noise_p` and `response_repeat_noise_s` are the target repeat ranges for an opcode and
the sum of target/control repeat ranges for a precompile.

Evaluate cumulative count prefixes in this fixed order:

```text
[0, 1, 2, 4]
[0, 1, 2, 4, 8, 16]
[0, 1, 2, 4, 8, 16, 32, 64]
[0, 1, 2, 4, 8, 16, 32, 64, 128, 256]
[0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024]
```

Freeze this out-of-fit checkpoint mapping before any measurement:

```text
selected_prefix_max -> extrapolation_check_count
4                   -> 8
16                  -> 32
64                  -> 128
256                 -> 512
1024                -> 2048
```

Fit ordinary least squares to one deterministic `z_p` per distinct `x`. Independently attempt the
same fit for `z_s` for the secondary artifact. The primary candidate gates are:

```text
g_p = sum((x-x_bar)*(z_p-z_p_bar)) / sum((x-x_bar)^2)
b_p = z_p_bar - g_p*x_bar
se(g_p) = sqrt((SS_res/(n-2)) / sum((x-x_bar)^2))
R2_p = 1 - SS_res / sum((z_p-z_p_bar)^2)
signal_p = max(z_p) - min(z_p)

g_s = sum((x-x_bar)*(z_s-z_s_bar)) / sum((x-x_bar)^2)
b_s = z_s_bar - g_s*x_bar
se(g_s) = sqrt((SS_res_s/(n-2)) / sum((x-x_bar)^2))
R2_s = 1 - SS_res_s / sum((z_s-z_s_bar)^2)
signal_s = max(z_s) - min(z_s)
```

A zero denominator in the primary slope or R2 is a rejection, not a perfect-fit special case. A
secondary zero denominator is recorded as a failed sample diagnostic.

Accept the first cumulative prefix satisfying every gate:

- `g_p > 0` and all primary values are finite;
- `signal_p >= max(1000 proverGas, 0.01 * abs(z_p_at_x_0), 20 * response_repeat_noise_p)`;
- `R2_p >= 0.995`;
- `se(g_p) / g_p <= 0.05`;
- maximum absolute `z_p` residual is at most `0.02` of its signal;
- the generator produced every requested count without exceeding its declared bound and the opcode
  isolation trace or precompile paired-control contract above passed.

After the first prefix passes, execute three repeats at its mapped `extrapolation_check_count` under
the identical target/control and isolation contract. This checkpoint is not appended to the fit and
cannot change the selected prefix, slope, intercept, signal, R2, standard error, residual, or
normalization. Define:

```text
observed_delta_p_check = z_p(extrapolation_check_count) - z_p(0)
predicted_delta_p_check = g_p * extrapolation_check_count
APE_p_check = abs(predicted_delta_p_check - observed_delta_p_check) /
              observed_delta_p_check
```

Require a finite positive observed delta, finite prediction, and `APE_p_check <= 0.10`; using the
delta prevents the fitted intercept or startup cost from masking slope error. Here `z_p` is the
opcode/precompile response above or `z_p,q` for a slope-derived overhead. A generation, repeat,
isolation, target/control, positivity, or APE failure rejects the case as
`extrapolation_check_failed`; do not continue to a larger fit prefix after observing the checkpoint.
If the secondary slope was otherwise available, apply the same check to `z_s`; secondary failure
marks `g_s/c_s` unavailable without changing the primary decision. Persist the checkpoint count,
observations, prediction, APE, and status separately from fit observations.

If no prefix passes through `1024`, reject the case with a machine-readable reason and omit it from
the candidate table. A case whose generator cannot produce its mapped checkpoint is not eligible for
calibration. Do not extend the sweep ad hoc after seeing results. For the selected primary
prefix, evaluate the same secondary signal, R2, slope-error, and residual thresholds and store its
status, but do not search for a different prefix or alter the primary decision on behalf of `s`.

For an accepted case with declared per-operation raw gas `r > 0`:

```text
c_p = g_p / r                     # proverGas per raw EVM gas
c_s = g_s / r                     # secondary SP1 instruction count per raw EVM gas
```

Zero, overflow, non-finite primary values, and invalid raw gas reject that case. A bad `c_s` rejects
only the secondary sample. For measurement key `k`, let `C_required(k)` be the exact
manifest-declared `required_case_ids`. The key enters the candidate table only when every member
produced an accepted `c_p` value and the primary consistency check passes:

```text
max(c_p(case) for case in C_required(k)) /
min(c_p(case) for case in C_required(k)) - 1 <= 0.05

```

Use the one required value when there is one; otherwise take the exact arithmetic mean as `c_p(k)`.
Compute and store `c_s(k)` only when every corresponding secondary case is available and passes the
same 5% consistency diagnostic. A missing, failed, rejected, or `confounded_template` primary case classifies
the entire key as `required_case_incomplete`; a primary consistency failure classifies it as
`scenario_dependent`. The candidate receives no value for that key. A failed secondary-only check
marks the sampling row unavailable and leaves `c_p(k)` unchanged. Accepted diagnostic cases are
reported separately and never define, rescue, or veto a candidate value. Proposal-ledger occurrence
does not remove a valid controlled measurement.

### Controlled Non-Opcode Overhead Acceptance

Measure `proposal_startup`, `block_base`, `tx_base`, and `native_value_transfer` as the four required
V1 non-operation costs. One `tx_base` unit is counted per `TransactionStart`, and its coefficient is
the common per-started-transaction residual after modeled work is removed; it is not limited to work
that occurs before that event. `native_value_transfer` is the additional exclusive cost for a
committed positive-value transaction that executes no recipient code. Measure
the block residual only after subtracting transaction, transfer, opcode, and precompile work.
Witness-byte, witness-node, stdin-byte, blob-byte, and KZG-invocation cases may be collected only as
V1 diagnostics. These raw features can overlap, so the manifest freezes a residualization DAG rather
than assuming every other quantity can remain physically constant. Each run records every changed
feature count and operation-gas delta.

Run the same three deterministic repeats, cumulative-prefix fit, frozen checkpoint mapping, and 10%
out-of-fit APE gate for primary `p` responses.
Measure overhead keys in topological order. For overhead key `q`, subtract its paired control, every
resolved operation delta, and every already-accepted descendant cost exactly once before fitting:

```text
z_p,q(x) = p_target(x) - p_control(x)
         - sum(delta_operation_gas(k, x) * c_p(k))
         - sum(delta_feature(t, x) * o_p(t) for t in subtract_closure(q))

z_s,q(x) = s_target(x) - s_control(x)
         - sum(delta_operation_gas(k, x) * c_s(k))
         - sum(delta_feature(t, x) * o_s(t) for t in subtract_closure(q))

o_p(q) = OLS slope of z_p,q(x) against target-unit count x
o_s(q) = OLS slope of z_s,q(x) against target-unit count x
```

Every changed feature must be the target, a transitive `subtract_keys` child, or a predeclared
`bundled_keys` child that is excluded from `Q_formula`; anything else is `confounded_overhead`.
`subtract_closure(q)` is deduplicated, so a descendant reachable through several paths is subtracted
once. Every bundled child must satisfy its manifest-frozen exact child-per-parent ratio in each
controlled delta and proposal row; a mismatch is `bundle_relation_mismatch`, not a fitted ratio.
Every nonzero induced operation delta must resolve to an accepted `c_p` key with the matching
execution basis; unresolved, rejected, ambiguous, or basis-mismatched work is
`confounded_overhead` and prevents sealing when the overhead is required.
Blob-byte and KZG-invocation diagnostics remain separate only when controlled traces prove this
accounting. They do not enter the V1 formula. The `proposal_startup` key is a fixed cost rather than
an ordinary count slope: use a predeclared minimal-proposal panel, subtract every accepted block,
transaction, transfer, opcode, and precompile contribution, require identical `p` across the three
repeats of each member and at most 5% residual dispersion across panel members, then take the exact
arithmetic mean. Never use a proposal-corpus intercept.

```text
startup_residual_p(h) = p(h)
                      - block_count(h) * o_p(block_base)
                      - started_transaction_count(h) * o_p(tx_base)
                      - native_value_transfer_count(h) * o_p(native_value_transfer)
                      - sum(operation_raw_evm_gas(e) * c_p(resolve_measurement_key(e)) for e in h)

max(startup_residual_p(h)) / min(startup_residual_p(h)) - 1 <= 0.05
o_p(proposal_startup) = arithmetic_mean(startup_residual_p(h))
```

Every startup residual must be finite and positive, and every operation in a panel member must resolve
to an accepted primary key. Otherwise startup is unmeasured and the candidate cannot seal. Compute
the analogous secondary residual only when all needed `s` samples exist; its failure remains
non-gating.

Every required variable-count controlled case must pass the same primary determinism, positive-slope,
signal, R2, standard-error, residual, and isolation gates as opcode/precompile cases; startup passes
the fixed-residual gates above. Failure of any member of `Q_formula` or its transitive dependencies
prevents candidate sealing. Apply the secondary diagnostics to `s` and store `o_s` when available,
but never let them change the primary decision. Failed diagnostic overheads remain explicitly
unmeasured; proposal rows cannot supply, repair, rescale, or promote them.

### SP1 Bridge Sidecar And Primary Candidate Seal

For every controlled run, write the exact `sp1_instruction_count`, `proverGas`, workload identity,
repeat ranges, selected primary sweep, and any available `g_s/c_s/o_s` diagnostic to
`samples/controlled-cycle-cost-samples.json`. Hash that file independently. Do not reference this
sample digest from the candidate manifest. A missing or failed secondary row is visible but never
prevents primary sealing.
Write the candidate-referenced normalized observation stream as a separate primary-only projection
that contains no instruction-count field. This keeps secondary bytes outside the candidate digest.

Generate one canonical backend-independent `workload_spec` for every controlled fixture. It contains
the schema version, measurement/overhead key, case ID, target count, target/control lane, complete
state/environment/input parameters, and expected operation/feature deltas. It contains no backend,
run ID, repeat index, SDK/ELF, backend input encoding, or result. Define:

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
rows also store the canonical `workload_spec`. Reject a duplicate execution ID or an identity that
does not match its canonical source fields. Future cross-backend data joins on `workload_id`, never
`execution_row_id`, a free-text case label, or proposal ID alone.

From the controlled samples only, apply the already-frozen `bridge_key_ids`, through-origin median
model, and 10% threshold. Emit every per-key ratio/residual, stratum metric, missing key, and either
`stable_controlled`, `not_stable_controlled`, or `insufficient_data`. Write `bridge-manifest.json`,
`controlled-bridge.json`, `bridge-root.json`, and `bridge.sha256` before proposal output is visible.
The canonical root contains the manifest, controlled-sample, and controlled-result hashes plus their
schema identities; `bridge.sha256` hashes only the root's canonical compact bytes. The bridge root is
not referenced by the candidate root.

Also emit `sp1-diagnostic-overheads.json` and its digest with every accepted/rejected
witness/input/blob/KZG `o_p/o_s` result, fit evidence, and sample identity. This diagnostic table is
not a candidate or bridge dependency.

The later proposal run writes paired observed totals to
`samples/proposal-cycle-cost-samples.json` under the validation ID. It cannot modify the calibration
directory. `bridge-ref.json` binds the sealed bridge digest before any proposal row is executed.

The calibration run emits and seals these review artifacts before any proposal result exists:

- raw controlled rows and all rejected/unmeasured reasons;
- `c_p(k)` for every accepted opcode/precompile key;
- exact ADD-normalized `m_p(k)` while preserving raw costs;
- `o_p(q)` for the four accepted V1 fixed/base overheads;
- the independently hashed controlled sample, diagnostic overhead table, and bridge sidecar roots;
- source, guest, SDK, schedule, manifest, fixture, and command provenance;
- `review_only = true` and `production_write = false`.

ADD and every `Q_formula` dependency must pass the primary `proverGas` gates. Write a canonical
`candidate-manifest.json` containing hashes and schema identities for every normalized primary raw
and cost file, accepted/rejected statuses, `Q_formula`, the residualization DAG, normalization,
formula, thresholds, the frozen prefix-to-checkpoint mapping, provenance, and review-only flags. The
hashed primary artifacts contain the checkpoint count, observed and predicted delta, APE, and status
for every candidate-referenced slope-derived cost; diagnostic slope costs keep equivalent evidence
under their independent artifact digest. `candidate.sha256` hashes only the canonical bytes of this
root manifest, thereby transitively covering the complete candidate. Validation resolves artifacts
only through the manifest and verifies each digest. Refuse to seal an accepted slope-derived cost
without a passing checkpoint. After sealing, neither candidate values nor status may change in the
same run. A changed fixture, manifest, guest, source, normalization anchor, formula, checkpoint
mapping or evidence, or candidate value creates a new calibration ID and a new candidate digest.

Changing `bridge_key_ids`, bridge model, bridge threshold, controlled sample bytes, or controlled
bridge result creates a new bridge digest. Once any proposal output is visible, such a change also
requires a newly versioned corpus containing previously unopened GuestInputs for a new positive
bridge-validation label. It does not require changing an otherwise identical candidate digest.

### Frozen Proposal-Only Validation

Only after the candidate and bridge digests exist may the experiment acquire and execute the fixed
Mainnet/Hoodi proposal corpus. An `insufficient_data` bridge is a valid sealed state and does not
block primary validation. Every proposal row is `final_validation`; there are no fit, calibration,
or holdout proposal splits. The proposal ledger resolves executed operations to the already sealed
measurement keys, and the proposal feature extractor uses the exact controlled overhead units:

```text
execution_raw_evm_gas(e) = e.interpreter_raw_gas for an opcode OperationExecuted
execution_raw_evm_gas(e) = e.native_gas for a precompile OperationExecuted

p_hat(j) = sum(execution_raw_evm_gas(e) * c_p(resolve_measurement_key(e)))
         + sum(controlled_feature(j, q) * o_p(q) for q in Q_formula)

APE_main(j) = abs(p_hat(j) - p(j)) / p(j)
```

`Q_formula` is exactly `[proposal_startup, block_base, tx_base, native_value_transfer]` in canonical
order. Their proposal feature counts are respectively one, successful block count,
`TransactionStart` count, and committed structured native-value-transfer count. Every key and
transitive subtraction dependency must be accepted before sealing. Diagnostic
or bundled keys never appear independently in the sum. This same exclusive feature basis is used in
controlled fixtures and proposal extraction, so witness/blob bytes are not also charged as full stdin
bytes and block/transaction features do not repeat operation work.

The execution sums include resolved operations in committed, attempted, and pre-execution system
work, even when a later charge exceeds the current zkGas limit or overflows. They exclude intrinsic
or pre-validation failures, the unattempted truncation tail, and missing/rejected/ambiguous keys.
Spawned CALL/CREATE is included only when the frozen event match and execution basis resolve exactly.
Current intrinsic charges, spawn estimates, failsafe values, block cap, and current multipliers remain
trace/coverage metadata and never enter the prediction.

The extractor records raw diagnostics including input/started/committed/attempted/unattempted
transaction counts, structured native-value-transfer count, total stdin bytes, typed witness bytes/
nodes, blob bytes, and KZG invocations, but computes formula features according to the sealed
residualization ownership. `input_transaction_count` includes the canonical anchor plus every
manifest candidate, including recovery failures and the unattempted tail. `sp1_stdin_bytes` is the
exact first SP1 stdin-buffer length. `witness_bytes` is the sum of the bincode-serialized witnesses,
proposal ancestor headers, and proposal state nodes; `witness_nodes` is the corresponding frozen node
count. `blob_payload_bytes` is the sum of all `tx_data_from_blob` lengths, and `kzg_invocations` comes
from the frozen trace event representing the controlled KZG unit. Raw overlaps remain diagnostics;
only the exclusive `Q_formula` features enter predictions. Missing a required feature is a validation
error, not a fitting opportunity.

For every proposal, report operation-count and execution-raw-gas coverage over all executed
operations with a valid execution basis. A zero denominator is `coverage_unavailable`, never 100%.
Coverage is descriptive: unsupported keys remain visible and are not priced with the current table.

For `p_hat` against observed `p`, compute every per-proposal APE, combined/per-network MAPE, maximum
error, and the 10% pass/fail decision from `APE_main` with 50-digit `Decimal` arithmetic. MAPE is the
arithmetic mean of proposal APE values, not a ratio of aggregate gas totals. Also report
underprediction count, maximum underprediction, coverage, and worst rows, and emit the raw
paired proposal `(sp1_instruction_count, proverGas)` observations under their separate digest. Apply
the independently sealed bridge without refitting:

```text
p_hat_bridge(j) = kappa_sp1 * sp1_instruction_count(j)
```

If the controlled bridge has a finite `kappa_sp1` (`stable_controlled` or
`not_stable_controlled`), report combined and per-network bridge MAPE, maximum absolute percentage
error, underprediction count, maximum underprediction, strata, and worst rows. Label it
`validated_on_proposals` only when all 60 proposal rows have finite positive observed
`proverGas` and absolute percentage error `<= 10%`; otherwise label it
`not_validated_on_proposals`. If the controlled bridge is `insufficient_data` or a proposal lacks a
finite positive instruction count, emit `not_evaluable_on_proposals` with exact reasons. Report the
controlled and proposal statuses independently; neither changes the candidate classification.

Write `bridge-validation.json` with every proposal prediction and APE, both independent statuses,
and the final `bridge_conclusion` computed from the frozen precedence table above. Validation may
read the sealed bridge root and proposal samples but cannot modify them.

The sealed candidate is
`candidate_table_validated_at_reported_coverage` only when every proposal has finite positive actual
`p(j)`, finite `p_hat(j)`, `APE_main(j) <= 0.10`, positive coverage denominators, exact trace/SP1
identity and public-output joins, and every required fixed feature. Otherwise it is
`candidate_table_not_validated`, with each failed condition recorded. Secondary sample availability
never changes this classification.

Proposal validation cannot fit coefficients, choose features or models, change normalization,
replace a rejected key, or write any calibration artifact. A validation failure leaves the candidate
unchanged and ends the run. Any follow-up hypothesis requires new controlled fixtures, a fresh
calibration ID and candidate digest, and then a newly versioned proposal corpus containing previously
unopened GuestInputs. Reusing the fixed V1 rows is regression evidence only and cannot grant a new
final-validation label.

---

## Task 1: Freeze The Schedule, Controlled Manifest, And Validation Corpus Tooling

**Files:**

- Modify: `xtask/src/export_unzen_zk_gas_schedule.rs`
- Modify: `xtask/src/main.rs` if exporter flags are required
- Modify: `experiments/opcode-gas/opcode_gas.py`
- Create: `experiments/opcode-gas/tests/test_run_manifest.py`
- Modify: `experiments/opcode-gas/README.md`
- Modify: `.gitignore`

### Step 1: Test Complete Schedule Identity

Write failing Rust/Python tests for all 256 opcode rows, explicit versus failsafe identity,
precompile fallback, intrinsic charge, spawn estimates, block limit, deterministic canonical JSON,
and schedule hash changes when any exported field changes. Extend the existing revision-parity test
instead of adding a hand-maintained table.

### Step 2: Implement The Fixed Final-Validation Corpus Selection

Add `prepare-corpus`. Its candidate pool is exactly the 140 unique `(network, proposal_id)` rows in:

```text
tests/fixtures/risc0-zkgas/2026-09-02-m2-aggregation-direct-v3/hoodi-fit.jsonl
tests/fixtures/risc0-zkgas/2026-09-02-m2-aggregation-direct-v3/validation.jsonl
```

Read only `network`, `proposal_id`, `block_count`, and `total_zkgas`. Assert the pool contains 120
Hoodi and 20 Mainnet rows, with no duplicates and positive `total_zkgas`.

- Sort Hoodi by `(block_count, total_zkgas, proposal_id)` and select the 40 rows at zero-based index
  `floor(k * (N-1) / 39)` for `k=0..39`.
- Keep all 20 Mainnet rows, sorted by the same key.
- Mark every selected row `purpose = "final_validation"`. Do not create fit, calibration, or holdout
  proposal subsets.

The exact V1 validation corpus is therefore 60 proposals: 40 Hoodi and 20 Mainnet. Its deterministic
selection is fixed before any controlled or proposal execution result exists, but the fixture bytes
are acquired only after the candidate has been sealed.

Add selection-boundary tests proving an `integration_smoke` row cannot enter this manifest and that
the smoke runner rejects any `(network, proposal_id)` in this exact 60-row membership set before
executing it. A smoke row cannot be relabeled as `final_validation` after execution.

### Step 3: Implement GuestInput Acquisition, Publication, And Sealing

For each network, `prepare-corpus` invokes
`scripts/regression/stress_shasta_proposal.py` with `--proposal-ids`, `--discover-only`, and
`--proposal-out` for the fixed proposal IDs, then invokes `target/release/preflight` with the
discovered proposal tuple,
`--proof-type sp1 --validate true`, network-specific L1/L2 RPC inputs, and an explicit `--output`.
Write atomically under the ignored repository-relative directory:

```text
experiments/opcode-gas/corpora/sp1-mainnet-hoodi-v1/<network>/proposal_<id>.json
```

The checked-in manifest records network, proposal ID, discovered L1 inclusion block, previous anchor,
L2 range, `purpose = "final_validation"`, fixture path, both SHA256 identities, backend-independent
`workload_id`, block count, sum of block difficulties, and acquisition chain-spec hash. Feed each
saved GuestInput unchanged to the SP1 proposal guest. Create a deterministic sorted tar archive with
normalized uid/gid, mode, and mtime. The large archive is not committed.

Add a separate `publish-corpus` command that requires one operator-supplied `gs://` object URI whose
object name ends in `<archive_sha256>.tar`. It invokes `gcloud storage cp` with
`--if-generation-match=0 --print-created-message`, records the returned version-specific URI, reads
back `generation` and `size` with `gcloud storage objects describe --format=json`, downloads that
version-specific URI into a temporary directory, and recomputes SHA256 before writing the checked-in
manifest. If a content-addressed object already exists, describe and verify its current exact
generation instead of overwriting it. The final manifest contains:

```text
archive_uri
archive_generation
archive_size_bytes
archive_sha256
```

`archive_uri` without a positive generation, positive size, successful exact-generation readback,
and matching SHA256 is `local_unpublished`, not a frozen corpus, and cannot enter
`prepare-validation`.
Mock the `gcloud` subprocess in unit tests; tests do not access GCS. No bucket name, account, or
credential path is hard-coded.

Fail instead of substituting a proposal when acquisition, validation, an expected count/total,
post-Unzen activation, nonzero per-block difficulty, or a hash check fails. A changed proposal set is
a new corpus version. Once sealed, all remaining tasks run from local fixtures and make no RPC calls.
Implement and unit-test this command in Task 1; invoke it for the final corpus only in Task 4, after
the Alethia revision and all guest code/artifacts are final.

### Step 4: Freeze Separate Calibration And Validation Identities

Add `prepare-calibration` to write `runs/<calibration-id>/experiment.json` from source revision,
dirty-state flag, Alethia/reth revisions, Rust/SP1 SDK versions, controlled manifest hash, SP1 ELF/VK
hashes, SP1 execution parameters, complete schedule hash, ADD normalization reference, primary
formulas and quality gates, the exact out-of-fit checkpoint mapping and threshold, the
workload-identity schema/version and canonicalization rule, and the exact `bridge_key_ids`, bridge
model, bridge thresholds, and missing-data rules. It also materializes the immutable
`bridge-manifest.json` before any measurement.
Final calibration requires a clean source tree and a materialized manifest with no
implicit include flags. Resume only rows whose complete calibration identity, backend, case identity,
input hash, output, and schema match.

Add a distinct `prepare-validation` command that can run only after `candidate.sha256` and the
independently sealed `bridge.sha256` exist; `insufficient_data` is a valid sealed bridge state. It
writes `validations/<validation-id>/candidate-ref.json` and `bridge-ref.json` plus the
version-qualified proposal corpus identity. Acceptance requires the exact candidate and bridge
digests, 60 completed GuestInputs, 40 Hoodi and 20 Mainnet `final_validation` rows, positive per-block
difficulty, recomputed proposal `workload_id` values, and the immutable archive checks above. Proposal
execution may write only inside that validation directory.

Final validation acceptance additionally requires positive proposal `proverGas`, matching trace/SP1
public output, exact nonzero block reconciliation from Task 2, at least one `OperationExecuted` event,
and positive coverage denominators in every proposal. Record a missing or non-positive SP1
instruction count as an unavailable secondary sample without changing primary acceptance. Rows are
never replaced after measurement, and validation never updates the calibration run.

### Step 5: Verify

```bash
cargo test -p xtask export_unzen_zk_gas_schedule
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_run_manifest.py'
```

Expected: tests pass; acquisition and publication themselves run only when RPC and GCS inputs are
explicitly supplied.

## Task 2: Land The Authoritative Observer And Host-Native Trace

This is the high-risk implementation unit. Land and review the Alethia prerequisite before updating
the raiko2 revision or writing the raiko2 collector.

**Alethia files:**

- Create: `crates/evm/src/zk_gas/observer.rs`
- Modify: `crates/evm/src/zk_gas/{mod.rs,meter.rs,adapter.rs}`
- Modify: `crates/evm/Cargo.toml`
- Modify: `crates/block/src/{executor.rs,derived_block.rs,factory.rs,lib.rs}`
- Modify: `crates/block/Cargo.toml`
- Add focused tests beside the changed meter, adapter, executor, and derived-block modules

**Raiko2 files after the Alethia merge:**

- Modify: every direct Alethia/reth `rev` pin in root, crate, xtask, and guest manifests
- Modify: `Cargo.lock`, `guests/sp1/Cargo.lock`, and `guests/risc0/Cargo.lock`
- Regenerate, never hand-edit: SP1 ELF/VK and RISC0 ELF plus both provenance files under
  `crates/guests/elf/`
- Create: `crates/stateless/src/zkgas_trace.rs`
- Modify: `crates/stateless/src/{lib.rs,validation.rs}`
- Modify: `crates/stateless/Cargo.toml`
- Modify: `bin/guest-launcher/{Cargo.toml,src/main.rs}`
- Create: `crates/stateless/tests/zkgas_trace.rs`
- Modify: `experiments/opcode-gas/opcode_gas.py`

### Step 1: Add Failing Alethia Observer Tests

Cover ordinary and dynamic opcodes, spawn/non-spawn CALL/CREATE, precompile dispatch without wrapper
double-counting, intrinsic charge, arithmetic/limit rejection, pre-execution system calls,
zero-signer filtering, invalid and block-gas filtering, zkGas truncation and tail index, committed EVM
revert, transaction reset, block commit, and fatal execution. Add an A/B test proving no-observer and
observer executions produce identical receipts, committed transactions, state root, and finalized
zkGas. Assert that an opcode/precompile emits `OperationExecuted` before `ChargeAttempt`, both share
one block-unique monotonic `operation_id`, and a later `limit_exceeded` or `arithmetic_overflow` does
not erase the executed event. Assert that an intrinsic/pre-validation failure and the unattempted
truncation tail emit no operation event. For spawned CALL/CREATE, assert that
`OperationExecuted` is emitted at `step_end` without guessing `spawned`, while the linked
`ChargeAttempt` records `spawned=true` and keeps `interpreter_raw_gas` distinct from
`charge_raw_gas` with source `spawn_estimate`.

Add structured transaction-classification tests for a committed positive-value transfer to an
account with no executable code, zero-value/no-code transaction, contract call, contract creation,
failed transaction, and unattempted tail. Only the first increments `native_value_transfer_count`;
every emitted `TransactionStart` increments `started_transaction_count` exactly once.

### Step 2: Implement One Alethia Source Of Truth

Emit `OperationExecuted` after the ordinary opcode step or precompile body has executed, before the
subsequent current-schedule charge result is known. Emit `ChargeAttempt` at the meter/adapter point
that already selects interpreter gas, spawn estimates, precompile gas, multipliers, and checked
outcomes; correlate operation charges by `operation_id` and leave intrinsic charges uncorrelated.
For CALL/CREATE, allocate and emit the execution event at `step_end`, carry its `operation_id` in the
deferred step, and emit the charge event only after dispatch resolves `spawned`.
Emit phase, transaction, disposition, reset, commit, and truncation events in the executor that
already owns those decisions. Expose `execute_derived_block_with_observer`; keep
`execute_derived_block` delegating to the normal no-observer path. Do not create a second executor or
filtering loop.

The `execution-observer` feature is default-off and forwarded from `alethia-reth-block` to
`alethia-reth-evm`. Merge this work into Alethia `main` and record the reviewed commit. Do not pin
raiko2 to a PR branch.

### Step 3: Update Raiko2 To The Reviewed Revision

Update every direct Alethia pin and the compatible direct reth pins as one dependency revision,
regenerate all tracked lockfiles, and inspect `cargo tree -d`. In particular verify the production
SP1 guest path `raiko2-guest-sp1 -> raiko2-guest-common -> alethia-reth-block` and the corresponding
RISC0 guest path both use the reviewed revision. Do not hide an incompatible mixed dependency graph
with a raiko2-side adapter.

Both built guests use that same Alethia revision with `execution-observer` disabled, although only
SP1 is a V1 measurement backend. Verify the separation explicitly:

```bash
cargo tree --manifest-path guests/sp1/Cargo.toml -e features
cargo tree --manifest-path guests/risc0/Cargo.toml -e features
just build-guest sp1
just build-guest risc0
sha256sum crates/guests/elf/sp1_shasta_proposal.elf \
  crates/guests/elf/sp1_shasta_proposal.vk.bin \
  crates/guests/elf/risc0_shasta_proposal.elf \
  crates/guests/elf/sp1.provenance.json \
  crates/guests/elf/risc0.provenance.json
cargo run -p xtask --features guest-tools -- guest-digests \
  --output target/zkgas-guest-digests.json
```

Neither feature tree may contain `execution-observer`. Record the SP1 ELF/VK identity and RISC0 ELF/
proposal image ID, build and test the host observer path, rebuild both unchanged guests, and require
all recorded hashes/image IDs to remain identical across those two builds. Equality with the
pre-revision artifacts is not required: the dependency revision itself legitimately changes guest
code.

### Step 4: Add The Raiko2 Derivation Observer And Collector

Write tests first for signer-recovery success/failure without changing `build_derived_block` results.
The ordinary function delegates to a no-observer helper; only the host trace command records the
derivation events.

Add `guest-launcher proposal-trace` to deserialize the sealed GuestInput and call the same Shasta
manifest reconstruction used by `prove_shasta_proposal`, but with Alethia's observer enabled. It
emits the transactional ledger above, exact bincode/input-size features, canonical output hash, and
`guest_input_sha256`. It must validate the generated blocks against canonical blocks exactly as the
guest path does.

The normal SP1 proposal run remains uninstrumented. The launcher adds only host-computed
`guest_input_sha256` and encoded input length before invoking SP1. Join trace and SP1 rows only on
that hash and matching public output; reject proposal-ID-only matches. Do not add a RISC0 proposal
execution step to V1.

### Step 5: Verify Alethia And Raiko2

Run the focused upstream tests required by Alethia, then in raiko2:

```bash
cargo fmt --all -- --check
cargo test -p raiko2-stateless --test zkgas_trace
cargo test -p guest-launcher proposal_trace
cargo clippy -p raiko2-stateless -p guest-launcher -- -D warnings
just build-guest sp1
just build-guest risc0
```

Before the final corpus is sealed, acquire one temporary post-Unzen candidate from each network for
this integration smoke test. Record `purpose = "integration_smoke"` before execution and reject any
candidate whose `(network, proposal_id)` belongs to the predetermined 60-row `final_validation`
selection; choose another unexecuted smoke candidate before running instead. Do not add the temporary
files or results to the final manifest, candidate, bridge, or report, and never relabel them
`final_validation`. Run both through the trace and SP1 passes. Require matching GuestInput hashes and
canonical/public outputs, identical pre/post host-feature guest identities for both zkVM builds,
positive `p`, recorded secondary `s` status, and exact nonzero reconciliation for every block. The
RISC0 requirement here is build/artifact isolation, not proposal execution.

## Task 3: Build And Seal The Controlled SP1 ProverGas Candidate

**Files:**

- Modify: `experiments/opcode-gas/opcode_gas.py`
- Create: `experiments/opcode-gas/manifests/sp1-calibration-v1.toml`
- Create: `experiments/opcode-gas/tests/test_sp1_candidate_report.py`
- Modify: `experiments/opcode-gas/tests/test_manifest.py`
- Modify: `experiments/opcode-gas/README.md`
- Modify: `crates/primitives/src/opcode_lab.rs`
- Modify: `guests/sp1/src/{revm_opcode_lab.rs,revm_opcode_lab_impl.rs}`
- Modify: `guests/sp1/src/{precompile_lab.rs,precompile_lab_impl.rs}`
- Create: `bin/guest-launcher/src/controlled_workload.rs`
- Create: `bin/guest-launcher/tests/controlled_workload.rs`
- Modify: `bin/guest-launcher/src/main.rs`
- Regenerate, never hand-edit: SP1 ELF/VK/provenance files under `crates/guests/elf/`

### Step 1: Test The Synthetic Gates

Write failing manifest tests for duplicate measurement-key IDs, missing production schedule keys,
empty or unknown required case IDs, a case listed twice or as both required and diagnostic, unsupported
event-match fields, overlapping matches, and `spawned` on a non-CALL/CREATE opcode. Prove that a
generic CALL/CREATE match without `spawned` is rejected and that explicit `spawned=true` and
`spawned=false` matches are disjoint.

Add separate rejection fixtures for every three-way coherence failure: case opcode differs from the
event match, case precompile address differs, case component differs, production schedule key differs
from the event identity, opcode/precompile execution basis is wrong, and CALL/CREATE `spawned` is
missing or differs between the case and event match. Run each fixture once with the bad case in
`required_case_ids` and once in `diagnostic_case_ids`. Assert that changing only the free-text
`scenario` never repairs or creates an identity match.

Write failing measurement tests for three-repeat identity, nonzero repeat noise, each cumulative sweep
expansion, absolute/relative signal, R2, slope standard error, residual bound, exhausted-sweep
rejection, generator-bound rejection, every frozen prefix-to-checkpoint mapping, and the 10%
out-of-fit APE boundary. Prove the checkpoint is absent from OLS inputs, a pass accepts the first
prefix, and a failure rejects without trying a later prefix. Cover missing/non-positive checkpoint
responses, generation failure, and secondary-only failure. Add bytecode-trace tests proving non-target executed work
and code/input size stay fixed; existing grow-with-count templates must fail as
`confounded_template`. Add paired target/control precompile tests proving identical
loop/input/output-folding shapes and exact `proverGas` slope subtraction; record the corresponding
instruction-count diagnostic without making it a primary gate.

Add controlled tests for `proposal_startup`, `block_base`, `tx_base`, and
`native_value_transfer`. Prove that startup uses the repeat-stable residual of the predeclared minimal
panel rather than an ordinary slope or proposal-derived intercept; that block base subtracts all
transaction/transfer/operation contributions; and that native transfer is additional to `tx_base`.
Prove every slope-derived required or diagnostic overhead uses the same frozen checkpoint gate and
that fixed startup is exempt.
Add diagnostic-only witness-byte, witness-node, stdin-byte, blob-byte, and KZG-invocation cases. Each
test enumerates every induced feature and operation delta, then proves that the residualization DAG
owns it exactly once. Add rejection tests
for a cycle, missing dependency, two parents owning one bundled key, undeclared changed feature,
parent-before-child execution, failed required dependency, and any formula term counted both raw and
residualized. Add a three-level diamond DAG proving that the deduplicated transitive closure subtracts
each descendant exactly once. Test missing, zero, noncanonical, controlled-mismatched, and
proposal-mismatched bundled ratios. Assert that none of the diagnostic-only dimensions enters
`Q_formula`, that a proposal-derived intercept cannot enter the controlled overhead artifact, and
that the V1 manifest rejects any required overhead set other than the four frozen identities. Execute
these constructed GuestInputs with the production
`sp1-shasta-proposal` ELF; do not add a separate overhead guest whose program costs would differ from
the proposal path.

Add manifest rejection tests for an empty or duplicate `bridge_key_ids`; an unknown key or a
diagnostic-only overhead key; omission of ADD or any `Q_formula` key; omission of an additional opcode
or precompile; a bridge model other than `through_origin_equal_key_median`; and a controlled or
proposal threshold other than exactly 0.10. The accepted manifest fixes the complete bridge key set
before any measurement output exists.

Add workload-identity tests proving the same controlled `workload_spec` has the same `workload_id`
across backend, run ID, repeat, SDK/ELF, and backend encoding changes, while any semantic state,
environment, input, target-count, lane, expected-operation, or expected-feature change produces a
different ID. Prove that execution IDs differ across backend/run/repeat/input changes. For proposals,
the same `guest_input_sha256` must produce the same `workload_id` across validation IDs and backends.

### Step 2: Test High-Precision Primary Construction And Secondary Sampling

Cover exact `g_p/r` conversion without integer rounding and primary zero/non-finite/overflow
rejection. For one measurement key, test all required primary cases accepted within the 5%
consistency gate, the exact mean, one required case missing/rejected/confounded, and a primary value
outside the consistency gate. The latter cases exclude the complete key as
`required_case_incomplete` or `scenario_dependent`; no successful sibling case may enter alone. Test
that accepted/rejected diagnostic cases remain reported but do not define, rescue, or veto the
primary value.

Assert that ADD is the frozen normalization reference, that a failed primary ADD measurement prevents
candidate sealing, and that exact `m_p(k)=c_p(k)/c_p(ADD)` preserves raw `c_p`. Test the four required
`o_p` values and exact fixed-startup residual averaging. Separately test that `g_s/c_s/o_s`, repeat
noise, and failed secondary diagnostics are serialized to the cycle-cost sample artifact; no
secondary status changes `c_p`, `m_p`, `o_p`, the candidate digest, or the validation classification.
Test exact per-key `rho`, equal-key median `kappa_sp1`, through-origin predictions, absolute percentage
errors, strata, and all three controlled bridge states. Any missing/non-positive bridge input must
produce a sealed `insufficient_data` report without dropping or replacing the key. A ratio outside the
10% threshold produces `not_stable_controlled`. Changing the bridge key set, sample, model, threshold,
or result changes `bridge.sha256` but never `candidate.sha256`; no bridge status changes `c_p`, `m_p`,
`o_p`, primary validation, or candidate sealing. V1 emits no cycle-native candidate table.

Test that `sp1-diagnostic-overheads.json` reports witness/input/blob/KZG observations, units,
controlled evidence, and missing/confounded reasons under its own digest. None of these diagnostic
dimensions may enter `Q_formula`, the candidate root, or the bridge.

Test that every accepted slope-derived primary cost has a passing checkpoint in its hashed evidence.
Changing the frozen checkpoint mapping, threshold, observation, prediction, APE, or status changes the
candidate digest; missing or failed evidence prevents sealing. Secondary checkpoint status remains
outside the candidate root.

Cover proposal-ledger occurrence reporting without measurement deletion and add a guard that
experiment Python commands write to neither Alethia, production schedule, Boundless config, nor
generated ELF paths. Assert that candidate artifacts contain `review_only = true` and
`production_write = false`, and that no protocol integer table is synthesized. Add a large-integer
fixture that would lose significance through `float` and assert exact 50-digit `Decimal` slopes,
costs, normalization, primary prediction, and serialized-string reproduction.

### Step 3: Implement Controlled Runs And Candidate Construction

Add only the next declared count level when the current prefix fails a quality gate. Persist all
repeats and decisions so resume cannot silently choose a different prefix. Emit exact primary `g_p`,
per-case `c_p`, every frozen measurement key and event match, required/diagnostic membership,
selected per-key `c_p(k)`, ADD-normalized `m_p`, checkpoint observations/predictions/APE/status,
accepted/rejected evidence, and coverage gaps. Run
the controlled overhead cases and emit exact `o_p`, including the fixed startup residual. Write the
simultaneously observed `s` values and available `g_s/c_s/o_s` diagnostics to the separately hashed
sampling artifact. From those controlled samples only, build the frozen through-origin bridge,
including its ratios, `kappa_sp1`, controlled predictions, diagnostics, status, canonical manifest,
and independent digest. Emit the separately hashed diagnostic overhead cost table. Do not emit an
integer schedule or Rust table. Materialize and review
`sp1-calibration-v1.toml` without the
smoke manifest's implicit include flags before running any measurement. Changing the synthetic lab
guest is allowed for isolation/control scenarios; it does not enable the Alethia observer in the
measured proposal guest. Freeze the final SP1 artifacts only after these changes are complete.

Implement one canonical structured identity parser and one `EventMatchSpec` matcher. Manifest
coherence validation and proposal resolution must call those same helpers; do not maintain a second
string-based opcode/address/spawn interpretation in the report path.

Build all candidate components from controlled results only. Write `candidate-manifest.json` as the
canonical root containing every component hash, `Q_formula`, DAG, formula, thresholds, frozen
prefix-to-checkpoint mapping, normalization, status, provenance, and review-only field; then hash its
canonical bytes as `candidate.sha256`. Refuse to seal if ADD, a `Q_formula` key, or a transitive
dependency is absent; if an accepted slope-derived primary cost lacks passing checkpoint evidence;
if any referenced component hash fails; if any output is non-finite; or if a proposal result already
exists in the calibration directory.

Verify that the premeasurement `bridge-manifest.json` still matches `experiment.json`, then write
`controlled-bridge.json` and a canonical `bridge-root.json` containing the manifest,
controlled-sample, and controlled-result hashes plus schema identities. Seal `bridge.sha256` as the
SHA256 of that root before proposal output is visible. A complete manifest with an
`insufficient_data` result is sealable. Neither the bridge root nor the diagnostic overhead root is
referenced by `candidate-manifest.json`.

### Step 4: Test The Immutable Proposal Validation Boundary

Write failing tests for ledger transaction boundaries, attempted/system/committed recomputation,
current difficulty parity, checked overflow, exact trace/SP1 GuestInput and public-output joins, and
positive `p`. Record missing/non-positive `s` as a secondary-sample failure. Cover prediction
inclusion for executed committed/attempted/system operations, including an
operation whose later charge returns `limit_exceeded` or `arithmetic_overflow`. Cover exclusion for
intrinsic/pre-validation failure, unattempted work, unmeasured keys, and spawn-basis mismatch. Reject
any use of current zkGas multipliers or fixed spawn estimates as proving-gas coefficients.

Test exact measurement-key resolution from `OperationExecuted` plus its optional linked
`ChargeAttempt`: one match enters the fixed prediction; no match and missing required linked context
enter unmeasured coverage; ambiguous matches reject the manifest before execution. With two disjoint
CALL/CREATE spawned/non-spawned measurement keys, prove that failure of one required set excludes only
that measurement key. Separately prove the required P1 case: when two required scenarios share one
unresolvable measurement key and one is accepted while the other is rejected, the complete key stays
out of `K` and every matching proposal operation enters unmeasured coverage.

Test exact feature extraction for proposal startup, block base, started transactions, and committed
native-value transfers, plus diagnostic blob-byte and KZG-invocation counts. Test the fixed `p_hat`
formula and exact `APE_main = abs(p_hat-p)/p` denominator using 50-digit `Decimal`. Prove MAPE is the
arithmetic mean of proposal APE values rather than aggregate-gas error. Test metric signs,
per-network aggregation, worst-row ordering, operation-count and execution-raw-gas coverage,
zero-denominator rejection, the exact 10% boundary, the two primary validation classifications, and
deterministic report recomputation. A deliberately wrong controlled value must fail the
absolute-percentage-error gate; no proposal
coefficient, feature selection, normalization, fallback, or repair is permitted.

Test that paired proposal `s/p` rows are written under the validation ID with their own digest, that
validation never writes them into the calibration directory, and that it applies the sealed
`kappa_sp1` without refitting, dropping rows, changing the key set, selecting a model, or changing a
threshold. Cover `validated_on_proposals`, `not_validated_on_proposals`, and
`not_evaluable_on_proposals`; none may change the primary classification or candidate digest.
Test the exact APE denominator and all combinations of controlled/proposal status. The final result is
`inconclusive` for `insufficient_data`; `not_supported` for `not_stable_controlled` regardless of
proposal status; `inconclusive` for `stable_controlled + not_evaluable_on_proposals`; `supported` only
for `stable_controlled + validated_on_proposals`; and `not_supported` for the remaining stable pair.

Add write-boundary tests proving validation can read the sealed candidate and bridge but cannot modify any file
under its calibration ID. Reject validation when the root digest, a manifest-referenced component
digest, `Q_formula`, formula, threshold, status, bridge reference, or provenance differs; with any
proposal-purpose value other than `final_validation`; or when the candidate, bridge, and validation
provenance differ. Scan the implementation and output schema for forbidden proposal fit, model
selection, threshold-selection, and calibration-write fields.

### Step 5: Verify

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_sp1_candidate_report.py'
just build-guest sp1
```

## Task 4: Execute Controlled Calibration, Then Final Proposal Validation

**Files:**

- Create: controlled rows, primary costs, separately hashed cycle-cost samples, diagnostic overhead
  table, bridge artifacts, candidate JSON, and independent digests under
  `experiments/opcode-gas/runs/<calibration-id>/`
- Create: `experiments/opcode-gas/manifests/proposals/sp1-mainnet-hoodi-v1.json`
- Create: candidate and bridge references, proposal manifest, proposal rows, bridge validation JSON,
  primary validation JSON, `report.json`, and `report.md` under
  `experiments/opcode-gas/validations/<validation-id>/`
- Modify: `experiments/opcode-gas/README.md` with the exact completed-run commands

### Step 1: Run And Seal Controlled Calibration

Require a clean implementation revision, exact source/dependency/SP1 identities, complete schedule
identity, materialized controlled manifest, and an empty calibration directory. Print the immutable
calibration ID, run all opcode/precompile and non-opcode controlled suites, accept or reject every
declared required key, build the native and ADD-normalized `proverGas` views, write the separate
controlled cycle-cost samples and diagnostic overhead cost table, build the controlled bridge, and
seal the independent candidate and bridge digests. An `insufficient_data` bridge is allowed; a missing
or unsealed bridge is not. No proposal fixture or result may be read during this step.

### Step 2: Acquire And Seal The Final-Validation Corpus

Only after Step 1 has sealed `candidate.sha256` and `bridge.sha256`, invoke `prepare-corpus` with the
final binaries and network RPC inputs, inspect its 60 `final_validation` rows, publish the
content-addressed archive with `publish-corpus`, and read back its exact GCS generation and bytes.
Commit only the version-qualified manifest while fixtures and the local archive remain ignored.
`prepare-validation` binds the exact candidate and bridge digests, corpus generation, GuestInput
hashes, SP1 guest identity, and source provenance to an immutable validation ID.

### Step 3: Run Separate Proposal Validation Passes

Run all 60 proposal GuestInputs through the ordinary, uninstrumented SP1 proposal guest. Separately
run the host-native trace on all 60. Do not execute RISC0 proposals, submit proofs, contact Boundless,
or access RPC after the corpus was sealed.

Resume validates every completed row before reuse. Any missing, duplicate, failed, mixed-provenance,
hash-mismatched, non-positive `p`, trace/SP1 public-output mismatch, all-zero difficulty,
reconciliation, or canonical-output failure prevents final report generation. Missing/non-positive
`s` produces an unavailable secondary sample and `not_evaluable_on_proposals` bridge status but does
not change primary validation.

### Step 4: Apply The Frozen Candidate And Report

Apply the sealed `c_p` and `o_p` values directly to every frozen proposal ledger and feature row. Emit
the exact predicted/observed `proverGas`, per-key required/diagnostic outcomes, operation-count and
execution-raw-gas coverage, missing-key and feature diagnostics, classification, rejection reasons,
and worst proposals in JSON and Markdown. Recompute
and verify the candidate digest before and after validation; any changed calibration byte fails the
run. Separately emit the paired observed proposal instruction-count/`proverGas` samples under their
own digest. Apply the sealed `kappa_sp1` forward formula without fitting or selection and emit the
bridge diagnostics and one of the frozen proposal bridge statuses. Recompute `bridge.sha256` before
and after validation; any changed bridge byte fails the run. Emit the exact
`supported|inconclusive|not_supported` bridge conclusion from the frozen truth table. The bridge
result never changes the primary report or candidate classification.

### Step 5: Run Final Verification

```bash
env PYTHONDONTWRITEBYTECODE=1 ~/.venv/bin/python -m unittest discover \
  -s experiments/opcode-gas/tests -p 'test_*.py'
cargo fmt --all -- --check
cargo test -p raiko2-stateless
cargo test -p guest-launcher
cargo clippy -p raiko2-stateless -p guest-launcher -- -D warnings
just build-guest sp1
just build-guest risc0
cargo run -p xtask --features guest-tools -- guest-digests \
  --output target/zkgas-guest-digests.json
```

Independently review the complete Alethia and raiko2 diffs. Independently rerun the observer A/B
tests, one Mainnet and one Hoodi full trace-plus-SP1 reconciliation, both guest feature/artifact
isolation checks, one exact-generation corpus download/hash check, and a fresh recomputation of both
the candidate and validation reports from committed normalized rows. Investigate every material
finding and have the reviewer/tester re-check fixes before declaring the reports fixed.

### Step 6: Handoff Boundary

The handoff contains the calibration and validation IDs, candidate digest, exact source/dependency/
SP1/schedule/corpus identities, the RISC0 release-consistency artifact identity, commands and results,
raw and ADD-normalized `proverGas` table, controlled fixed/base costs, separately hashed controlled
and proposal cycle-cost samples, the diagnostic overhead cost table, controlled and proposal bridge
diagnostics/statuses/final conclusion, backend-independent workload identities, measured/unmeasured
proposal coverage, primary proposal-validation metrics, required and diagnostic case outcomes,
rejected keys, and the exact candidate classification.
Every candidate artifact remains explicitly review-only; the run makes no production write and does
not define a protocol integer scale.

Choosing a production integer encoding, changing the Alethia schedule, rebuilding a release, or
deploying it is a separate developer decision and separate reviewed change. A V2 RISC0/Boundless
design may rerun identical controlled workloads under any SP1 bridge conclusion to build an
independent RISC0 table or compare backend-specific tables. Direct scalar transport from SP1 requires
`bridge_conclusion = supported`; a cross-backend max policy remains a separate decision.
