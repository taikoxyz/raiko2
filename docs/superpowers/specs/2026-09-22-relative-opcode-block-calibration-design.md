# Relative Opcode And Controlled Block Calibration Design

## Status

Approved in principle on 2026-09-22. This document freezes the design delta before implementation.
It does not authorize a production zkGas schedule change, block-limit change, or proposal-derived
fit.

This design supersedes the parts of the existing recalibration documents that require every opcode
to have an independently measured absolute `proverGas / operation` slope before block overhead can
be calibrated. All provenance, post-Unzen, candidate-sealing, bridge-isolation, and proposal-only
validation rules remain unchanged.

## Problem

The fixed-footprint opcode lab now covers all 102 pure opcode keys with matched target/control
programs. Those programs preserve the transaction envelope, gas limit, bytecode footprint, operand
setup, and intended final stack shape. They measure reliable contextual differences, such as
`ADD - POP`, `MLOAD - NOT`, or `PUSHn - PUSH0`.

They do not provide 102 independent absolute costs. A valid EVM execution cannot run stack-consuming
operations such as `POP` without first producing stack values, and there is no zero-cost EVM opcode
that can replace an executed operation while preserving the complete path. Treating an early
`STOP` as that zero-cost control was empirically confounded and is forbidden.

For the frozen 102-key manifest, the raw-gas-weighted matched-control equation matrix has 98
independent rows and rank 98. Fixing the natural anchors `POP`, `PUSH0`, `DUP1`, and `SWAP1` leaves
a square full-rank 98-column system. Therefore the opcode lab has completed the relative
measurement problem; four anchor multipliers remain unidentified.

The experiment does not need to invent standalone absolute opcode costs. The dedicated opcode-lab
guest supplies the four anchor body-cost slopes after matched target/control subtraction. Controlled
blocks through the production SP1 proposal guest then fit a shared body scale, a shared production
interpreter cost per executed opcode, and the four already-declared fixed/base costs.

## Goals

1. Promote accepted matched-control slopes from diagnostics into the formal opcode relation model.
2. Preserve the four unidentified opcode anchor multipliers as an exact affine nullspace, then
   constrain them from four frozen synthetic body-cost slopes with two production transfer
   parameters.
3. Fit those two transfer parameters and the four required fixed/base costs using only frozen
   controlled block fixtures executed through `sp1-shasta-proposal`.
4. Reconstruct positive SP1-native raw-gas multipliers for the static-cost opcodes and derive the
   existing ADD-normalized view.
5. Keep the failed scalar lab-body dynamic-opcode hypothesis as a candidate gate, while separately
   fitting frozen semantic diagnostics to learn which dynamic terms are identifiable or negligible.
6. Seal the complete controlled candidate only after a later reviewed promotion chooses the dynamic
   production representation, and before opening final Mainnet or Hoodi proposal results.

## Non-Goals

- Do not measure a synthetic `POP-only` transaction or pre-seed an impossible production EVM stack.
- Do not use the current Unzen multiplier table as a prior, constraint, or hidden anchor.
- Do not fit any opcode anchor, fixed/base coefficient, threshold, or feature on proposal rows.
- Do not split trie, Merkle, hashing, system, or Anchor work below the existing `block_base` owner.
- Do not change the precompile, spawned-wrapper, bridge, RISC0, Boundless, or production-promotion
  scope except where their existing dependency on accepted opcode costs must consume the newly
  reconstructed table.

## Formal Model

### Relative Opcode Equations

Let `K` be the ordered 102-key pure-opcode set. Let `mu` be the column vector of SP1
`proverGas / raw EVM gas` multipliers in that order.

For each accepted matched-control relation `i`, derive the integer row `A_i` from the exact executed
target raw-gas totals minus the exact executed control raw-gas totals, grouped by opcode key. Fit
the signed marginal `proverGas` difference `d_i` from the frozen count sweep:

```text
A_i * mu = d_i
```

The relation orientation is part of the manifest and may produce a negative `d_i`. Positivity
applies to reconstructed opcode multipliers, not to signed relation deltas. Self-controls such as
`PUSH0 - PUSH0` validate zero-delta fixture behavior but add no equation. Dynamic-gas operations
use the fixture trace's actual raw gas; an execution count is never substituted for raw gas.

The complete system is:

```text
A * mu = d
rank(A) = 98
nullity(A) = 4
```

Choose the following parameterization, which has been checked to leave the remaining 98 columns
full rank:

