# SP1 ZKGas Multiplier Recalibration Implementation Plan

## Status

Ready for implementation as amended by the approved relative-opcode and controlled-block model. This
plan defers to `docs/plans/2026-09-06-zkgas-multiplier-recalibration-experiment-design.md` for the
primary candidate model. V1 removes
every proposal-derived normalization, coefficient, residual fit, feature-selection, and
model-selection path. It calibrates one SP1 `proverGas` candidate on controlled fixtures:
raw-gas opcode/precompile multipliers, fixed spawned-wrapper event costs, plus proposal-startup,
block-base, transaction-base, and
native-value-transfer costs. It seals that candidate and uses proposals only for final forward
validation. SP1 instruction count is sampled alongside `proverGas` and evaluated by an independently
sealed, non-gating V1 bridge sidecar. Production integer encoding, cross-backend/RISC0 bridge
validation, and table installation remain separate work.

> **For agentic workers:** Use `superpowers:executing-plans` for this plan and
> `superpowers:test-driven-development` for each behavioral change. Task 2 changes consensus
> metering timing in Alethia and adds a second host execution path in raiko2, so it also requires an
> independent adversarial review and an independent behavioral verification pass.

**Goal:** Reuse the existing experiment foundation to produce a reproducible SP1-native `proverGas`
raw-gas multiplier table from frozen matched-control relations and a joint four-anchor controlled
block fit, plus explicit spawned-wrapper, startup, block, transaction, and native-transfer costs, then validate
the completely frozen candidate on one Mainnet/Hoodi proposal corpus while independently fitting and
validating a non-gating SP1 instruction-count-to-`proverGas` sidecar.

**Architecture:** First run the frozen matched-control relation and dynamic raw-gas holdout suite,
then jointly fit four opcode anchors and four fixed/base costs on controlled production-guest block
rows, using normalized SP1 `proverGas` as the sole V1 candidate metric. Reconstruct one
schedule-shaped raw-gas native and ADD-normalized multiplier table, add independent precompile and
fixed spawned-wrapper components, and seal a content digest. SP1
total instruction count is captured from the same executions. Before proposal output is visible,
freeze an independent through-origin median bridge, eligibility, and 10% thresholds; it never gates
the candidate. Only after sealing both roots, measure each proposal with the normal SP1 guest and trace the
identical GuestInput in a separate host-native pass; validation applies the frozen `proverGas`
formula and separately validates the frozen bridge without fitting or selecting any proposal-derived
value. The trace schema and collector live in a host-only raiko2 crate; Alethia receives only the
minimal CALL/CREATE inspector-parity fix needed to make traced and plain execution stop at the same
pre-dispatch boundary. A later experiment may measure RISC0 costs independently under every bridge conclusion; only
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
- Every required relation must pass its frozen, non-fitting checkpoint and the dynamic raw-gas
  holdouts; the exact block fit/holdout matrix must pass its declared rank, positivity, APE, and
  leave-one-family-out gates.
- Experiment commands never modify the production schedule, runtime, Boundless configuration, or
  generated guest artifacts.
- The measured SP1 guest and RISC0 guest never depend on the host-only raiko2 trace crate.

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
- `r`: actual interpreter raw EVM gas charged by a synthetic execution;
- `mu(k)`: reconstructed pure-opcode `proverGas / raw EVM gas` multiplier;
- `g_p(k)` and `g_s(k)`: controlled marginal `proverGas / operation` and secondary
  `SP1 instruction count / operation` for precompile or fixed-event measurements, not independently
  required pure-opcode costs;
- `basis(k)`: frozen `raw_gas_slope|fixed_per_event` pricing basis;
- `c_p(k) = mu(k)` for pure opcodes and `c_p(k) = g_p(k)/r(k)` for direct precompiles: the V1
  backend-native proving-gas multiplier per raw EVM gas;
- `f_p(k) = g_p(k)`: V1 proving-gas cost per event for `fixed_per_event` keys;
- `m_p(k)`: the corresponding dimensionless multiplier normalized to
  `normalization_reference_key = "opcode:0x01"`;
- `o_p(q)`: controlled proving-gas cost per declared non-opcode unit `q`;
- `c_s(k) = g_s(k)/r(k)`, `f_s(k) = g_s(k)` for fixed-event keys, and `o_s(q)`: optional secondary
  instruction-count costs stored only in the cycle-cost sampling artifact;
- `fixture_sha256`: SHA256 of the saved JSON file bytes;
- `guest_input_sha256`: SHA256 of `bincode::serialize(GuestInput)`, which is the exact byte vector
  inserted into the first proposal SP1 stdin buffer;
- `case_input_sha256`: SHA256 of the bincode bytes inserted into the first synthetic-lab SP1 stdin
  buffer;
- `workload_id`: backend-independent logical workload identity; it excludes run ID, backend,
  repeat, SDK/ELF, backend input encoding, and measured results;
- `execution_row_id`: identity of one backend execution; it includes `workload_id`, run ID, backend,
  repeat, and the backend-specific input hash.

Pure opcode costs are reconstructed from accepted matched-control relations instead of requiring one
isolated positive `g_p(k) / operation` value per key. The canonical primary model is:

```text
A * mu = d
theta = [mu(POP), mu(PUSH0), mu(DUP1), mu(SWAP1)]
mu = mu_zero + B * theta

p_hat_j = x_j * mu_zero + [x_j * B, q_j] * [theta, beta]
beta = [proposal_startup, block_base, tx_base, native_value_transfer]
```

`A_i` and `x_j` are per-key actual raw-gas totals, not execution counts. The frozen 102-key relation
matrix has rank 98; `mu_zero` and `B` come from that matrix, while only the four natural anchors and
the four fixed/base coefficients are fitted from predeclared controlled block rows. Proposal rows
cannot modify a relation, anchor, coefficient, threshold, or row set.

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
for `raw_gas_slope` keys while preserving raw `c_p`. Preserve `fixed_per_event` values as `f_p`
without ADD normalization. This reference exposes the raw-gas SP1 vector shape and is not the
protocol integer scale. Do not combine these values with the current intrinsic charge, spawn
estimates, failsafe value, or 100M block cap.

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

Use `fractions.Fraction` for exact relation algebra: derive and validate `A`, its rank, the natural
anchor reduction, `mu_zero`, and `B` without binary floating point or Decimal rounding. Use
`decimal.localcontext(prec=80, rounding=ROUND_HALF_EVEN)` for observations, fitting, coefficients,
secondary sample diagnostics, predictions, errors, and serialization. Parse integer observations
directly into `Decimal`; never route canonical values through binary `float`. Serialize canonical
numeric fields as normalized decimal strings, reject non-finite values, and compare every quality
threshold in `Decimal` arithmetic.

