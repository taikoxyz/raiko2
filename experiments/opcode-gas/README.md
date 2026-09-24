# Opcode Workload Metric Experiment

This experiment suite is a local, repeatable scaffold for measuring opcode and precompile
contribution to zkVM workload metrics. The direct opcode/precompile lab currently runs on SP1
software `proverGas`; opcode fixtures can run through either the fast synthetic guest or a
revm-backed guest; proposal-level GuestInput runs can also collect RISC0 cycle metrics.

V1 intentionally exports a one-dimensional result shape compatible with the current alethia-reth
Unzen table. Warm/cold storage access, argument-dependent precompile cost, and memory-size sweeps are
tracked as future dimensions, not V1 coefficients.

The multiplier source of truth is the Cargo-pinned
`alethia-reth-evm::zk_gas::unzen::UNZEN_ZK_GAS_SCHEDULE`. Python keeps only experiment metadata
such as names, categories, and fixture templates; it does not duplicate the pricing table. Commands
that need the active schedule obtain it at runtime through:

```bash
cargo run --quiet --locked -p xtask --no-default-features -- export-unzen-zk-gas-schedule
```

`generate` invokes the exporter when a manifest enables schedule-driven expansion, while `damage`
and `inventory` always invoke it. The first such command may pay the one-time Cargo dependency build
cost; subsequent invocations use Cargo's incremental cache. `run`, `run-proposal`, and `fit` do not
load the schedule.

## Commands

Build or refresh the SP1 lab ELFs, then build a release launcher once:

```bash
cargo run -r -p xtask -- build-guest sp1 --bench
cargo build -r -p guest-launcher --features sp1-sdk/profiling
```

### Synthetic Four-Anchor Ratio Probe

The dedicated `sp1-opcode-lab` guest has an independent diagnostic mode for the four free relation
anchors `POP`, `PUSH0`, `DUP1`, and `SWAP1`. It is not the REVM transaction guest and does not
produce production opcode costs or a candidate table. It calls REVM's actual `Stack` primitives in
a synthetic loop, so its accepted output is a frozen body-cost prior. The authoritative rank-98
real-REVM relation system and production controlled-block validation remain mandatory. The
production fit consumes the four per-operation slopes only through two shared transfer parameters:
`theta_i = (body_scale * synthetic_body_cost_i + common_opcode_overhead) / raw_gas_i`.

Generate, run, and fit it only inside a freshly frozen calibration run, using the canonical commands
below. The gas estimator is host code, so the evidence binds the calibration revision, SP1 SDK
version, exact guest-launcher binary digest, fixture manifest, raw rows, and opcode-lab ELF. An
artifact copied from an older or standalone host run is diagnostic only and is rejected by the
candidate path.

The frozen fit counts are `[0, 1024, 4096, 16384, 65536]`, the predeclared checkpoint is `131072`,
and every lane has three exact repeats. The probe keeps the historical `OpcodeLabInput` wire schema:
the dedicated opcode-lab binary decodes the equal-length scenarios `anchor_target_` and
`anchor_control` only when the exact case/opcode/envelope declaration also matches. This binary-only
path is deliberate; adding probe state to shared primitives or the shared guest library changes
unrelated proposal, precompile, and REVM guest programs. Serialized bincode length and bytecode
remain fixed across counts; `target_count` is the only count-varying workload value. Both lanes perform the same stack
seeding, loop bookkeeping, one indirect step call, compiler barrier, accumulator mix, and fixed-size
public-output preparation. Lane/opcode selection occurs once before the loop. The control step is a
no-op; the target step calls exactly one REVM `Stack::{pop,push,dup,swap}` primitive.

For each count, define `delta(n) = P_target(n) - P_control(n)` and fit it with a free intercept. This
removes common fixed guest startup from the slope; equivalently, the slope sees
`[P_target(n)-P_target(0)] - [P_control(n)-P_control(0)]`. A nonzero `delta(0)` is allowed fixed lane
overhead, not per-operation cost. The fit requires positive signal, `R2 >= 0.99`, relative slope
standard error at most `0.05`, residual/signal and count-zero intercept residual/signal at most
`0.02`, and marginal checkpoint APE at most `0.10`, where
`predicted = slope * checkpoint_count` and
`observed = delta(checkpoint_count) - fitted_intercept`. The denominator is this positive observed
marginal signal, not total proverGas, so fixed startup cannot hide nonlinear extrapolation. Output fields are
`prover_gas_per_operation` and `prover_gas_per_raw_gas`, with explicit
`synthetic_prior_only=true` and `candidate_eligible=false`.

Target/control subtraction and the free intercept remove fixed ELF startup and fixed lane overhead.
They do not include the production interpreter's per-opcode dispatch/wrapper path because the lab
calls `Stack` primitives directly. `common_opcode_overhead` represents that repeated production
work; it is distinct from startup and from the four block/base costs. A one-scale transfer was
tested and rejected because it could not keep all reconstructed multipliers positive while meeting
the controlled-family error gates.

The old variable-bytecode sweeps are not substitutes. A free intercept removes fixed startup, but
their bytecode, serialized input/deserialization, and helper work still grow with count and enter the
slope. The launcher also requires an opcode-lab guest exit code of zero before single or batch
reports can be published; an obsolete ELF that exits nonzero with empty public values fails closed
instead of becoming a zero-slope row.