```text
theta = [mu(POP), mu(PUSH0), mu(DUP1), mu(SWAP1)]
mu = mu_zero + B * theta
```

The integer/rational matrix `B` is derived from `A`. `mu_zero` is the unique solution of the
accepted `A * mu = d` system when the four natural anchors are set to zero. Neither is fitted from
block data. The solver must verify the rank and the natural-anchor property from the frozen
manifest rather than trusting constants in code.

### Synthetic Anchor-Ratio Prior

The experiment-only `sp1-opcode-lab` guest may measure a diagnostic positive ratio among the four
free anchors before production-block fitting. This is a prior on relative shape only: it is not a
production opcode cost, cannot enter the rank-98 relation equations as an accepted row, cannot set
the final anchor scale, and cannot satisfy or bypass production controlled-block validation.

The probe reuses the historical `OpcodeLabInput` wire schema. Inside the dedicated
`sp1-opcode-lab` binary, an exact case/opcode/envelope declaration plus one of the equal-length
scenarios `anchor_target_` or `anchor_control` decodes to a binary-private typed lane. No probe type
or dispatch code is added to shared primitives or the shared SP1 guest library, so proposal,
aggregation, precompile, and REVM guest artifacts remain byte-identical. The ordinary
bytecode-interpreter path remains unchanged for every other declaration. Probe mode supports exactly
`POP (0x50)`, `PUSH0 (0x5f)`, `DUP1 (0x80)`, and `SWAP1 (0x90)` and rejects every other opcode or
malformed declaration. For both lanes, the guest executes identical runtime-count loop setup,
bookkeeping, fixed-slot observation, accumulator folding, and fixed-size public-output preparation.
The probe uses REVM's actual `revm::interpreter::Stack` primitive, while remaining outside the full
EVM transaction and dispatch path. Every iteration clears and seeds the same two `U256` words in
both lanes. Lane and opcode selection happens once before the loop, producing an `#[inline(never)]`
function pointer. Both lanes make one function-pointer call per iteration; the control function is
a no-op and each target function calls exactly one of `Stack::pop`, `push(U256::ZERO)`, `dup(1)`, or
`swap(1)`. Both lanes then pass the stack through the same compiler barrier and perform identical
accumulator mixing that does not inspect stack length or contents. This avoids count-scaled opcode
match ordering and observation-branch confounds while keeping the target operation live.

For each anchor and lane, all guest fields and bytecode remain fixed while `target_count` changes.
The bincode representation uses the fixed-width `u64` count, so input length is constant. Target and
control differ only in the equal-length scenario that decodes to the typed lane. Freeze fit counts `[0, 1024, 4096, 16384, 65536]`, checkpoint
`131072`, and three exact repeats. Bind the guest ELF SHA-256, canonical bincode input SHA-256 and
length, fixture hash, gas-estimator engine/cadence, sample/pair identity, and raw ordering.
The calibration identity also binds the exact guest-launcher binary SHA-256; generation, execution,
replay, and candidate consumption reject a different launcher even if all derived rows are
self-consistently rewritten.

Fit `delta_p(n) = P_target(n) - P_control(n)` with a free intercept and Decimal arithmetic. A
nonzero count-zero lane delta is allowed fixed overhead, not per-operation cost. Require each
count-zero lane to be repeat-deterministic and require
`abs(delta_p(0) - fitted_intercept) / total_fit_signal <= 0.02`. Also require positive slope,
`R2 >= 0.99`, relative slope standard error `<= 0.05`, maximum residual/total signal `<= 0.02`,
and marginal checkpoint APE `<= 0.10`, with predicted marginal signal `slope * checkpoint` and
observed marginal signal `delta_p(checkpoint) - fitted_intercept`. Report
`prover_gas_per_operation` and the slope divided by raw gas
`(2, 2, 3, 3)` as `prover_gas_per_raw_gas`.

The sealed diagnostic declares `synthetic_prior_only = true` and `candidate_eligible = false`.
This means it cannot become a candidate table by itself. The production controlled-block fit may
consume its four per-operation slopes only through the transfer model below and must bind the
diagnostic artifact hash. The earlier variable-bytecode sweeps are not reusable: although a free
slope removes fixed guest startup, their changing bytecode/input and helper/deserialization work
remains count-dependent.