### Relative Relations And Controlled Block Artifacts

The former sequential overhead-residualization implementation is superseded for the primary
candidate. It must not fit `proposal_startup`, `block_base`, `tx_base`, or
`native_value_transfer` one at a time after isolated absolute opcode slopes. Instead, freeze the
relation cohort, dynamic raw-gas holdouts, controlled block fit rows, and controlled block holdout
rows before running SP1. The fit/holdout rows use the production `sp1-shasta-proposal` guest, exclude
precompile and spawned-wrapper work, and are rejected unless the exact eight-column design matrix is
full rank. System and Anchor work remain owned by `block_base`; trie, Merkle, hashing, and other
block baseline work are not split below that owner. Bridge inputs stay independently isolated and
cannot alter the primary fit.

The primary artifact sequence is exactly:

```text
opcode-relations.json
block-calibration-rows.jsonl
block-calibration.json
controlled-fit.json                 # precompile and fixed-event results only
candidate/candidate-manifest.json
candidate/candidate.sha256
```

Candidate sealing transitively binds the relation matrix and natural-anchor verification, all
dynamic raw-gas holdouts, the controlled block matrix/rows/holdouts, precompile and fixed-event
components, provenance, coverage, formula, and thresholds. `candidate.sha256` is required before
`prepare-validation` may access final proposal rows. Failed/missing rows preserve raw artifacts and
emit no seal; neither proposal results nor the current production table may repair a candidate.

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

### Host-Native Trace Boundary

Alethia does not own an experiment observer. PR #239 is reduced to one production-parity fix in the
existing `ZkGasInspector`: CALL/CREATE spawn detection and wrapper charging occur at `step_end`, after
the interpreter selects `NewFrame` and before child dispatch, exactly as in `run_metered_plain`. A
failed wrapper charge clears the pending frame action and halts before child code can run. The change
does not add event types, serialization, transaction lifecycle hooks, derived-block entrypoints, or
feature flags.

All offline instrumentation lives in the host-only `raiko2-zkgas-trace` crate. It uses Alethia's
existing public `TaikoEvmFactory::create_evm_with_inspector` and
`TaikoBlockExecutor::execute_block_with_committed_transactions` APIs. It does not reproduce the
schedule, meter formula, transaction filters, retry/reset rules, commit loop, or block truncation
logic. The crate is a dependency of `guest-launcher` only; `raiko2-guest-common`, the SP1 guest, and
the RISC0 guest do not depend on it.

The raiko2-local serialized schema contains execution facts, not charge reconstruction:

```text
ProposalTrace {
  guest_input_sha256, status=complete|failed, public_output?,
  blocks[], recovery_failures[], failure?
}
RecoveryFailure {
  block_index, manifest_index, signed_transaction_hash,
  outcome=signer_recovery_failed
}
TraceFailure {
  block_index?, stage=ordinary|traced|parity, error,
  partial_block_diagnostics?
}
BlockTrace {
  block_index, block_number, input_transaction_count,
  started_transaction_count, committed_transaction_hashes,
  attempted_transaction_hashes, unattempted_transaction_hashes,
  native_value_transfer_count, finalized_block_zkgas,
  transactions[], operations[]
}
OperationTrace {
  operation_id, phase=system|transaction, tx_index?, frame_depth,
  component=opcode {
      opcode,
      pricing_basis?=raw_gas_slope|fixed_per_event,
      interpreter_raw_gas?,
      spawned?,
      dispatch_status=not_applicable|confirmed|selected_not_dispatched
    } | precompile {
      address,
      pricing_basis=raw_gas_slope,
      native_gas
    }
}
TransactionTrace {
  recovered_index, started_tx_index?, manifest_index?, tx_hash, is_anchor,
  disposition=committed_success|committed_failure|attempted|unattempted,
  execution_class?=native_value_transfer|contract_call|contract_create|no_code_no_value|other
}
```

`OperationTrace` rows are emitted by the local REVM inspector. Before the transaction iterator yields
its first item, inspector callbacks are classified as `system`. Each iterator yield sets the current
transaction index and records one started transaction. After Alethia returns, the collector joins
those occurrences with the returned committed transaction list and aligned receipts using an
ordered, one-use subsequence match. Hash equality alone or set membership is invalid because the same
signed transaction can occur more than once. A yielded but
uncommitted transaction is `attempted`; an input transaction never yielded is `unattempted`. This is
an observation of Alethia's existing loop, not a second implementation of its rejection reasons.
A fatal execution writes a failed trace row and never promotes its partial ledger.
For every block,
`input_transaction_count == transactions.len() + recovery_failures_for_block.len()`; the transaction
array includes the anchor and every successfully recovered manifest occurrence exactly once.

The structured native-transfer class is derived after the join from the original non-create
transaction's positive value, the top-level frame/code-execution and precompile-dispatch facts already
visible to the inspector, committed membership, and successful receipt. Trace collection must not add
a database read. A precompile recipient is never a native transfer. Empty calldata or a fixture label
alone is insufficient. Only a committed successful
`native_value_transfer` increments that feature; every yielded transaction increments
`started_transaction_count` exactly once.

Ordinary and non-spawned opcodes record the interpreter step gas and use `raw_gas_slope`.
Precompiles record their completed native gas and use `raw_gas_slope`. At `step_end`, `NewFrame`
creates a pending wrapper row with `selected_not_dispatched` and no `pricing_basis`; it is promoted to
`dispatch_status=confirmed` and `fixed_per_event` only when the following local `call`/`create`
callback proves Alethia actually dispatched that action after charging. A charge rejection clears the
action and leaves an explicit unmeasured `selected_not_dispatched` row. It must not resolve to a
successful fixed-event key. The manifest resolver rejects every row without `pricing_basis` before
candidate math or coverage resolution. Confirmed wrappers omit `interpreter_raw_gas` from all candidate math:
that value includes forwarded child gas, while child execution appears as independent operation rows.
Non-spawned, confirmed-spawn, and rejected-spawn rows therefore remain distinct. The current
schedule's spawn estimate is exported only for provenance and is never used as the measured
fixed-event cost.

Every accepted controlled or proposal trace passes a mandatory fresh-state A/B gate:

1. Execute the ordinary stateless reconstruction with a fresh witness-backed database.
2. Execute the traced reconstruction from the same input bytes with a second fresh database.
3. Require identical committed transaction hashes, receipts/execution result, hashed post-state and
   state root, finalized block zkGas, assembled block, canonical validation result, and proposal
   public output.