First freeze the controlled manifest. `CALIBRATION_RUN` must be the exact directory emitted by
`prepare-calibration`; do not discover the newest run directory. The machine-readable
`--run-path-file` flow captures that directory for every subsequent command:

```bash
RUN_PATH_FILE="$(mktemp)"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-calibration \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --guest-launcher target/release/guest-launcher \
  --out experiments/opcode-gas \
  --run-path-file "$RUN_PATH_FILE"
CALIBRATION_RUN="$(<"$RUN_PATH_FILE")"
```

The candidate-bearing probe must be generated after that freeze and persisted inside the exact run.
The consumer checks that its ELF hash equals the `sp1_opcode_lab.elf` hash frozen in
`experiment.json`; the `/tmp` diagnostic above is not accepted as a substitute for this canonical
artifact path:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate-anchor-probe \
  --calibration-run "$CALIBRATION_RUN" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_opcode_lab.elf \
  --out "$CALIBRATION_RUN/generated/anchor-probe"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-anchor-probe \
  --calibration-run "$CALIBRATION_RUN" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_opcode_lab.elf \
  --fixtures "$CALIBRATION_RUN/generated/anchor-probe" \
  --out "$CALIBRATION_RUN/raw/anchor-probe.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-anchor-probe \
  --calibration-run "$CALIBRATION_RUN" \
  --runs "$CALIBRATION_RUN/raw/anchor-probe.jsonl" \
  --out "$CALIBRATION_RUN/anchor-probe-fit.json"
```

### Authoritative Relative-Relation And Block-Calibration Flow

Run these commands in this order. They use the exact `CALIBRATION_RUN` emitted above; do not replace
it with a discovered or manually selected run directory.

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate-relations \
  --manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --calibration-run "$CALIBRATION_RUN" \
  --out "$CALIBRATION_RUN/generated/formal-relations"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-relations \
  --fixtures "$CALIBRATION_RUN/generated/formal-relations" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_revm_opcode_lab.elf \
  --calibration-run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/raw/formal-relations.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-relations \
  --runs "$CALIBRATION_RUN/raw/formal-relations.jsonl" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/opcode-relations.json"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-block-calibration \
  --guest-launcher target/release/guest-launcher \
  --calibration-run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --anchor-probe "$CALIBRATION_RUN/anchor-probe-fit.json" \
  --out "$CALIBRATION_RUN/block-calibration-rows.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-dynamic-opcode-models \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --anchor-probe "$CALIBRATION_RUN/anchor-probe-fit.json" \
  --runs "$CALIBRATION_RUN/block-calibration-rows.jsonl" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/dynamic-opcode-models.json"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-block-calibration \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --anchor-probe "$CALIBRATION_RUN/anchor-probe-fit.json" \
  --runs "$CALIBRATION_RUN/block-calibration-rows.jsonl" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/block-calibration.json"
```

`fit-dynamic-opcode-models` is a non-candidate diagnostic. The first completed calibration,
re-evaluated with anchor body costs divided by anchor raw gas in the opcode-lab unit system, showed
that one scalar lab-body `proverGas / raw EVM gas` multiplier does not describe `EXP`, `KECCAK256`,
`MLOAD`, `MSTORE`, `MSTORE8`, or `MCOPY` across their frozen scenarios. The command therefore
recovers each target opcode's body cost from its signed target/control relation and fits a frozen
semantic model:

```text
f_mem     = beta_event * memory_growth_event
          + beta_evm * memory_evm_gas_delta
          + beta_boundary * memory_4k_boundary_event
EXP        = beta_0 + beta_b * exponent_bytes + beta_b2 * exponent_bytes^2
keccak_permutations = 0 if input_length == 0 else floor(input_length / 136) + 1
keccak_zero_length_event = 1 if input_length == 0 else 0
KECCAK256  = beta_0 + beta_z * keccak_zero_length_event
           + beta_p * keccak_permutations + f_mem
MLOAD      = beta_load + f_mem
MSTORE     = beta_store + f_mem
MSTORE8    = beta_store8 + f_mem
MCOPY      = beta_0 + beta_w * copy_words + f_mem

production_cost = body_scale * body_cost + common_opcode_overhead
```

Run `25db29d91bd3311441fe8317` showed that the former linear 4-KiB page-count term fit its training
rows (production MAPE 0.259%, maximum APE 1.103%) and passed the `0x0800` and `0x0fe0` holdouts, but
overpredicted `0x2000` by about 42% for all three memory opcodes. Replacing the count with the binary
`memory_4k_boundary_event` lowered those viewed-row errors to 5.72%, 5.79%, and 5.96%. Because that
row informed the revised hypothesis, `0x2000` is now fit/diagnostic evidence and cannot remain a
holdout.

The then-current 66-scenario matrix had aggregate exact rank and parameter count 13; its shared-memory
fit has exact rank six. For each memory opcode, the reused warmed rows at offsets `0x0100` and
`0x1000` remain zero-growth holdouts, `0x0800` and `0x0fe0` are retained after passing the prior
model, and `0x4000` is the fresh untouched expansion holdout. Production-space fit MAPE must be at
most 5%, and
fit/holdout maximum APE must be at most 10%. A quality failure is preserved as `not_supported`; it
does not abort the schema-3 diagnostic artifact, modify a coefficient after seeing results, or enter
candidate construction. The revised manifest requires another fresh calibration identity; the run
above motivated the change but cannot validate it.