The target/control difference removes ELF startup, input decoding, loop bookkeeping, and the
shared indirect call. A free intercept then prevents a fixed target/control lane difference from
becoming per-operation cost. It does not measure the production EVM interpreter's repeated
dispatch/wrapper path, because the synthetic guest invokes `Stack` primitives directly. Therefore
one ratio scale is insufficient. Let `s_i` be the accepted synthetic proverGas-per-operation slope
and `g_i` the corresponding raw EVM gas `(2, 2, 3, 3)`. Define the four production anchors from two
shared transfer parameters:

```text
a = body_scale
h = common_opcode_overhead_per_operation
theta_i(a, h) = (a * s_i + h) / g_i
```

`a` absorbs the shared difference between the synthetic primitive body and its production context.
`h` represents production per-executed-opcode work omitted by direct primitive invocation. `h` is
not guest startup and is not any of the four block/base costs. Require `a > 0` and `h >= 0`; the
controlled production rows, not the synthetic guest, identify both values.

### Controlled Block Model

The required fixed/base vector remains:

```text
beta = [proposal_startup, block_base, tx_base, native_value_transfer]
```

For controlled production-guest row `j`, let:

- `p_j` be normalized SP1 `proverGas`;
- `x_j` be the exact non-Anchor transaction-phase raw-gas vector from the host-native trace, where
  each element is the sum of actual interpreter raw gas for one resolved opcode key;
- `q_j` be the exact four-element fixed/base feature vector using the existing ownership rules.

Let `C` be the exact four-by-two transfer matrix whose row `i` is
`[s_i / g_i, 1 / g_i]`, and let `lambda = [a, h]`. The fitted model is:

```text
theta = C * lambda
mu = mu_zero + B * C * lambda

p_hat_j = x_j * (mu_zero + B * C * lambda) + q_j * beta
```

Only the six parameters `[a, h, beta]` are fitted, but not in one joint regression: `[a, h]` come
from within-family opcode count slopes, then `mu` is frozen before `beta` is fitted from residual
block cost. The synthetic slopes, raw-gas divisors, affine basis, and 98 accepted relation slopes
remain frozen; block rows cannot alter them. Both stages use Decimal arithmetic and only the
predeclared controlled rows.

Precompile costs remain paired-control marginals. Confirmed spawned-wrapper costs remain
fixed-per-event marginals measured after their opcode/precompile dependencies resolve. Neither is an
additional block-anchor parameter.

## Controlled Fixture Contract

### Relation Cohort

The formal campaign reruns the frozen matched-control suite under one fresh content-addressed
calibration identity. Earlier diagnostic runs remain evidence for fixture design but are not
promoted across implementation revisions.

Each nonzero relation must have:

- three deterministic repeats at every selected count;
- identical provenance, engine, cadence, GuestInput identity, public values, and primary
  `proverGas` across repeats;
- a frozen count prefix and a larger, non-fitting extrapolation checkpoint;
- exact target/control per-key executed raw-gas-total validation from the fixture contract;
- `R2 >= 0.995`, relative slope standard error `<= 0.05`, maximum fit residual `<= 0.02` of
  signal, and checkpoint APE `<= 0.10`;
- matching observed and predicted signs at the checkpoint.

Here `count` means the number of active target-microprogram repetitions inside the fixed relation
footprint. It is not a transaction count, block count, gas limit, or runtime gas-derived value.
Every selected relation round also pre-generates and executes one `active_tail` sample: the same
target microprogram executes exactly once in the final footprint slot. Its ordinary count-1 peer
executes once in the first slot. The two samples have the same target/control raw-gas multisets and
fixed footprint and differ only in slot order. `relation_placement` and `relation_sample_id` are
part of pair identity, raw ordering, completeness, resume replay, and sealing.
Sampling metadata does not enter the guest-visible `case`: every lane keeps one stable case string
across counts and placements. Thus fixed-bound control GuestInputs are identical, and prefix-one
versus tail-one target GuestInputs differ only in bytecode slot order. Formal replay reconstructs
the complete canonical case/guest declaration from the manifest relation, generator bound, count,
placement, lane, and generation formulas. Persisted scenario/relation-scenario, gas limit,
opcode/count, fixed length, template, operands, maps, and opcode counts are evidence to compare,
never reconstruction inputs. Replay then recomputes fixture, pair, workload, and execution-row
identities and checks their controlled-trace joins. Repeated identical inputs may have identical
identities; acceptance depends on exact recomputation, not global uniqueness.
Static EVM fixture counts use `evm_opcode_counts`; SP1's `opcode_counts` remains the RISC-V execution
profile in the guest report. Raw-row construction preserves both namespaces and rejects any formal
fixture/report key collision instead of allowing report metadata to replace canonical evidence.
The resulting formal relation artifact uses schema version 3; schema versions 1 and 2 do not carry
the complete activation/tail and dynamic `model_split` evidence and are not accepted by candidate
construction.