The trace pass also requires positive finalized zkGas and exact equality to the assembled block's
`header.difficulty`. A mismatch, zero-valued reconciliation, duplicate operation ID, missing
transaction association, or ambiguous event match rejects the sample. This A/B oracle verifies that
the local inspector did not alter behavior without copying Alethia's charge ledger.

### Controlled Acceptance And SP1 Candidate Tables

The controlled manifest defines the candidate boundary explicitly. Add
`normalization_reference_key = "opcode:0x01"`, a `measurement_keys` array, and controlled overhead
keys with this canonical shape:

```toml
[[measurement_keys]]
id = "opcode:0x01"
production_schedule_key = "opcode:0x01"
event_match = { component = "opcode", opcode = "0x01" }
pricing_basis = "raw_gas_slope"
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

[[measurement_keys]]
id = "opcode:0xf1:spawned"
production_schedule_key = "opcode:0xf1"
event_match = { component = "opcode", opcode = "0xf1", spawned = true, dispatch_status = "confirmed" }
pricing_basis = "fixed_per_event"
required_case_ids = ["call_spawned"]
diagnostic_case_ids = []

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
tx_base                unit=started_non_anchor_transaction
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
CaseSpec.execution_basis: interpreter_raw_gas|native_gas|fixed_per_event
CaseSpec.spawned: bool?                         # CALL/CREATE only
CaseSpec.dispatch_status: confirmed?             # required for spawned=true fixed-event cases
MeasurementKeySpec.id: str
MeasurementKeySpec.production_schedule_key: str
MeasurementKeySpec.event_match: EventMatchSpec
MeasurementKeySpec.pricing_basis: raw_gas_slope|fixed_per_event
MeasurementKeySpec.required_case_ids: tuple[str, ...]
MeasurementKeySpec.diagnostic_case_ids: tuple[str, ...]
EventMatchSpec.component: opcode|precompile
EventMatchSpec.opcode: int?                     # opcode only, 0..255
EventMatchSpec.address: int?                    # precompile only, nonnegative
EventMatchSpec.spawned: bool?                   # CALL/CREATE only
EventMatchSpec.dispatch_status: confirmed?       # required for spawned=true fixed-event keys
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
OverheadKeySpec.unit: proposal|block|started_non_anchor_transaction|native_value_transfer|
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
descriptive only. Ordinary and non-spawned opcode cases require
`pricing_basis = "raw_gas_slope"` and `execution_basis = "interpreter_raw_gas"`; precompile cases
require `pricing_basis = "raw_gas_slope"` and `execution_basis = "native_gas"`. Spawned
CALL/CREATE cases require `pricing_basis = execution_basis = "fixed_per_event"` and
`dispatch_status = "confirmed"`. `spawned` and confirmed dispatch are the only V1 scenario
discriminators read from the local inspector, and every CALL/CREATE measurement key and case must
declare the applicable values. Other opcodes and all precompiles must omit them. A
`selected_not_dispatched` row is always unmeasured in V1. Do not add
warm/cold, state shape, input-size bucket, or another discriminator unless Task 2's local trace schema
contains the exact field and the proposal collector validates it.

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
same tuple. Require the case's `execution_basis` and key's `pricing_basis` to match the rules above,
and for CALL/CREATE require `case.spawned == event_match.spawned` and identical required dispatch
status; a missing value is not a wildcard.
Reject structured context on components that do not support it. The native controlled trace must also
assert that generated execution actually matches the declared opcode/address, pricing basis,
execution basis, and spawned context. A spawned event with forwarded interpreter gas is invalid if
that gas enters slope fitting, ADD normalization, proposal prediction, or coverage. Never use the
free-text `scenario` field in these checks. Freeze the validated mapping in the controlled-manifest
hash before any run.

The existing `sp1-smoke.toml` include flags remain smoke-only conveniences. The final
`sp1-calibration-v1.toml` materializes every generated case, measurement key, overhead key, event
match, unit, and required/diagnostic assignment explicitly. It must be reviewed and hashed before
collecting any controlled or proposal result; result-dependent regeneration is forbidden.

Define `resolve_measurement_key(e)` over one raiko2-local `OperationTrace`. It returns a key only when
exactly one frozen `event_match` and pricing basis succeed. A match that requires `spawned` fails when
the local action context is absent. Zero or multiple matches classify the operation as unmeasured;
report `no_measurement_key` or `ambiguous_measurement_key` rather than falling back to the production
schedule key.

For every exact `(case, target_count, lane=target|control)` run three local SP1 execute repeats.
Require identical `case_input_sha256`, exit code, public values, and `proverGas` across all three
repeats. Define `repeat_noise_p = max(p)-min(p)`; nonzero primary noise rejects the case as
non-deterministic instead of averaging it away. Also record
`repeat_noise_s = max(s)-min(s)`. Nonzero or missing secondary instruction-count data marks that
cycle-cost sample unavailable but does not reject an otherwise valid `proverGas` case.

An ordinary/non-spawned raw-gas opcode slope is eligible only when a host-native REVM trace of the
exact synthetic bytecode proves
that, across variants, the target's executed raw gas is exactly `x*r` while all non-target executed
opcode/precompile counts and raw gas, bytecode length, calldata length, transaction envelope, and
setup state remain fixed. Keep the static bytecode layout fixed and vary only the immediate or input
that selects how many target operations execute. A current template that grows helper opcodes or code
length with `x` is `confounded_template`, not a calibration result; omit it from the candidate table
until it has an isolated scenario.

Each direct-precompile case has a paired control lane in the same synthetic ELF. The control runs the
same loop, branching, input handling, and output folding over deterministic fixture output with the
same `gas_used` and output length, but does not invoke the precompile. Pure opcodes instead emit the
signed formal relation `p_target(x) - p_control(x)` with its exact per-key raw-gas row in `A`; no
pure-opcode fixture creates an absolute candidate coefficient. Define the precompile response as:

```text
z_p,precompile(x) = p_target(x) - p_control(x)
z_s,precompile(x) = s_target(x) - s_control(x)
```

This prevents growing helper bytecode and per-iteration lab overhead from being assigned to the
target multiplier. A missing or mismatched control rejects the precompile case.
`response_repeat_noise_p` and `response_repeat_noise_s` are the target repeat ranges for an opcode and
the sum of target/control repeat ranges for a precompile.

A `fixed_per_event` spawned wrapper uses a paired target/control fixture and is measured only after
all induced child opcode/precompile and nested-wrapper keys have been accepted. The trace must show
exactly `x` additional target wrapper events and enumerate every other operation delta. Subtract each
non-target delta in its own accepted basis exactly once:

```text
z_p,spawn(x) = p_target(x) - p_control(x)
             - sum(delta_raw_operation_gas(k, x) * c_p(k))
             - sum(delta_fixed_operation_count(k, x) * f_p(k) for k != target_key)