Fresh run `8e2abe743f28f69ec2f89c8b` validated the revised shared-memory hypothesis. The shared fit had
exact rank six, 1.926% production MAPE, 7.416% fit maximum APE, and 8.887% holdout maximum APE. The
previously untouched `0x4000` production-space holdouts were 7.223% for MLOAD, 7.288% for MSTORE,
and 7.410% for MSTORE8, all below the frozen 10% gate. The aggregate artifact is nevertheless
`not_supported` and `candidate_eligible = false` because the independent KECCAK256 model failed its
fit gates (10.318% MAPE and 29.384% maximum APE). EXP, MCOPY, and the shared-memory family are
supported; the aggregate status must not be interpreted as a shared-memory failure. That run used
the superseded `input_words` KECCAK256 model, so its KECCAK256 failure is motivation rather than
validation evidence for the permutation model.

KECCAK256 now has 22 fit rows, including its canonical row and all previously viewed evidence, plus
seven frozen untouched holdouts at 95, 333, 543, 544 (with 17 warmed memory words), 545, 1500, and
4096 bytes. The fit rows exercise both sides of the 136- and 272-byte padding boundaries; the new
holdouts add the 544-byte padding boundary and distant extrapolation. A fresh manifest identity and
calibration run must validate this matrix; the shared-memory evidence from run
`8e2abe743f28f69ec2f89c8b` remains valid prior evidence because that family and its rows are
unchanged.

Run `77cff7accc62325aad8c047b` then accepted all 162 formal relations and confirmed the nonzero
permutation model on all seven then-frozen holdouts (1.841% maximum production APE), but the two-parameter
fit failed because the zero-length fit row had 232.621% APE. REVM returns `KECCAK_EMPTY` for that
row without entering the nonempty hash/syscall setup path, so one shared constant was the wrong
semantic model. A read-only diagnostic with the explicit zero-length event reduced fit MAPE to
0.603%, fit maximum APE to 1.388%, and holdout maximum APE to 0.801%. Those viewed results motivate
the three-parameter model but do not validate it, and those seven viewed holdouts are now fit
evidence. The implementation revision and replacement holdouts require a fresh calibration identity
and run. The zero-length coefficient has no distinct length-domain holdout; support for that branch
is limited to the REVM fast-path semantics plus its three repeated fit measurements, while the fresh
holdouts validate only the nonzero permutation relation.

Fresh run `e8663a09b3f35da37196cbdf`, frozen at implementation revision `3b9ba3b3`,
accepted all 169 formal relations and produced a full-rank 14-parameter structured model with
`status = supported`. KECCAK256 used 22 fit rows and seven untouched holdouts: production-space fit
MAPE was 0.535%, fit maximum APE was 1.378%, and holdout maximum APE was 0.675%. The holdout APEs
for 95, 333, 543, warmed 544, 545, 1500, and 4096 bytes were respectively 0.488%, 0.029%, 0.675%,
0.308%, 0.291%, 0.052%, and 0.007%. Its fitted production-space term was approximately
`678.14 - 500.51 * zero_length_event + 2147.09 * keccak_permutations + f_mem`. EXP, MCOPY, and the
shared-memory family also remained supported; the shared-memory holdout maximum APE remained 8.887%.
The diagnostic is deliberately `candidate_eligible = false`. The old scalar block-candidate path
still fails closed on a nonpositive reconstructed lab-body scalar for static `opcode:0x19`, before
reaching dynamic-scalar validation. This is a separate static affine/scalar issue; it neither
validates nor invalidates the structured dynamic result.

A replay audit of this run found that direct library calls to `fit_formal_relation_round` inherited
the caller's Decimal precision, while the CLI already ran at the required 80-digit precision. The
round fitter now isolates that context directly, and default-precision replay reproduces the sealed
169-relation ledger byte-for-byte. This fix does not change the sampled rows or fitted artifact, so
the run remains evidence for revision `3b9ba3b3` and does not require resampling.

`fit-block-calibration` deliberately retains the scalar lab-body dynamic holdout gate and therefore
remains fail-closed until a separately reviewed promotion defines the production representation.
A lab-body scalar pass would not by itself prove that one production per-raw-gas scalar is valid:
the nonzero common per-operation overhead must also be represented explicitly when raw gas varies.

`initial_memory_words` is executable fixture state, not descriptive metadata. Dynamic memory
relations allocate exactly that many words with the fixed warmup before the measured opcode. This
lets equal-size operations vary memory growth independently; changing only the manifest field would
produce a falsely full-rank design matrix over identical executions.

`run-relations` owns the frozen per-relation adaptive bounds `8, 32, 128, 512, 2048`. The initial
bound-8 batch must contain the complete manifest relation set and pass the full frozen dynamic
preflight. Later rounds regenerate and execute only relations whose prior failure was limited to the
frozen fit, signal, R2, residual, or checkpoint gates. Trace, repeat, provenance, raw-gas, schema,
fixture, or identity failures abort instead of expanding. Each round is sealed in
`formal-relation-decisions.json`; resume replays every recorded raw/result hash and skips relations
already accepted. The canonical `raw/formal-relations.jsonl` is created atomically only after every
relation has accepted exactly one generator bound.