`formal-relation-decisions.json` plus its SHA-256 seal is the terminal source of truth for every
downstream relation consumer. The loader replays the complete accepted decision state, derives the
canonical row sequence, and requires the final formal JSONL to equal canonical JSON-line bytes in
that exact order. Block calibration, candidate construction, and candidate/bridge replay use this
loader; candidate provenance also binds `formal_relation_decisions_sha256`.

For the deterministic target/control response `delta_p(x) = p_target(x) - p_control(x)`, freeze the
signed-fit quantities as:

```text
positive fit counts = selected counts x >= 1
delta_p(x) = positive_fit_intercept_p + slope_p * x + residual_p(x)
signal_p = abs(max(delta_p(x >= 1)) - min(delta_p(x >= 1)))
signal_p >= max(1000 proverGas,
                0.01 * abs(positive_fit_intercept_p),
                20 * response_repeat_noise_p)
relative_slope_stderr = se(slope_p) / abs(slope_p) <= 0.05
max(abs(fit_residual_p)) / signal_p <= 0.02

zero_delta_p = delta_p(0)
activation_gap_p = zero_delta_p - positive_fit_intercept_p
activation_gap_ratio = abs(activation_gap_p) / signal_p

observed_delta_check = delta_p(checkpoint) - positive_fit_intercept_p
predicted_delta_check = slope_p * checkpoint
checkpoint_APE = abs(predicted_delta_check - observed_delta_check) /
                 abs(observed_delta_check) <= 0.10

tail_observed_marginal_p = delta_p(active_tail, count=1) - zero_delta_p
tail_predicted_marginal_p = slope_p
```

`response_repeat_noise_p` is the sum of the target and control repeat ranges and is expected to be
zero under the primary determinism rule. Count zero never enters a non-self slope, R2, standard
error, residual, or signal fit; it reports activation only. A positive activation gap means count
zero is above the positive-count line's extrapolated intercept. A non-self exact-flat relation is
flat only when all positive fit counts plus the checkpoint are identical. Self-controls retain the
stronger all-count exact-flat requirement.

The tail holdout becomes an acceptance gate exactly when `activation_gap_ratio > 0.02`. If
`signal_p == 0` and the gap is zero, the ratio is reported as zero and does not trigger. If
`signal_p == 0` and the gap is nonzero, the ratio is reported as unavailable with an explicit
`zero_signal_nonzero_gap` status and the gate must trigger; JSON never contains Infinity or NaN.
For the tail comparison, two zero marginals pass exactly. If only one marginal is zero or their
signs differ, it fails. Otherwise its APE must be at most `0.10`. An untriggered tail result remains
a serialized diagnostic. A negative relation slope is valid. A required non-self relation with
insufficient signal, or one that is missing, confounded, scenario-incomplete, or rejected, prevents
the 102-key candidate from sealing. After filtering, the accepted equation matrix must still have
exact rank 98.

The sealed failed run `experiments/opcode-gas/runs/1bee0a5941fddc9984b009b8` is read-only design
evidence, not a candidate input. At generator bound 2048 its JUMPI row showed count-0 gas
`85829242`, first-slot count-1 gas `85786709`, and last-slot count-1 gas `85830087`; the positive
fit slope was about `817.4076`, positive-fit residual/signal about `0.000486`, and the signed
activation gap about `+43254.95`. This isolates count-0/activation and slot-order effects without
promoting that failed artifact.

### Dynamic Opcode Models

The current V1 pure-opcode set has six keys whose actual interpreter raw gas can change for the
same opcode identity: `EXP`, `KECCAK256`, `MLOAD`, `MSTORE`, `MSTORE8`, and `MCOPY`. The manifest
marks these keys explicitly and rejects an unmarked dynamic-gas template.

The original candidate hypothesis assigned one scalar lab-body multiplier to all raw-gas units of
one key. Re-evaluating the completed controlled run in consistent opcode-lab units falsifies that
hypothesis: anchor body costs are first divided by their raw-gas values, and the canonical and
non-canonical scenarios still cannot satisfy the frozen relation APE and implied-multiplier
consistency gates simultaneously. Keep that lab-body scalar validator in `fit-block-calibration` as
the candidate fail-closed boundary. Do not repair or average the failed scalar after opening the
observations. Passing this body-space check alone would not validate a production scalar because
the nonzero common per-operation overhead also needs an explicit representation when raw gas varies.