z_s,spawn(x) = s_target(x) - s_control(x)
             - sum(delta_raw_operation_gas(k, x) * c_s(k))
             - sum(delta_fixed_operation_count(k, x) * f_s(k) for k != target_key)
```

The target wrapper's forwarded gas is never one of those deltas. An unresolved, recursive, or
rejected dependency makes the case `confounded_template`. Freeze a topological measurement order for
spawn keys and reject cycles. Apply the same slope, repeat, consistency, and out-of-fit checkpoint
gates to `z_p,spawn`; its accepted slope is `f_p` in `proverGas / event`.

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

For an accepted `raw_gas_slope` case with declared per-operation raw gas `r > 0`:

```text
c_p = g_p / r                     # proverGas per raw EVM gas
c_s = g_s / r                     # secondary SP1 instruction count per raw EVM gas
```

Zero, overflow, non-finite primary values, and invalid raw gas reject that case. A bad `c_s` rejects
only the secondary sample. For an accepted `fixed_per_event` case, set `f_p = g_p` and optional
`f_s = g_s`; raw gas is neither required nor consumed. For measurement key `k`, let
`C_required(k)` be the exact manifest-declared `required_case_ids` and let `v_p(case)` be `c_p(case)`
for `raw_gas_slope` or `f_p(case)` for `fixed_per_event`. The key enters the candidate table only
when every member produced an accepted value in the same frozen basis and the primary consistency
check passes:

```text
max(v_p(case) for case in C_required(k)) /
min(v_p(case) for case in C_required(k)) - 1 <= 0.05

```

Use the one required value when there is one; otherwise take the exact arithmetic mean as `c_p(k)` or
`f_p(k)` according to the frozen basis. Compute and store the matching secondary value only when every
corresponding secondary case is available and passes the same 5% consistency diagnostic. A missing,
failed, rejected, or `confounded_template` primary case classifies the entire key as
`required_case_incomplete`; a primary consistency failure classifies it as `scenario_dependent`.
The candidate receives no value for that key. A failed secondary-only check marks the sampling row
unavailable and leaves the primary value unchanged. Accepted diagnostic cases are reported separately
and never define, rescue, or veto a candidate value. Proposal-ledger occurrence does not remove a
valid controlled measurement.

### Controlled Non-Opcode Overhead Acceptance

Measure `proposal_startup`, `block_base`, `tx_base`, and `native_value_transfer` as the four required
V1 non-operation costs. One `tx_base` unit is counted per non-Anchor candidate transaction yielded
to Alethia's executor; the Anchor transaction is owned exclusively by `block_base`,
and its coefficient is
the common per-started-non-Anchor-transaction residual after modeled work is removed; it is not limited to work
that occurs before that event. `native_value_transfer` is the additional exclusive cost for a
committed positive-value transaction that executes no recipient code. Measure
the block residual only after subtracting transaction, transfer, opcode, and precompile work.
Witness-byte, witness-node, stdin-byte, blob-byte, and KZG-invocation cases may be collected only as
V1 diagnostics. These raw features can overlap, so the manifest freezes a residualization DAG rather
than assuming every other quantity can remain physically constant. Each run records every changed
feature count plus raw-gas and fixed-event operation deltas.

V1 assigns the complete pre-transaction `OperationPhase::System`, the Anchor transaction, and the
fixed per-block MPT, trie, and host hashing baseline to `block_base`. Controlled operation deltas
therefore contain only non-Anchor transaction-phase pricing units: interpreter raw gas for ordinary opcodes,
native gas for precompiles, and event count for confirmed spawned wrappers. Zero-gas events contribute
zero units. Proposal execution terms use the same non-Anchor transaction boundary, preventing the
system/Anchor work from being charged once through an operation coefficient and again through
`block_base`. A future V2 may split individual system, trie, Merkle, witness, or hash actions after it
defines independently controlled units; V1 diagnostics do not enter `Q_formula`, the candidate, or
the bridge.

Run the same three deterministic repeats, cumulative-prefix fit, frozen checkpoint mapping, and 10%
out-of-fit APE gate for primary `p` responses.
Measure overhead keys in topological order. For overhead key `q`, subtract its paired control, every
resolved operation delta, and every already-accepted descendant cost exactly once before fitting:

```text
z_p,q(x) = p_target(x) - p_control(x)
         - sum(delta_raw_operation_gas(k, x) * c_p(k))
         - sum(delta_fixed_operation_count(k, x) * f_p(k))
         - sum(delta_feature(t, x) * o_p(t) for t in subtract_closure(q))

z_s,q(x) = s_target(x) - s_control(x)
         - sum(delta_raw_operation_gas(k, x) * c_s(k))
         - sum(delta_fixed_operation_count(k, x) * f_s(k))
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
                      - started_non_anchor_transaction_count(h) * o_p(tx_base)
                      - native_value_transfer_count(h) * o_p(native_value_transfer)
                      - sum(raw_operation_gas(e) * c_p(resolve_measurement_key(e))
                            for non-Anchor transaction-phase e in h)
                      - sum(f_p(resolve_measurement_key(e))
                            for non-Anchor transaction-phase fixed_per_event e in h)

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
- `c_p(k)` for every accepted raw-gas opcode/precompile key;
- `f_p(k)` for every accepted spawned CALL/CREATE fixed-event key;
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
execution_raw_evm_gas(e) = e.interpreter_raw_gas for a raw_gas_slope opcode OperationTrace
execution_raw_evm_gas(e) = e.native_gas for a raw_gas_slope precompile OperationTrace

p_hat(j) = sum(execution_raw_evm_gas(e) * c_p(resolve_measurement_key(e))
               for non-Anchor transaction-phase e
               where basis(resolve_measurement_key(e)) == raw_gas_slope)
         + sum(f_p(resolve_measurement_key(e))
               for non-Anchor transaction-phase e
               where basis(resolve_measurement_key(e)) == fixed_per_event)
         + sum(controlled_feature(j, q) * o_p(q) for q in Q_formula)