The sealed decisions ledger, not the final JSONL by itself, is the downstream source of truth.
Relation fitting, block-calibration execution/fitting, candidate construction, and candidate/bridge
replay verify `formal-relation-decisions.sha256`, replay every decision, require a complete accepted
terminal state, reconstruct the canonical accepted rows, and then require
`raw/formal-relations.jsonl` to match those canonical JSON lines byte-for-byte and in order. The
candidate provenance binds `formal_relation_decisions_sha256` in addition to the relation artifact
digest.

Relation `count` is the number of active target-microprogram repetitions in the fixed footprint. It
is not transaction count, block count, gas limit, or a value recovered from runtime gas. Non-self
OLS uses only positive fit counts (`count >= 1`); count zero never enters slope, R2, standard error,
residual, signal, or checkpoint baselining. The artifact reports
`positive_fit_intercept_p`, `zero_delta_p`, and signed
`activation_gap_p = zero_delta_p - positive_fit_intercept_p`. A positive gap means count zero lies
above the positive-count line's extrapolated intercept. `activation_gap_ratio` is
`abs(activation_gap_p) / signal_p` when signal is positive. These fields and the tail evidence are
part of formal relation artifact schema version 3; version-1 and version-2 relation artifacts cannot
enter the new candidate path. Version 3 additionally binds each dynamic row's `model_split`.

Every selected relation round pre-generates and executes an `active_tail` count-one pair. Its target
microprogram executes in the final slot; ordinary count one executes in the first slot. Their
raw-gas multisets and footprint match, while structured placement/sample identity keeps grouping,
raw order, three-repeat completeness, resume replay, and hashes distinct. If activation-gap ratio
is strictly greater than `0.02`, the tail becomes an acceptance gate: its observed marginal is
`delta_tail - delta_zero`, its prediction is `slope_p`, signs must match, and APE must be at most
`0.10`. Both zero passes exactly; one zero or opposite signs fails. With zero positive signal, a
zero gap reports ratio zero and does not trigger, while a nonzero gap reports an explicit
`zero_signal_nonzero_gap` status and must trigger. Untriggered tail results remain diagnostics; JSON
never uses Infinity or NaN.

Count and placement remain host-only sampling metadata. The serialized guest-visible `case` is
stable per lane (`<case>__relation_target` or `<case>__relation_control`) across every count and
placement; consequently all control inputs at one generator bound are byte-identical, while the
prefix-one and tail-one target inputs differ only in bytecode slot order. Replay reconstructs the
entire canonical case and guest declaration from the manifest relation and generation formulas—not
from persisted scenario, relation scenario, gas limit, opcode/count, length, maps, or fixture
metadata. It then recomputes pair, workload, and execution-row identities and joins them to the
controlled trace. Identical canonical inputs may intentionally reuse workload/execution identities;
arbitrary relabeling, declaration changes, or reuse fails exact recomputation.
Every opcode fixture's `evm_opcode_counts` field records the static EVM bytecode multiset. The raw
report field `opcode_counts` remains the SP1 RISC-V execution profile; both are retained under those
distinct names, and any opcode fixture/report key collision is rejected before the raw row is built.

An exactly constant non-self target-minus-control response across positive fit points and checkpoint
is a valid zero-slope relation; count zero then remains activation evidence. Self-controls retain
all-count exact-flat semantics, including the tail placement. A non-self exact-flat row still binds
its signed raw-gas coefficient map and all ordinary trace/provenance evidence; non-flat small signals
keep the ordinary signal gate.

The sealed failed run `runs/1bee0a5941fddc9984b009b8` remains read-only design evidence. Its JUMPI
row at bound 2048 had count-zero gas `85829242`, first-slot count-one gas `85786709`, last-slot
count-one gas `85830087`, positive-count slope about `817.4076`, positive-fit residual/signal about
`0.000486`, and signed activation gap about `+43254.95`. It demonstrates activation/slot-order
contamination but is not a candidate source.

The relation fit reconstructs pure-opcode costs from the frozen rank-98 system and four internal
algebraic basis coordinates; their opcode labels do not give them special physical meaning. `A_i`
and `x_j` use actual per-key raw-gas totals, not execution counts. Calibration is deliberately
staged: within-family count slopes first solve only the two anchor-transfer parameters, then the
reconstructed opcode table is frozen and the four fixed/base costs are fitted from residual block
cost. This prevents the roughly 159M startup term from compensating for the much smaller opcode
signal in one joint least-squares solve. Dynamic raw-gas scenarios are holdouts and never refit the
model.

The commands below are retained as exploratory predecessor workflows only. They are not the current
candidate path, and the old early-STOP opcode fit is explicitly non-candidate.

Generate sealed controlled case metadata and lab inputs:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate \
  --manifest experiments/opcode-gas/manifests/<controlled>.toml \
  --calibration-run experiments/opcode-gas/runs/<calibration-id> \
  --out experiments/opcode-gas/runs/<calibration-id>/fixtures
```

Run existing `guest-input.json` cases with a prebuilt launcher:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run \
  --fixtures experiments/opcode-gas/runs/<calibration-id>/fixtures \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_opcode_lab.elf \
  --precompile-elf crates/guests/elf/sp1_precompile_lab.elf \
  --calibration-run experiments/opcode-gas/runs/<calibration-id> \
  --controlled-manifest experiments/opcode-gas/manifests/<controlled>.toml \
  --out experiments/opcode-gas/runs/<calibration-id>/raw-runs.jsonl
```