The next diagnostic asks the narrower question needed before choosing a production representation:
which semantic components of each operation's SP1 cost are independently identifiable? Freeze 51
scenarios, including reused zero-growth warmed holdouts, retained previously passing expansion
holdouts, viewed `0x2000` fit rows, and one fresh untouched `0x4000` holdout per memory opcode, and
fit the following target-body models:

```text
f_mem = beta_event * memory_growth_event
      + beta_evm * memory_evm_gas_delta
      + beta_boundary * memory_4k_boundary_event

q_EXP       = beta_0 + beta_b * exponent_bytes + beta_b2 * exponent_bytes^2
q_MLOAD     = beta_load + f_mem
q_MSTORE    = beta_store + f_mem
q_MSTORE8   = beta_store8 + f_mem
keccak_permutations = 0 if input_length == 0 else floor(input_length / 136) + 1
keccak_zero_length_event = 1 if input_length == 0 else 0
q_KECCAK256 = beta_0 + beta_z * keccak_zero_length_event
              + beta_p * keccak_permutations + f_mem
q_MCOPY     = beta_0 + beta_w * copy_words + f_mem

p_operation = body_scale * q_operation + common_opcode_overhead
```

Here `q_operation` is the target opcode body cost recovered from the signed target/control relation,
not raw EVM gas. Static reference-opcode terms use the already reconstructed lab multipliers and are
subtracted before fitting. `memory_growth_event` is one only when logical memory grows.
`memory_evm_gas_delta` is the exact positive delta of `C(w) = 3w + floor(w^2 / 512)`, and
`memory_4k_boundary_event` is one when the operation crosses any additional 128-word logical-page
boundary beyond the warmed state, regardless of the number crossed. The manifest's executable
warmup allocates `initial_memory_words` before the target operation, so copy/input length and memory
expansion can vary independently. Metadata-only warmup would create an apparently full-rank matrix
over identical executions and is invalid.

Fit the three memory opcodes jointly with three opcode-specific constants and the three shared
memory coefficients. Subtract that shared body contribution before fitting KECCAK256 and MCOPY,
then add it back for their predictions and gates. In production space, add common overhead only to
the opcode-specific constants; scale shared coefficients by `body_scale` without adding overhead.
The shared matrix has exact rank six and the overall structured model has exact rank and parameter
count 14. The schema-3 diagnostic reports the shared family explicitly, alongside EXP, KECCAK256,
and MCOPY operation-specific models. Acceptance uses production-space fit MAPE at most 5%, fit
maximum APE at most 10%, and holdout maximum APE at most 10%. Failed quality gates produce
a content-addressed `not_supported` artifact rather than aborting or refitting the matrix. The
artifact is explicitly `candidate_eligible = false` and cannot update the production table.

Fresh run `25db29d91bd3311441fe8317` fit the former linear page-count model at 0.259% production
MAPE and 1.103% maximum APE, and passed the `0x0800` and `0x0fe0` holdouts, but consistently
overpredicted `0x2000` by about 42% across MLOAD, MSTORE, and MSTORE8. A post-hoc binary boundary
event reduced those viewed-row errors to 5.72%, 5.79%, and 5.96%. The viewed `0x2000` rows therefore
move into fit/diagnostic evidence, while `0x4000` becomes the fresh untouched holdout.

This second expanded memory matrix changes the frozen manifest identity again and requires a fresh
calibration run. Neither the earlier 39-scenario artifact nor run `25db29d91bd3311441fe8317` can
validate the revised shared-memory model.

Fresh run `8e2abe743f28f69ec2f89c8b` supplied that independent validation. Its shared-memory family had
exact rank six, 1.926% production MAPE, 7.416% fit maximum APE, and 8.887% holdout maximum APE. The
new `0x4000` production-space holdouts were 7.223% for MLOAD, 7.288% for MSTORE, and 7.410% for
MSTORE8, so the frozen shared `f_mem` hypothesis is supported. The aggregate dynamic artifact remains
`not_supported` and ineligible for candidate construction solely because the separate, now
superseded `input_words` KECCAK256 model failed its fit gates (10.318% MAPE and 29.384% maximum
APE); EXP and MCOPY are supported. This aggregate status does not invalidate the shared-memory
result, but the run cannot validate the replacement KECCAK256 model.