APE_main(j) = abs(p_hat(j) - p(j)) / p(j)
```

`Q_formula` is exactly `[proposal_startup, block_base, tx_base, native_value_transfer]` in canonical
order. Their proposal feature counts are respectively one, successful block count,
started non-Anchor candidate transaction iterator-yield count, and committed structured
native-value-transfer count.
Every key and
transitive subtraction dependency must be accepted before sealing. Diagnostic
or bundled keys never appear independently in the sum. This same exclusive feature basis is used in
controlled fixtures and proposal extraction, so witness/blob bytes are not also charged as full stdin
bytes and block/transaction features do not repeat operation work.

The execution sums include resolved non-Anchor transaction-phase operations in committed and
attempted work. Pre-execution system work and the Anchor transaction are owned exclusively by
`block_base`; a double-counting guard must
reject any extractor that also places it in an execution sum. The sums exclude intrinsic or
pre-validation failures, the unattempted truncation tail, and
missing/rejected/ambiguous keys. Spawned CALL/CREATE is included only as one `fixed_per_event` term
when the frozen event match and basis resolve exactly; its forwarded interpreter gas is never added.
Current intrinsic charges, spawn estimates, failsafe values, block cap, and current multipliers remain
provenance metadata and never enter the prediction.

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

For every proposal, report operation-count coverage over all executed operations, execution-raw-gas
coverage over only raw-gas-basis events, and spawned-event count coverage over only fixed-event
wrappers. Forwarded gas never enters a denominator. A zero family denominator is
`coverage_unavailable`, never 100%; an absent optional family does not fail the proposal. Overall
operation-count and raw-gas denominators must remain positive. Coverage is descriptive: unsupported
keys remain visible and are not priced with the current table.

For `p_hat` against observed `p`, compute every per-proposal APE, combined/per-network MAPE, maximum
error, and the 10% pass/fail decision from `APE_main` with 80-digit `Decimal` arithmetic. MAPE is the
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
`--proof-type sp1 --validate`, network-specific L1/L2 RPC inputs, and an explicit `--output`.
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

Add `prepare-calibration` to write `runs/<calibration-id>/experiment.json` from one clean
`implementation_revision`, dirty-state flag, Alethia/reth revisions, Rust/SP1 SDK versions,
controlled manifest hash, SP1 ELF/VK hashes, exact guest-launcher binary hash, SP1 execution
parameters, complete schedule hash, ADD
normalization reference, primary formulas and quality gates, the exact out-of-fit checkpoint mapping
and threshold, the workload-identity schema/version and canonicalization rule, and the exact
`bridge_key_ids`, bridge model, bridge thresholds, and missing-data rules. It also materializes the
immutable `bridge-manifest.json` before any measurement.
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

`prepare-validation` must require candidate, bridge, and validation provenance to name the same
`implementation_revision`, and require current `HEAD` to remain that revision. After controlled
measurement begins, only the declared run, corpus, proposal-manifest, and validation output paths may
be dirty; any implementation, dependency, controlled-manifest, or guest-artifact change rejects the
run. Generated outputs remain uncommitted until final verification completes.

Final validation acceptance additionally requires positive proposal `proverGas`, matching trace/SP1
public output, exact nonzero A/B block reconciliation from Task 2, at least one `OperationTrace` row,
and positive required coverage denominators in every proposal. Record a missing or non-positive SP1
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

## Task 2: Land Minimal Spawn Parity And Add The Host-Only Trace Crate

This is the high-risk implementation unit. First shrink Alethia PR #239 to the minimal existing-
inspector parity fix and merge that reviewed change into Alethia `main`. Then pin raiko2 to that
commit and add all experiment tracing in raiko2. Never pin a PR branch or retain the removed Alethia
observer surface as a compatibility path.

**Alethia files:**

- Modify: `crates/evm/src/zk_gas/{adapter.rs,runtime.rs,tests.rs}`
- Do not retain changes to `meter.rs`, `alloy.rs`, factory code, block code, or Cargo features
- Delete/revert from PR #239: observer modules, event schema, block-executor lifecycle wiring,
  observed derived-block entrypoints, observer feature flags, and serialization documentation

**Raiko2 files after the Alethia merge:**

- Modify: every direct Alethia/reth `rev` pin in root, crate, xtask, and guest manifests
- Modify: `Cargo.toml`, `Cargo.lock`, `guests/sp1/Cargo.lock`, and `guests/risc0/Cargo.lock`
- Regenerate, never hand-edit: SP1 ELF/VK and RISC0 ELF plus both provenance files under
  `crates/guests/elf/`
- Create: `crates/zkgas-trace/Cargo.toml`
- Create: `crates/zkgas-trace/src/{lib.rs,inspector.rs,transactions.rs,reconstruct.rs}`
- Create: `crates/zkgas-trace/tests/{inspector.rs,reconstruction.rs}`
- Modify: `crates/stateless/src/{lib.rs,validation.rs}`
- Modify: `crates/guest-common/src/lib.rs`
- Modify: `bin/guest-launcher/{Cargo.toml,src/main.rs}`
- Create: `bin/guest-launcher/tests/proposal_trace.rs`
- Modify: `experiments/opcode-gas/opcode_gas.py`

### Step 1: Write The Failing Alethia Parity Tests

Add focused tests for CALL, CALLCODE, DELEGATECALL, STATICCALL, CREATE, and CREATE2. For each family,
cover `NewFrame` and non-spawn actions. Prove that the plain metered path and the existing
`ZkGasInspector` path select the same raw-gas source at the same `step_end` boundary, before child or
precompile dispatch. At a near-limit boundary, prove a rejected spawned wrapper clears/replaces the
pending `NewFrame`, produces the same fatal outcome in both paths, and executes no child work. Cover
a successful child, precompile dispatch plus its separate native charge, an ordinary dynamic opcode,
and a custom inner inspector so the existing public composition contract remains intact.

These tests assert execution and finalized zkGas parity only. They introduce no observer callbacks,
event schema, transaction lifecycle, serialization, or new feature matrix.

### Step 2: Reduce Alethia PR #239 To The Parity Fix

In `ZkGasInspector::step_end`, inspect the already-selected interpreter action. Charge the fixed
production spawn estimate immediately for `NewFrame`; otherwise charge the actual interpreter step
gas. On failure, use one shared helper that removes the pending frame action before invoking the fatal
halt. Use the same helper in `run_metered_plain` so debug and release behavior match. Remove deferred
CALL/CREATE tracking and any observer-only formula copy; the existing meter remains the only schedule
and arithmetic source of truth.

Remove every #239 addition outside this boundary, including `ExecutionObserver`, `ExecutionEvent`,
executor transaction classifications, phase/block events, observed derived-block functions, serde
types, and `execution-observer` Cargo features. Restore the original lazy transaction iteration; do
not retain the observer branch's eager `collect`, because the raiko2 tracing iterator uses each
`next()` call as the existing executor's started-transaction boundary. Run formatting, the focused
zkGas tests, and clippy for every feature combination touched by the final small diff. Commit the
scope reduction normally; do
not force-push or rewrite PR history without separate authorization. Update the PR title/body to
describe only spawn timing and halt parity, then obtain review and merge it into Alethia `main`.

### Step 3: Update The Pin And Add The Shared Reconstruction Seam

Only after the Alethia merge, update every direct pin to the resulting `main` commit, regenerate all
tracked lockfiles, and inspect `cargo tree -d`. Verify both production guest paths resolve that exact
commit. Do not hide an incompatible mixed graph with a local path override or raiko2-side adapter.

Write failing `raiko2-stateless` tests, then add one narrow injection seam: the current public
reconstruction function uses an internal default implementation of a public `DerivedBlockExecutor`
trait, while a sibling
`reconstruct_block_from_transactions_with_executor_and_witness_resources` accepts another
implementation. The trait's generic `execute<DB: Database + Debug>` method receives the prepared
parent header, derived block, witness-backed database, and `TaikoEvmConfig`, and returns Alethia's
existing `DerivedBlockExecutionOutcome`. This keeps sparse-state creation, state-root calculation,
block assembly, and consensus/post-state validation in their current single implementation.

```rust
pub trait DerivedBlockExecutor {
    fn execute<DB: Database + Debug>(
        &mut self,
        evm_config: &TaikoEvmConfig,
        parent_header: &SealedHeader,
        derived_block: &RecoveredBlock<Block>,
        db: DB,
    ) -> Result<DerivedBlockExecutionOutcome, BlockExecutionError>;
}
```

The sibling reconstruction function receives `executor: &mut E where E: DerivedBlockExecutor`;
all other arguments and its `FilteredBlockExecutionOutcome` result match the existing function.

Write failing `raiko2-guest-common` tests, then refactor the already-existing Shasta block-verifier
seam into one narrow public callback API:

```rust
pub struct ShastaBlockReconstruction<'a> {
    pub index: usize,
    pub anchor_tx: Recovered<TransactionSigned>,
    pub transactions: Vec<TransactionSigned>,
    pub block_env: TaikoNextBlockEnvAttributes,
    pub witness: &'a ExecutionWitness,
    pub ancestor_headers: &'a [WitnessHeader],
    pub shared_state_nodes: &'a [WitnessStateNode],
    pub chain_spec: &'a Arc<TaikoChainSpec>,
    pub evm_config: &'a TaikoEvmConfig,
}