Run the same opcode fixtures through the revm-backed SP1 guest:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run \
  --fixtures experiments/opcode-gas/runs/<calibration-id>/fixtures \
  --guest-launcher target/release/guest-launcher \
  --opcode-stage revm-opcode-lab \
  --calibration-run experiments/opcode-gas/runs/<calibration-id> \
  --controlled-manifest experiments/opcode-gas/manifests/<controlled>.toml \
  --out experiments/opcode-gas/runs/<calibration-id>/revm-raw-runs.jsonl
```

Use this `revm-opcode-lab` path as the primary opcode-tuning path once the smoke suite is stable. It
executes the generated bytecode through revm's transaction execution/interpreter stack, rather than
the experiment-only mini interpreter used by `opcode-lab`.

### Fast SP1 Opcode Execution

The experiment runner selects SP1's `gas-estimator` execution engine for both `opcode-lab` and
`revm-opcode-lab`. This executes the real SP1 program with `GasEstimatingVM` and returns normalized
`ExecutionReport::gas()`; it is not a fitted formula or a replacement metric. The runner freezes the
canonical gas-estimation cadence (`gas_trace_chunk_threshold = 134217728`,
`gas_trace_chunk_slots = 2`) in calibration provenance and rejects rows produced by another engine
or cadence.

The fast engine is restricted to local SP1 execute-mode opcode labs. Proposal, precompile, overhead,
aggregation, and proof-generation paths continue to use their normal engines. Use direct
`guest-launcher --sp1-execution-engine standard` runs only for focused parity checks; do not mix
standard and gas-estimator rows in one fit. The estimator is an offline calibration and validation
oracle, not a production online quote or admission path. Production online behavior continues to
use the host-native operation ledger and sealed coefficient table.

### Matched-Control Opcode Diagnostics

Matched-control diagnostics keep the bytecode footprint, transaction gas limit, operand setup, and
final stack height equal between target and control lanes. They are useful for finding fixture and
operand-path problems before running the formal controlled candidate workflow:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-calibration \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --guest-launcher target/release/guest-launcher \
  --out experiments/opcode-gas

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate-matched-control \
  --manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --calibration-run experiments/opcode-gas/runs/<calibration-id> \
  --out experiments/opcode-gas/runs/<calibration-id>/fixtures/matched-small-nonzero \
  --operand-profile small_nonzero \
  --generator-max-count 32 \
  --case add --case div --case eq --case exp

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-matched-control \
  --fixtures experiments/opcode-gas/runs/<calibration-id>/fixtures/matched-small-nonzero \
  --guest-launcher target/release/guest-launcher \
  --calibration-run experiments/opcode-gas/runs/<calibration-id> \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out experiments/opcode-gas/runs/<calibration-id>/matched-small-nonzero.jsonl
```

The `zero` operand profile is the compatibility/default profile: binary operations use `[0, 0]`,
EXP uses `[2, 2]`, and unary operations use `[1]`. The `small_nonzero` profile uses `[7, 3]`,
`[3, 5]`, and `[7]`, respectively. The profile, resolved operands, target/control bytecode, fixture
hashes, and execution identity are all bound into each pair. Validation reconstructs the exact
GuestInput bytecode rather than trusting metadata labels.

These reports contain contextual relative signals. Binary and EXP cases currently report
`OP - POP`; unary cases report `OP - NOT`, so NOT's self-control delta is intentionally zero. The
`zero` profile additionally supports `MLOAD - NOT`, `KECCAK32 - POP`, `DUPn - DUP1`,
`SWAPn - SWAP1`, and `PC/MSIZE/GAS - PUSH0`; PUSH0 is its own self-control. DUP1, SWAP1, and PUSH0
self-control pairs also intentionally have identical target/control bytecode and zero delta. These
additional families reject `small_nonzero` because they do not define a second operand profile. The
`zero` profile also supports `ADDMOD/MULMOD - (POP+ADD)`, `POP - (NOT+POP)`,
`MSTORE/MSTORE8 - (ADD+POP)`, `MCOPY - (ADD+MUL+POP)`, `PUSHn - PUSH0`, `JUMP - POP`,
`JUMPI - (ADD+POP)`, and `JUMPDEST - (PUSH0+POP)`. Compound controls bind the exact executed target
and reference programs, padding, common suffix, opcode counts, and raw-gas total into the pair.
Coefficients from an early-STOP control are explicitly rejected: equal serialized size alone does
not establish an equal executed workload. These numbers remain contextual relative signals, not
absolute opcode costs, and cannot be copied into a production multiplier table.
Different operand profiles may exercise materially different guest paths: in the initial comparison,
EQ, signed comparisons, division/modulo, EXP, and LT/GT changed between zero and small-nonzero
profiles. A measurement key that needs multiple scenarios must satisfy the formal required-case
rules; a successful diagnostic scenario cannot be generalized to rejected or unmeasured siblings.

Never continue an existing calibration directory after implementation, manifest, dependency, or
guest-artifact changes. Preserve the old run and use `prepare-calibration` at the new clean revision
to create a new content-addressed identity. See the
[SP1 opcode proverGas calibration pitfalls][opcode-calibration-pitfalls] for failure modes and
recovery rules.

[opcode-calibration-pitfalls]: ../../docs/solutions/experiment-correctness/sp1-opcode-prover-gas-calibration.md