The replacement 73-scenario matrix keeps the validated shared-memory rows and expands KECCAK256 to
22 fit rows and seven fresh holdouts. Its semantic feature follows the measured REVM path: a
zero-length operation returns `KECCAK_EMPTY` without a permutation, inputs of 1 through 135 bytes
require one padded Keccak-f[1600] permutation, 136 through 271 require two, and each subsequent
136-byte interval adds one. All previously viewed rows are fit evidence; untouched holdouts are
frozen at 95, 333, 543, 544 (with 17 warmed memory words), 545, 1500, and 4096 bytes. The fit rows
cover both sides of the 136- and 272-byte boundaries; the holdouts add the 544-byte boundary and
distant extrapolation while retaining a warmed boundary case so that shared `f_mem` is subtracted
independently. The 67 noncanonical rows, exact scenarios, and
model splits are manifest-bound. A fresh calibration identity must validate the full replacement
matrix; run `8e2abe743f28f69ec2f89c8b` remains prior validation only for the unchanged shared-memory
family.

Fresh run `77cff7accc62325aad8c047b` accepted all 162 formal relations. The nonzero KECCAK256 rows
supported the permutation slope and all seven then-frozen holdouts stayed below 1.841% production APE,
but the two-parameter fit was `not_supported`: its zero-length fit row had 232.621% APE because
REVM returns `KECCAK_EMPTY` without entering the nonempty hash/syscall setup path. A read-only
diagnostic with the explicit zero-length event produced 0.603% fit MAPE, 1.388% fit maximum APE,
and 0.801% holdout maximum APE. This is motivation, not validation, and every viewed row moves to
fit evidence. Freeze the semantic branch and replacement holdouts in the implementation and require
a new calibration identity before calling the model supported. The zero-length branch itself has
no distinct length-domain holdout: its evidence is explicitly limited to REVM semantics and three
repeated fit measurements; fresh holdouts validate the nonzero permutation relation only.

Fresh run `e8663a09b3f35da37196cbdf`, bound to implementation revision `3b9ba3b3`, accepted all 169
formal relations and supported the exact-rank-14 structured model. KECCAK256's 22 fit rows produced
0.535% production MAPE and 1.378% maximum production APE; its seven untouched holdouts produced
0.675% maximum production APE. The production-space coefficients were approximately 678.14 for the
nonempty setup constant, -500.51 for the zero-length fast-path adjustment, and 2147.09 per padded
permutation, plus the independently fitted shared `f_mem`. EXP, MCOPY, and shared memory also
remained supported, while the entire diagnostic remained ineligible for candidate construction.
The scalar block-candidate path continued to fail closed on a nonpositive `opcode:0x19` multiplier;
this is a separate static lab-body affine/scalar precheck reached before dynamic-scalar validation.
It neither validates nor invalidates the structured dynamic result.

A post-run replay audit found that direct round-fit library calls inherited caller Decimal precision,
although the CLI had generated the run inside the required 80-digit context. Isolating the Decimal
context at `fit_formal_relation_round` makes default-precision replay reproduce all five persisted
rounds exactly. Because the CLI sampling and fitting already used the same 80-digit context, this
replay fix does not alter the sealed raw evidence or require a new empirical run.

The result may show that a term is stable, only approximately stable, or below the experiment's
resolving power. It is valid to merge indistinguishable terms or omit a negligible term in a future
production model, but only after reporting the resulting block-level residual. This diagnostic does
not assume that every member is uniquely recoverable. Shared `f_mem` is a V1 hypothesis, not an
architectural promise: future evidence may split it by opcode family or zkVM without changing the
independent non-memory multiplier model.

### Block Anchor Cohort

The controlled manifest declares all fit and holdout rows before any block result is opened. Every
row uses post-Unzen synthetic state and the production `sp1-shasta-proposal` guest. Rows must
exclude precompile and spawned-wrapper execution so those later dependencies cannot enter the
  anchor fit. The six dynamic-gas opcode totals must be zero or identical across every anchor-fit
  row; their separate holdouts cannot influence the six fitted parameters.

The frozen fit cohort retains its 40 rows and exceeds five rows per free parameter for the six
parameters. It must independently vary:

- valid stack-consuming arithmetic/`POP`-family work;
- `PUSH0`/PUSH and environment-producer work;
- `DUP1`/DUP-family work;
- `SWAP1`/SWAP-family work;
- proposal, block, started non-Anchor transaction, and native-transfer counts.