pub fn prove_shasta_proposal_with_reconstructor<R>(
    guest_input: &GuestInput,
    reconstruct: R,
) -> Result<B256>
where
    R: for<'a> FnMut(ShastaBlockReconstruction<'a>)
        -> Result<FilteredBlockExecutionOutcome>;
```

The wrapper, not the callback, continues to own Shasta derivation, manifest transaction conversion,
canonical-block comparison, ancestor-window update, and public-output construction. Default
`prove_shasta_proposal` calls it with the ordinary stateless reconstructor. Add tests proving the
default path returns identical errors and outputs and that a callback cannot bypass canonical
comparison.

After those shared guest-path changes pass their behavior tests, complete every Cargo-graph edit
required by the host-only trace integration before recording the artifact baseline: register
`raiko2-zkgas-trace` as a workspace member, declare all of its workspace dependencies, add the
guest-launcher dependency, regenerate the root lockfile, and create the minimal compilable crate
shell. Confirm that neither guest dependency tree contains the new crate. No `Cargo.toml`, lockfile,
workspace-member, or workspace-dependency edit may occur after this point without taking a new
baseline.

This ordering is required because guest provenance unconditionally fingerprints the root
`Cargo.toml`, even when a new workspace member is absent from the guest dependency graph. Build both
guests and record the post-manifest baseline. Then add only trace-crate implementation/test source,
guest-launcher source/tests, and experiment analysis code. Because those later files remain outside
the guest provenance input set, build both guests again and require byte-identical ELF/VK, image-ID,
and provenance results:

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
```

Equality with artifacts from the older Alethia revision or before the shared seam refactor is not
required. The shared seam instead has behavior/output compatibility tests. Byte equality is required
only between the post-seam baseline and the later trace-crate/guest-launcher-only change.

### Step 4: Write Failing Raiko2 Trace Tests

In `raiko2-zkgas-trace`, write failing tests for:

- ordinary and dynamic opcode gas, precompile native gas, and all six spawned/non-spawned families;
- spawned wrappers serialized as `fixed_per_event` with no usable raw-gas field;
- confirmed child/precompile dispatch promoting a pending `NewFrame` wrapper, while near-limit charge
  rejection leaves `selected_not_dispatched` with no `pricing_basis`, unmeasured, and executes no child work;
- child and precompile work emitted independently without wrapper double-counting;
- system operations before the first iterator yield, transaction index changes, committed success,
  committed failure (revert or halt), attempted work, and unattempted tail classification;
- two adjacent transactions with distinct opcode traces, proving iterator consumption remains
  interleaved with execution rather than eagerly collected;
- positive-value no-code success as the only native-transfer class, with no trace-only DB read;
  explicitly reject contract, create, precompile, reverted, attempted, and zero-value rows;
- deterministic monotonic operation IDs and exact event-match fields;
- duplicate signed-transaction hashes joined as an ordered one-use subsequence, including the pattern
  attempted duplicate, intervening commit, then committed duplicate;
- fatal execution retaining diagnostics but rejecting the partial trace;
- fresh-state ordinary/traced equality for committed hashes, receipts/result, hashed state and state
  root, finalized zkGas, assembled/canonical block, and public output;
- deliberate inspector perturbation making the A/B gate fail.

### Step 5: Implement The Host-Only Trace Crate

Implement `TraceInspector` around a shared in-memory sink. Snapshot opcode and remaining gas in
`step`, compute ordinary step gas in `step_end`, and read `InterpreterAction::NewFrame` there. Emit a
raw-gas operation for ordinary/non-spawned opcodes. Keep `NewFrame` wrappers pending by frame depth;
promote one to a fixed-event operation only from its following `call`/`create` callback. Clear an
unconfirmed pending row as `selected_not_dispatched` with `pricing_basis = None` at the next step,
frame end, transaction boundary, or fatal finalization. Never serialize forwarded gas as a candidate
input. Record completed
precompile native gas through the existing inspector callbacks as a separate operation.

Implement `TracingTransactions<I>` over the exact recovered transaction iterator passed to Alethia.
Its `next` method finalizes pending state for the preceding occurrence, sets the shared transaction
index, and records one started `(recovered_index, manifest_index, hash)` occurrence. Unattempted rows
retain `recovered_index` but have no `started_tx_index`. After the unchanged Alethia
executor returns, scan started occurrences and the ordered committed list with a single committed
cursor. Consume a committed occurrence and its aligned receipt only when it is the next subsequence
match; otherwise classify that started occurrence as attempted. Reject an unconsumed committed row or
receipt-count mismatch. The untouched input suffix is unattempted. Preserve every signer-recovery
failure as a proposal-level `RecoveryFailure` before reconstruction. Do not invent Alethia rejection
reasons.