### Exploratory Predecessor: Matched Controls And Sequential Overheads

The following legacy matched-control and sequential-overhead commands are exploratory evidence only;
they must not create, repair, or seal the relative-relation/block-fit candidate. Each round
regenerates and reruns one complete frozen footprint; resume verifies the sealed round decisions and
never mixes rows from different generator maxima:

If a required primary overhead has not passed, every dependent overhead is recorded as rejected with
`unmeasured_overhead_dependency` and the missing dependency IDs. The round expands to the next frozen
footprint instead of attempting residualization or aborting; secondary dependency status remains an
independent `unresidualized_dependencies` diagnostic.

Operation workloads retain the frozen generator rounds through 2048. Production-proposal overhead
workloads stop at `overhead_generator_max_count = 128` (at most 129 synthetic blocks). The later
operation rounds reuse the sealed 128 overhead raw file and refit those rows against their current
operation coefficients; they do not rerun or relabel overhead raw data. Decisions and fit artifacts
persist both generator bounds, and resume validation rejects a changed bound or a later operation
record that does not reference the exact sealed 128 raw artifact.

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-controlled \
  --fixtures experiments/opcode-gas/runs/<calibration-id>/fixtures \
  --guest-launcher target/release/guest-launcher \
  --calibration-run experiments/opcode-gas/runs/<calibration-id> \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out experiments/opcode-gas/runs/<calibration-id>/controlled-runs.jsonl

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-controlled-costs \
  --runs experiments/opcode-gas/runs/<calibration-id>/controlled-runs.jsonl \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out experiments/opcode-gas/runs/<calibration-id>/controlled-fit.json

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py build-candidate \
  --run experiments/opcode-gas/runs/<calibration-id> \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --relations experiments/opcode-gas/runs/<calibration-id>/opcode-relations.json \
  --anchor-probe experiments/opcode-gas/runs/<calibration-id>/anchor-probe-fit.json \
  --block-calibration experiments/opcode-gas/runs/<calibration-id>/block-calibration.json \
  --controlled-fit experiments/opcode-gas/runs/<calibration-id>/controlled-fit.json \
  --provenance experiments/opcode-gas/runs/<calibration-id>/provenance.json

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py build-sp1-bridge \
  --run experiments/opcode-gas/runs/<calibration-id> \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --samples experiments/opcode-gas/runs/<calibration-id>/samples/controlled-cycle-cost-samples.json
```

`run-controlled` executes every overhead fixture through the production `sp1-shasta-proposal`
guest, records three repeats, and emits raw primary and instruction-count observations. Candidate
and bridge construction consume marginal same-unit pairs only: `c_p/c_s` or `f_p/f_s` for operations
and `o_p/o_s` for overheads. Missing or unresidualized secondary dependencies remain explicit
`unavailable` samples and produce a sealable `insufficient_data` bridge; they never fabricate an
instruction-count coefficient or change the primary candidate.

`build-candidate` accepts only the canonical `controlled-fit.json`, `controlled-overheads.json`, and
`provenance.json` inside the calibration run. It revalidates the contiguous persisted decisions,
requires the terminal decision to be `complete`, and binds their exact hashes plus the frozen
experiment identity into the candidate. `build-sp1-bridge` likewise recomputes the expected marginal
sample artifact from that terminal round and binds its exact hash and candidate digest into the
bridge root; copied or edited accepted-looking JSON is rejected.

Run one real proposal GuestInput through RISC0 execute mode and write a normalized raw JSONL row:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-proposal \
  --guest-launcher target/release/guest-launcher \
  --guest-input /tmp/proposal-170-guest-input.json \
  --proof-type risc0 \
  --case proposal-170 \
  --target-raw-gas 30000000 \
  --out /tmp/raiko2-opcode-gas/risc0-proposal-170.jsonl
```

The RISC0 proposal path uses `guest-launcher --stage proposal --proof-type risc0 --mode execute`
and records `risc0_padded_cycles` as the primary workload metric. It also records
`risc0_user_cycles`, segment count, and segment `po2` histogram. It does not produce a proof and
does not call Boundless or any other network prover.

SP1 `prover_gas` and RISC0 cycle counts are backend-native metrics, not the same unit. The SP1
working heuristic that a 30M Ethereum-gas block maps to roughly 10B `proverGas` must not be applied
to RISC0. For RISC0, use a RISC0-native cycle budget, such as a working 30M Ethereum-gas to 10B-cycle
anchor, and report it separately until a cost or time calibration layer exists. Cross-backend
envelopes should compare normalized workload units, not `max(sp1_prover_gas, risc0_padded_cycles)`.

Fit reports from raw runs. SP1 lab runs default to `prover_gas`; RISC0 raw runs can select a cycle
metric explicitly:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit \
  --runs /tmp/raiko2-opcode-gas/raw-runs.jsonl \
  --out /tmp/raiko2-opcode-gas/report

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit \
  --runs /tmp/raiko2-opcode-gas/risc0-proposal-runs.jsonl \
  --metric risc0_padded_cycles \
  --out /tmp/raiko2-opcode-gas/risc0-report
```

Compute an eth-limit damage frontier and current-Unzen containment report from fit results:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py damage \
  --fit /tmp/raiko2-opcode-gas/report/fit.json \
  --manifest experiments/opcode-gas/manifests/sp1-smoke.toml \
  --eth-gas-limit 30000000 \
  --zk-gas-limit 100000000 \
  --out /tmp/raiko2-opcode-gas/damage
```