Within each opcode-anchor workload family, paired variants keep the deployment topology, block and
transaction envelopes, calldata and bytecode lengths, touched state keys, and every non-modeled
diagnostic feature count fixed. The fixed-width count immediate necessarily changes the bytecode
hash, serialized input, public output, and final state root; each value is bound exactly per row and
its proverGas effect is bounded by the data-control family below. Any other input-byte change, or a
variant that changes witness nodes, blobs, KZG work, or another undeclared feature, is ineligible
rather than silently assigning that work to an opcode anchor.

The transaction `gas_limit` is one of those frozen envelope fields and must not encode the loop
count. Count-bearing opcode rows use empty calldata, one fixed gas limit, one fixed bytecode length,
and a fixed-width immediate in the bytecode. Host tracing must reject a row that retains a runtime
count-source opcode such as `GAS` or `CALLDATALOAD` in the measured program.

Changing a fixed-width immediate still changes input bytes, code hash, final state root, and public
output. Before fitting, run a non-fitting data-control family that varies only that immediate and
executes an otherwise identical modeled operation sequence. Exact repeats of each individual row
remain mandatory. The range across distinct control inputs is recorded as a deterministic
cross-input data floor, not averaged into proverGas and not fitted as a coefficient. The observed
floor and every anchor-family signal/residual relative to it are reported. A fixture or guest change
invalidates the floor and requires a new calibration identity.

Controlled-block SP1 observations use the production proposal ELF and local gas-estimator engine.
Before the formal campaign, one frozen controlled input must demonstrate exact standard-versus-
estimator equality for proverGas, instruction count, syscall count, and public values. CLI
`--mode execute` denotes a local no-proof run; `--sp1-execution-engine gas-estimator` selects the
actual execution implementation.

The gas estimator is an offline calibration and validation oracle only. It is not a production
online quote or admission path; online operation continues to use the host-native operation ledger
and the sealed coefficient table.

Fixture generation must compute both staged exact-rational design matrices before SP1 execution.
The four opcode-family count slopes over `[x_j * B * C]` must have transfer rank two, and the 40
fit-row `q_j` matrix must have fixed/base rank four. It must not round `B`, `C`, family slopes, or
either rank input.
The manifest also freezes at least eight controlled holdout rows that are excluded from fitting and
exercise every anchor family and every fixed/base feature.

Every fit and holdout row runs three times. Primary `proverGas`, public values, trace ledger,
feature counts, and GuestInput hash must be identical across repeats. A trace with an unresolved
operation, ambiguous measurement key, system/Anchor ownership violation, precompile, or spawned
wrapper is ineligible.

## Fit And Acceptance

1. Within each of the four opcode families, fit the observed `p_j`, `x_j * mu_zero`,
   `x_j * B * synthetic_body_cost / raw_gas`, and `x_j * B / raw_gas` against the frozen workload
   count with a free intercept. This within-family slope removes startup and every fixed/base
   feature, which are constant inside the family.
2. Solve the resulting four-by-two Decimal slope system for `[a, h]`. Do not jointly fit transfer
   parameters with the much larger startup/base columns; that permits those columns to compensate
   for the smaller opcode signal and can produce nonphysical anchors.
3. Reconstruct the four anchors as `theta_i = (a * s_i + h) / g_i`, then all 102 opcode
   multipliers as `c_p = mu_zero + B * theta`.
4. Freeze `c_p`, subtract `x_j * c_p` from all 40 fit rows, and solve only the rank-four `q_j`
   system for `beta`.
5. Compute the existing exact ADD-normalized view `m_p(k) = c_p(k) / c_p(ADD)` and apply both
   stages without refitting to the controlled holdout rows.

The fit is accepted only when:

- the exact opcode-family slope matrix has rank two and the fixed/base matrix has rank four;
- `body_scale` is positive, `common_opcode_overhead_per_operation` is nonnegative, and every fitted
  fixed/base parameter is finite and positive;
- every reconstructed opcode multiplier and every required fixed/base cost is positive;
- every fitted opcode-family production-slope APE is at most 10%, and extrapolating each family to
  its frozen larger-count holdout has signal-relative APE at most 10%;
- fit MAPE is at most 5% and maximum fit-row APE is at most 10%;
- maximum holdout-row APE is at most 10%;
- every dynamic raw-gas holdout and per-key consistency gate passes;
- removing any one opcode family retains transfer rank two and predicts that omitted family's
  complete production slope with APE at most 10%; removing any one workload family also retains
  fixed/base rank four;