Implement `TracingDerivedBlockExecutor` using `TaikoEvmFactory::create_evm_with_inspector`,
`TaikoBlockExecutor::new`, and
`execute_block_with_committed_transactions(TracingTransactions::new(...))`. Reuse the same
environment/context construction and outcome finalization as the ordinary Alethia helper. The crate
may reproduce this thin EVM-construction adapter; it must not copy the transaction/filter/reset/commit
loop or any schedule/meter formula.

Implement `trace_shasta_proposal` as two complete calls to
`prove_shasta_proposal_with_reconstructor` from identical GuestInput bytes and fresh witness state.
Each callback projects its returned `FilteredBlockExecutionOutcome` into this in-memory typed equality
record; no debug formatting, map serialization, or implementation-defined digest is allowed:

```text
ExecutionParityRecord {
  committed_transaction_hashes: Vec<B256>,
  execution_result: BlockExecutionResult<Receipt>,
  hashed_state: HashedPostState,
  assembled_block: Block,
  assembled_senders: Vec<Address>,
  state_root: B256,
  finalized_block_zkgas: u64
}
```

The first callback uses the ordinary reconstructor; the second uses `TracingDerivedBlockExecutor` and
also records trace rows. Compare every field with Rust typed equality in block order, and compare the
two final public outputs, before returning normalized trace JSON. Persist only the explicit A/B pass
status and mismatch field names, not a made-up parity hash. Add `guest-launcher proposal-trace` to emit
that JSON plus exact bincode input length,
`guest_input_sha256`, and public output. The normal local SP1 execution remains uninstrumented. Join
trace and SP1 rows only on GuestInput hash and matching public output; reject proposal-ID-only joins.

### Step 6: Verify Alethia And Raiko2

Run the focused upstream Alethia tests and supported-feature clippy checks, then in raiko2:

```bash
cargo fmt --all -- --check
cargo test -p raiko2-stateless
cargo test -p raiko2-guest-common proposal
cargo test -p raiko2-zkgas-trace
cargo test -p guest-launcher proposal_trace
cargo clippy -p raiko2-stateless -p raiko2-guest-common \
  -p raiko2-zkgas-trace -p guest-launcher -- -D warnings
just build-guest sp1
just build-guest risc0
cargo run -p xtask --features guest-tools -- guest-digests \
  --output target/zkgas-guest-digests.json
```

Before the final corpus is sealed, acquire one temporary post-Unzen candidate from each network for
this integration smoke test. Record `purpose = "integration_smoke"` before execution and reject any
candidate whose `(network, proposal_id)` belongs to the predetermined 60-row `final_validation`
selection; choose another unexecuted smoke candidate before running instead. Do not add the temporary
files or results to the final manifest, candidate, bridge, or report, and never relabel them
`final_validation`. Run both through the trace and SP1 passes. Require matching GuestInput hashes and
canonical/public outputs, identical pre/post host-feature guest identities for both zkVM builds,
positive `p`, recorded secondary `s` status, and exact nonzero A/B reconciliation for every block.
The RISC0 requirement here is build/artifact isolation, not proposal execution.

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

### Approved Replacement: Relative Relations And Joint Block Fit

This replacement supersedes the sequential overhead-residualization instructions in the historical
Task 3 detail below. Implement and test the following primary path instead:

1. Derive the frozen 102-key matched-control relation matrix from per-key actual raw-gas totals;
   require 98 independent nonzero rows, exactly the four natural anchors, signed relation gates, and
   dynamic-gas canonical plus holdout scenarios. Early-STOP controls remain non-candidate.
2. Derive `mu_zero` and `B` from the accepted relation matrix, validating rank and anchor ordering
   from the manifest. Do not hand-code a relation offset or fit a relation from a block/proposal row.
3. Materialize predeclared post-Unzen `sp1-shasta-proposal` block fit and holdout rows. Require exact
   rank eight before SP1 execution, three identical repeats, declared workload-family coverage, and
   no precompile, spawned-wrapper, system/Anchor ownership violation, unresolved operation, or
   dynamic-gas variation in anchor-fit rows.
4. Fit `[theta, beta]` with Decimal OLS using
   `p_hat_j = x_j * mu_zero + [x_j * B, q_j] * [theta, beta]`, reconstruct all opcode multipliers,
   apply dynamic holdouts without refitting, and reject non-positive values, fit/holdout failures, or
   leave-one-family-out instability.
5. Measure precompile and confirmed fixed-event wrapper components independently, then write exactly
   `opcode-relations.json`, `block-calibration-rows.jsonl`, `block-calibration.json`,
   `controlled-fit.json`, and the candidate manifest/digest sequence recorded above. The candidate
   root binds provenance, coverage, bridge isolation, all primary artifacts, thresholds, and formula.
6. Refuse to acquire, execute, read, or use final proposal results until `candidate.sha256` exists;
   proposal validation is forward-only and cannot repair the candidate.

The remaining historical detail is retained only for reusable trace, provenance, bridge, and
proposal-barrier requirements. Where it conflicts with this replacement, the replacement controls.

### Step 1: Test The Synthetic Gates

Write failing manifest tests for duplicate measurement-key IDs, missing production schedule keys,
empty or unknown required case IDs, a case listed twice or as both required and diagnostic, unsupported
event-match fields, overlapping matches, and `spawned` on a non-CALL/CREATE opcode. Prove that a
generic CALL/CREATE match without `spawned` is rejected and that explicit `spawned=true` and
`spawned=false` matches are disjoint. Reject a missing/unknown pricing basis, raw-gas basis on a
spawned key, fixed-event basis on an ordinary opcode or precompile, and any raw-gas field attached to
a fixed-event proposal term. Require `dispatch_status=confirmed` on every `spawned=true`
fixed-event key/case and reject any key that attempts to price `selected_not_dispatched`.

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
Freeze `system_operation_ownership = "block_base"` and
`anchor_operation_ownership = "block_base"`. Assert that overhead operation deltas use only
non-Anchor transaction-phase pricing units, that raw-gas cases accumulate raw gas rather than event count, that
zero-gas events are omitted, and that system/Anchor operations cannot be counted again in proposal
execution terms.

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

Cover exact `g_p/r` conversion without integer rounding for raw-gas keys, exact `f_p=g_p` for
fixed-event keys, and primary zero/non-finite/overflow rejection. For one measurement key in each
basis, test all required primary cases accepted within the 5% consistency gate, the exact mean, one
required case missing/rejected/confounded, and a primary value outside the consistency gate. The
latter cases exclude the complete key as
`required_case_incomplete` or `scenario_dependent`; no successful sibling case may enter alone. Test
that accepted/rejected diagnostic cases remain reported but do not define, rescue, or veto the
primary value.