Write the Unzen opcode/precompile coverage inventory:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py inventory \
  --manifest experiments/opcode-gas/manifests/sp1-smoke.toml \
  --out /tmp/raiko2-opcode-gas/inventory
```

The `run` command invokes `target/release/guest-launcher` directly with `--stage opcode-lab`,
`--stage revm-opcode-lab`, or `--stage precompile-lab`, always using `--proof-type sp1 --mode
execute --sp1-prover local`. It batches generated inputs into one launcher process per lab stage
through `--input-list` and `--jsonl-out`, so the expensive SP1 executor startup cost is paid once
per stage instead of once per variant. It does not use `cargo run`, does not submit network proofs,
and does not access live L1/L2 RPC.

The `run-proposal` command invokes `guest-launcher` directly for one real Shasta proposal
`GuestInput`. Use this path for block/proposal-level calibration and cross-zkVM observation. It is
not an opcode coefficient generator by itself because one proposal is one workload sample, not a
controlled target-count sweep. Its `--target-raw-gas` value is report metadata for comparing runs;
it does not mean the proposal itself is exactly one 30M-gas block.

The `damage` command is measurement-only. It answers how much measured workload fits under an Ethereum gas
limit, then shows how much of that surface remains reachable under the current-Unzen multipliers
and a chosen block zk gas limit. Realistic block and app impact is a later input to the same report,
not inferred from the smoke lab.

The `inventory` command is coverage-only. It lists every opcode and precompile entry from the current
Unzen table and marks whether the smoke manifest measures it or which future measurement path is
needed.

For a quick smoke after changing launcher code, `target/debug/guest-launcher` built with
`cargo build -p guest-launcher --features sp1-sdk/profiling` also works. Use the release binary for
longer research loops once it has been rebuilt.

## Current Limitation

The default opcode-lab guest is a small experiment-only bytecode interpreter, not revm and not a
complete EVM. It uses narrow synthetic stack and memory semantics to isolate guest-side opcode
workload. Use it as a fast smoke signal and regression anchor, not as evidence that the measured
slope is the exact cost of real EVM execution.

The `revm-opcode-lab` guest runs the same `OpcodeLabInput.bytecode` through revm with a fixed
Prague/mainnet benchmark transaction and empty benchmark database. This adds real revm transaction
execution, interpreter dispatch, stack, memory, and gas semantics while still removing
block/proposal noise. It is the current primary candidate for opcode coefficient tuning. Do not
replace it with a lower-level direct interpreter call unless there is a specific measurement bug:
that would drop useful revm execution context. The remaining gap is not interpreter reuse, but
realistic context.

The smoke manifest now expands to every active Unzen opcode that can be isolated without state, environment,
or CALL/CREATE wrapper semantics, including arithmetic, comparison, bitwise, stack, fixed
control-flow, and memory-copy templates.

The precompile-lab guest supports direct body measurements for the active Unzen precompiles with
existing fixed deterministic input templates. CLZ (`0x1e`) and p256 (`0x100`) are listed by
`inventory` as `unsupported_by_experiment`; this change does not add guest implementations or
fixtures for them. These are direct body calls, not `STATICCALL` dispatch measurements.
`STATICCALL` wrapper cost, warm/cold account access, precompile argument sweeps, stateful opcodes,
and full block execution are TODOs for later suites.

Stack, control, memory, and precompile slopes include the setup needed by the lab templates, so use
them as smoke damage signals and regression anchors, not as final consensus coefficients.

The real proposal `GuestInput` path is the calibration layer for full block behavior.

For tiny stack and memory templates, always inspect `fit.json` before using `damage.md`. Low R2 means
the current variant counts are too small or the template has setup/cleanup noise. In that case, rerun
with larger variants or a cleaner template before treating the slope as a candidate coefficient.

## Follow-Up TODO

## Frozen SP1 Calibration Inputs

The controlled SP1 candidate is review-only. Relation-matrix construction, rank validation,
anchor reduction, `mu_zero`, and `B` use exact `Fraction` algebra. Observations, fitting,
prediction, error computation, and canonical numeric serialization use 80-digit `Decimal` values;
binary `float` is never a canonical intermediate. The candidate never writes a protocol multiplier
table, Alethia source, generated guest artifact, or production prover config.
Candidate sealing requires the accepted rank-98 relation system, positive reconstructed ADD,
dynamic raw-gas holdouts, the staged rank-two transfer and rank-four fixed/base fits and holdouts,
and canonical component hashes. The
instruction-count sample, controlled SP1 bridge, and diagnostic overheads have
independent digests and cannot change the primary candidate.

The component ledger keeps unlike units separate:

- measured non-Anchor transaction-phase ordinary opcodes (including EVM `SHA3`) use interpreter raw
  gas, the current schedule multiplier, marginal SP1 `proverGas / raw gas`, and
  ADD-normalized cost index;
- direct precompiles use native gas and the same raw-gas/index convention;
- confirmed CALL/CREATE dispatches use a fixed `proverGas / event` value and
  are never normalized by ADD or forwarded gas;
- startup, block, started-non-Anchor-transaction, and native-transfer costs use their
  declared fixed/base unit, with `N/A` when the current zkGas schedule has no
  independent charge;
- V1 `block_base` owns the complete system/Anchor work and its fixed per-block
  MPT, trie, Merkle, and host-hash baseline. `BLOCKHASH` is not a measured V1
  operation coefficient. Witness/input/blob/KZG counts, unique accesses, and
  dirty-state entries remain diagnostic/unmeasured and never enter `Q_formula`,
  the candidate, or the bridge.

Host/guest internal actions are not silently folded into an opcode raw-gas
index. The V1 block residual deliberately includes system/Anchor and fixed
trie/hash work as one block-level unit, while the execution sum includes only
non-Anchor transaction operations. The same ownership guard is applied to
controlled and proposal rows, so the two terms cannot double count. A future V2
may split these actions after defining independently controlled units; proposal
rows cannot fit or repair them.

Every variable-count case runs three identical local SP1 executions. A
grow-with-count bytecode template is `confounded_template`: bytecode/input
length and all non-target executed counts/raw gas must stay fixed while only
the selected target count changes. Direct precompile measurements additionally
require target/control lanes with identical loop, input, output-length, and
folding shapes. The first passing cumulative sweep is frozen before its mapped
out-of-fit checkpoint; checkpoint data never enters OLS.

`prepare-calibration` freezes a clean implementation revision and the controlled
manifest before measurement. It creates an immutable bridge manifest alongside
the calibration directory; the bridge may later be sealed as `insufficient_data`
without changing the primary `proverGas` candidate.

`prepare-corpus` selects the fixed 60-row V1 validation corpus (40 Hoodi, 20
Mainnet) from the two checked-in 2026-09-02 fixture files before it contacts an
RPC endpoint. It accepts only network-qualified RPC, chain-spec hash, and
chain-spec file inputs. Each chain-spec file is the JSON list format consumed by
preflight, must contain exactly one selected network entry with an explicit
`hard_forks.UNZEN` activation, and is passed unchanged to preflight. The command
uses the fixed proposal IDs with discovery-only mode and invokes preflight with
the valueless `--validate` flag. It never substitutes a failed proposal. The
saved GuestInputs are written below the ignored repository-relative corpus path
and packed with deterministic tar metadata.

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-corpus \
  --corpus-root experiments/opcode-gas/corpora/sp1-mainnet-hoodi-v1 \
  --manifest experiments/opcode-gas/manifests/proposals/sp1-mainnet-hoodi-v1.json \
  --l1-rpc taiko_hoodi=https://l1.example.invalid \
  --l1-rpc taiko_mainnet=https://l1.example.invalid \
  --l2-rpc taiko_hoodi=https://l2.example.invalid \
  --l2-rpc taiko_mainnet=https://l2.example.invalid \
  --chain-spec-hash taiko_hoodi=<sha256> \
  --chain-spec-hash taiko_mainnet=<sha256> \
  --chain-spec-file taiko_hoodi=config/frozen-taiko-chain-specs.json \
  --chain-spec-file taiko_mainnet=config/frozen-taiko-chain-specs.json
```