- all required relation, provenance, ownership, and repeat checks pass.

APE is always:

```text
APE = abs(predicted_prover_gas - actual_prover_gas) / actual_prover_gas
```

No positivity shift or post-fit offset repair is allowed. A provisional offset may be displayed
for diagnostic plots, but it must be labeled non-candidate and cannot enter fit inputs, digests,
reports, or sealing.

## Artifacts And Sealing

The calibration run adds independently hashed artifacts for:

- canonical relation rows, accepted signed slopes, gates, and equation-matrix digest;
- the derived `mu_zero`, `B`, anchor ordering, rank, and natural-anchor verification;
- controlled block fit/holdout row identities, exact feature matrices, and raw observations;
- dynamic raw-gas holdout identities, signed slopes, predictions, APE values, and per-key
  consistency results;
- fitted anchors, fixed/base costs, reconstructed opcode multipliers, staged residuals, family
  slopes, APE values, holdout extrapolations, and leave-one-family-out predictions.

The candidate root transitively binds those artifacts, the existing precompile and spawned-wrapper
components, provenance, formula, thresholds, and the complete review-only cost table. A changed
relation, anchor ordering, block row, split, threshold, source revision, dependency, manifest, guest
artifact, or fit result creates a new calibration identity and candidate digest.

`candidate.sha256` must exist before `prepare-validation` can acquire or execute final proposal
rows. Integration-smoke rows remain excluded from fitting and final validation.

## Failure Handling

Fail closed with a machine-readable reason when:

- the relation matrix is not exactly rank 98 or the selected natural anchors do not leave rank 98;
- a required relation is absent, rejected, or bound to a different target/control program;
- the block matrix is rank-deficient or contains an undeclared row;
- a row mixes revisions, manifests, guest artifacts, engines, cadence, or trace schemas;
- a fit or holdout row contains unresolved, system/Anchor-owned, precompile, or spawned work;
- a dynamic-gas key or scenario is undeclared, has the wrong raw-gas totals, or fails its holdout;
- a proposal-purpose row is presented to any calibration command;
- a reconstructed required multiplier or fixed/base cost is non-positive, or any acceptance gate
  fails.

Failure preserves all raw artifacts and emits no seal. It never falls back to the current production
table and never changes the frozen row set after results are visible.

## Verification

Implementation tests must prove:

1. The checked-in 102-key manifest derives 98 independent nonzero relations and the four natural
   anchors leave a full-rank 98-column system.
2. A synthetic known-cost staged fixture recovers both transfer parameters, all four reconstructed
   anchors, all four fixed/base coefficients, and all 102 reconstructed opcode multipliers exactly,
   including dynamic-gas operation rows.
3. Signed negative relations are accepted when their gates pass; self-controls remain zero-only
   checks.
4. EXP, KECCAK256, MLOAD, MSTORE, MSTORE8, and MCOPY require frozen multi-raw-gas holdouts; a
   scenario-dependent multiplier prevents sealing without changing the canonical relation fit.
5. Missing relations, wrong relation orientation, rank-deficient block rows, proposal-purpose rows,
   unresolved operation traces, and non-positive reconstructed costs are rejected.
6. Opcode-slope holdout, block holdout, and leave-one-family-out prediction failures prevent
   sealing without modifying raw observations.
7. Candidate hashing binds every relation, parameterization, dynamic holdout, controlled-block,
   threshold, and fit artifact.
8. Existing precompile, spawned-wrapper, bridge isolation, provenance, resume, and final-validation
   invariants continue to pass.

## Execution Order

1. Implement and test canonical relation-matrix derivation and four-anchor parameterization.
2. Promote matched-control sampling into the formal repeat/gate artifact.
3. Add and run manifest-frozen dynamic raw-gas holdouts without changing the canonical equation
   system.
4. Add manifest-frozen block anchor fit and holdout fixtures plus pre-execution rank validation.
5. Implement the rank-two transfer-slope stage, freeze the opcode table, then implement the
   rank-four fixed/base stage and reconstruction gates.
6. Reconnect precompile, spawned-wrapper, candidate sealing, and bridge sampling to the
   reconstructed opcode table.
7. Create a fresh calibration identity and rerun the frozen relation cohort.
8. Run controlled block calibration and all holdouts, then seal the review-only candidate.
9. Only after sealing, run the separately frozen final proposal validation corpus.