Assert that ADD is the frozen normalization reference, that a failed primary ADD measurement prevents
candidate sealing, that exact `m_p(k)=c_p(k)/c_p(ADD)` preserves raw `c_p`, and that no `f_p` is ADD
normalized. Test the four required
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
fixture that would lose significance through `float` and assert exact 80-digit `Decimal` slopes,
costs, normalization, primary prediction, and serialized-string reproduction.

### Step 3: Implement Controlled Runs And Candidate Construction

Add only the next declared count level when the current prefix fails a quality gate. Persist all
repeats and decisions so resume cannot silently choose a different prefix. Emit exact primary `g_p`,
per-case `c_p|f_p`, every frozen measurement key, pricing basis, and event match,
required/diagnostic membership, selected per-key `c_p(k)|f_p(k)`, raw-gas-only ADD-normalized `m_p`,
checkpoint observations/predictions/APE/status,
accepted/rejected evidence, and coverage gaps. Run
the controlled overhead cases and emit exact `o_p`, including the fixed startup residual. Write the
simultaneously observed `s` values and available `g_s/c_s/o_s` diagnostics to the separately hashed
sampling artifact. From those controlled samples only, build the frozen through-origin bridge,
including its ratios, `kappa_sp1`, controlled predictions, diagnostics, status, canonical manifest,
and independent digest. Emit the separately hashed diagnostic overhead cost table. Do not emit an
integer schedule or Rust table. Materialize and review
`sp1-calibration-v1.toml` without the
smoke manifest's implicit include flags before running any measurement. Changing the synthetic lab
guest is allowed for isolation/control scenarios; it does not add the host trace crate to the
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

Write failing tests for transaction iterator boundaries, attempted/committed recomputation and
system-to-block ownership,
ordinary/traced difficulty and complete A/B parity, exact trace/SP1 GuestInput and public-output
joins, and positive `p`. Record missing/non-positive `s` as a secondary-sample failure. Cover
prediction inclusion for executed non-Anchor transaction-phase committed/attempted operations,
exclusive `block_base` ownership of system and Anchor operations, and exclusion for intrinsic/
pre-validation failure, unattempted work, unmeasured keys, and pricing-basis mismatch. Reject any use
of current zkGas multipliers, fixed spawn estimates, or spawned forwarded gas as proving-gas
coefficients.

Test exact measurement-key resolution from one local `OperationTrace`: one match enters the frozen
prediction; no match, missing required spawn context, and `selected_not_dispatched` enter unmeasured
coverage; ambiguous matches reject the manifest before execution. With two disjoint
CALL/CREATE spawned/non-spawned measurement keys, prove that failure of one required set excludes only
that measurement key. Separately prove the required P1 case: when two required scenarios share one
unresolvable measurement key and one is accepted while the other is rejected, the complete key stays
out of `K` and every matching proposal operation enters unmeasured coverage.

Test exact feature extraction for proposal startup, block base, started non-Anchor candidate
transactions, and committed
native-value transfers, plus diagnostic blob-byte and KZG-invocation counts. Test the fixed `p_hat`
formula and exact `APE_main = abs(p_hat-p)/p` denominator using 80-digit `Decimal`. Prove MAPE is the
arithmetic mean of proposal APE values rather than aggregate-gas error. Test metric signs,
per-network aggregation, worst-row ordering, operation-count, raw-gas, and spawned-event coverage,
required zero-denominator rejection, optional absent-family handling, the exact 10% boundary, the two
primary validation classifications, and
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

Test that candidate, bridge, and validation all retain the original `implementation_revision` while
generated experiment paths are dirty. Reject a changed `HEAD` or any dirty implementation,
dependency, controlled-manifest, or guest-artifact path. No generated corpus or report commit occurs
before final verification.

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
declared required key, build the raw-gas native and ADD-normalized views plus fixed spawn-event
`proverGas` costs, write the separate
controlled cycle-cost samples and diagnostic overhead cost table, build the controlled bridge, and
seal the independent candidate and bridge digests. An `insufficient_data` bridge is allowed; a missing
or unsealed bridge is not. No proposal fixture or result may be read during this step.

### Step 2: Acquire And Seal The Final-Validation Corpus

Only after Step 1 has sealed `candidate.sha256` and `bridge.sha256`, invoke `prepare-corpus` with the
final binaries and network RPC inputs, inspect its 60 `final_validation` rows, publish the
content-addressed archive with `publish-corpus`, and read back its exact GCS generation and bytes.
Keep the version-qualified manifest and every generated experiment artifact uncommitted through
final verification so `HEAD` remains the clean `implementation_revision`; fixtures and the local
archive remain ignored. `prepare-validation` binds the exact candidate and bridge digests, corpus
generation, GuestInput hashes, SP1 guest identity, and that implementation revision to an immutable
validation ID.

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

Apply the sealed `c_p`, `f_p`, and `o_p` values directly to every frozen proposal ledger and feature
row. Emit the exact predicted/observed `proverGas`, per-key required/diagnostic outcomes,
operation-count, raw-gas, and spawned-event coverage, missing-key and feature diagnostics,
classification, rejection reasons,
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
cargo test -p raiko2-guest-common proposal
cargo test -p raiko2-zkgas-trace
cargo test -p guest-launcher
cargo clippy -p raiko2-stateless -p raiko2-guest-common \
  -p raiko2-zkgas-trace -p guest-launcher -- -D warnings
just build-guest sp1
just build-guest risc0
cargo run -p xtask --features guest-tools -- guest-digests \
  --output target/zkgas-guest-digests.json
```

Independently review the complete Alethia and raiko2 diffs. Independently rerun the Alethia spawn
parity tests, the raiko2 fresh-state A/B tests, one Mainnet and one Hoodi full trace-plus-SP1
reconciliation, both guest feature/artifact
isolation checks, one exact-generation corpus download/hash check, and a fresh recomputation of both
the candidate and validation reports from sealed normalized rows. Investigate every material
finding and have the reviewer/tester re-check fixes before declaring the reports fixed.

After those checks pass, commit the version-qualified corpus manifest and the declared calibration,
validation, and report artifacts as experiment evidence. They continue to record the unchanged
`implementation_revision`; the evidence commit is not substituted as the revision that executed the
experiment.

### Step 6: Handoff Boundary

The handoff contains the calibration and validation IDs, candidate digest, exact source/dependency/
SP1/schedule/corpus identities, the RISC0 release-consistency artifact identity, commands and results,
raw and ADD-normalized `proverGas` table, fixed spawn-event costs, controlled fixed/base costs,
separately hashed controlled
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