The actual final acquisition belongs only to the later measurement step. To
freeze an already-created archive, `publish-corpus` requires a single
content-addressed `gs://.../<archive_sha256>.tar` destination. It uses GCS
create-only upload, reads back the exact object generation, downloads that
generation, and checks the archive hash before recording the URI, generation,
and size in the manifest. `local_unpublished` corpora cannot enter validation.

`prepare-validation` requires the independently sealed candidate and bridge
digests, current `HEAD` equal to their shared `implementation_revision`, and a
fully published 60-row corpus. It writes only below its new validation directory.
Generated corpus, run, manifest, and validation output paths may remain dirty;
any implementation, dependency, controlled-manifest, or guest-artifact change
rejects the operation.

`prepare-calibration` copies and seals the exact controlled TOML under its new
calibration run. Controlled `generate` and `run` commands require both
`--calibration-run` and `--controlled-manifest`; they reject a swapped manifest
or generated fixture whose calibration/controlled-manifest provenance does not
match that run before starting guest execution. Repository-relative paths in
these commands are resolved from the repository root and cannot escape it.

For an `integration_smoke` proposal execution, first write the smoke record with
`prepare-integration-smoke --guest-input <GuestInput.json> --out <record.json>`,
then supply the same `--network`, `--proposal-id`, and
`--smoke-record <record.json>` to
`run-proposal --purpose integration_smoke`. The command validates that the
record is still purpose-labelled `integration_smoke`, is disjoint from the
frozen final 60 rows, and binds the exact GuestInput bytes and embedded
network/proposal identity before invoking guest-launcher.

- Add a Taiko/reth-context revm lab that keeps the `revm-opcode-lab` execution path but uses Taiko
  fork config, block env, and realistic tx env instead of fixed Prague/mainnet benchmark defaults.
- Add stateful benchmark databases for account/storage opcodes, including warm/cold dimensions and
  representative account/storage distributions.
- Add CALL/CREATE wrapper scenarios that separate spawn/no-spawn estimates, call depth, value
  transfer, return-data copying, and precompile dispatch.
- Add `STATICCALL` precompile wrapper measurements on top of the direct precompile body lab.
- Add real proposal/app attribution so damage reports can show normal-workload `zk_util` percentiles
  and top opcode/precompile contributors, not only synthetic attack surfaces.
- Keep SP1 `prover_gas` and RISC0 cycle budgets separate until a calibrated common cost unit exists.
