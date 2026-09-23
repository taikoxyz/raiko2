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

The experiment does not need to invent standalone absolute opcode costs. Controlled blocks through
the production SP1 proposal guest can fit those four anchors together with the four already-declared
fixed/base costs.

## Goals

1. Promote accepted matched-control slopes from diagnostics into the formal opcode relation model.
2. Preserve the four unidentified opcode anchor multipliers as explicit model parameters.
3. Fit those four anchors and the four required fixed/base costs using only frozen controlled block
   fixtures executed through `sp1-shasta-proposal`.
4. Reconstruct one positive SP1-native raw-gas multiplier for every required opcode, then derive
   the existing ADD-normalized view.
5. Validate dynamic-gas opcode multipliers on frozen raw-gas ranges without refitting them.
6. Seal the complete controlled candidate before opening final Mainnet or Hoodi proposal results.

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

The fitted model is:

```text
p_hat_j = x_j * (mu_zero + B * theta) + q_j * beta
        = x_j * mu_zero + [x_j * B, q_j] * [theta, beta]
```

Only the eight parameters `[theta, beta]` are fitted. The 98 accepted relation slopes remain frozen;
block rows cannot alter them. The fit uses Decimal arithmetic and the predeclared controlled rows
only.

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
The resulting formal relation artifact uses schema version 2; schema version 1 does not carry the
activation/tail evidence and is not accepted by candidate construction.

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

### Dynamic Raw-Gas Holdouts

The current V1 pure-opcode set has six keys whose actual interpreter raw gas can change for the
same opcode identity: `EXP`, `KECCAK256`, `MLOAD`, `MSTORE`, `MSTORE8`, and `MCOPY`. The manifest
marks these keys explicitly and rejects an unmarked dynamic-gas template.

For each dynamic key, freeze one canonical relation scenario plus at least two non-fitting holdout
scenarios before SP1 output is opened. Their host-native traces must expose at least three distinct
positive target raw-gas totals. The middle target raw-gas total must be at least twice the canonical
total, and the largest must be at least four times the canonical total. The manifest binds the
operand, exponent width, memory offset, input/copy length, initial memory state, expected reference
program, and expected target/reference raw-gas totals for every scenario.

Only the canonical scenario contributes a row to the rank-98 relation system. After block fitting
reconstructs `c_p`, apply it without refitting to every dynamic holdout:

```text
predicted_relation_slope(h) = A_h * c_p
relation_APE(h) = abs(predicted_relation_slope(h) - observed_relation_slope(h)) /
                  abs(observed_relation_slope(h))

implied_mu_h(k) = (observed_relation_slope(h) - reference_raw_gas_terms(h, c_p)) /
                  target_raw_gas(h, k)
```

Every observed holdout slope must be finite and nonzero, have the predicted sign, and satisfy
`relation_APE <= 0.10`. For each dynamic key, all canonical and holdout `implied_mu` values must be
positive and satisfy `max(implied_mu) / min(implied_mu) - 1 <= 0.05`. A failed dynamic holdout
rejects the key and prevents candidate sealing; it never adds a row, changes an offset, selects a
new scenario, or repairs the multiplier.

### Block Anchor Cohort

The controlled manifest declares all fit and holdout rows before any block result is opened. Every
row uses post-Unzen synthetic state and the production `sp1-shasta-proposal` guest. Rows must
exclude precompile and spawned-wrapper execution so those later dependencies cannot enter the
anchor fit. The six dynamic-gas opcode totals must be zero or identical across every anchor-fit
row; their separate holdouts cannot influence the eight fitted parameters.

The fit cohort contains at least five rows per free parameter: at least 40 rows for the eight
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

Fixture generation must compute the exact rational design matrix before SP1 execution and reject it
unless all eight columns have exact full rank. It must not round `B`, `x_j * B`, or the rank input.
The manifest also freezes at least eight controlled holdout rows that are excluded from fitting and
exercise every anchor family and every fixed/base feature.

Every fit and holdout row runs three times. Primary `proverGas`, public values, trace ledger,
feature counts, and GuestInput hash must be identical across repeats. A trace with an unresolved
operation, ambiguous measurement key, system/Anchor ownership violation, precompile, or spawned
wrapper is ineligible.

## Fit And Acceptance

1. Subtract the frozen `x_j * mu_zero` term from every fit observation.
2. Solve ordinary least squares for the eight Decimal parameters `[theta, beta]`.
3. Reconstruct all 102 opcode multipliers as `c_p = mu_zero + B * theta`.
4. Compute the existing exact ADD-normalized view `m_p(k) = c_p(k) / c_p(ADD)`.
5. Apply the frozen model without refitting to the controlled holdout rows.

The fit is accepted only when:

- the exact fit matrix has rank eight;
- every fitted parameter is finite;
- every reconstructed opcode multiplier and every required fixed/base cost is positive;
- fit MAPE is at most 5% and maximum fit-row APE is at most 10%;
- maximum holdout-row APE is at most 10%;
- every dynamic raw-gas holdout and per-key consistency gate passes;
- refitting after removing any one predeclared workload family changes every opcode anchor and
  fixed/base coefficient by at most 5%;
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
- fitted anchors, fixed/base costs, reconstructed opcode multipliers, residuals, APE values, and
  leave-one-family-out stability results.

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
2. A synthetic known-cost fixture recovers all four anchors, all four fixed/base coefficients, and
   all 102 reconstructed opcode multipliers exactly, including dynamic-gas operation rows.
3. Signed negative relations are accepted when their gates pass; self-controls remain zero-only
   checks.
4. EXP, KECCAK256, MLOAD, MSTORE, MSTORE8, and MCOPY require frozen multi-raw-gas holdouts; a
   scenario-dependent multiplier prevents sealing without changing the canonical relation fit.
5. Missing relations, wrong relation orientation, rank-deficient block rows, proposal-purpose rows,
   unresolved operation traces, and non-positive reconstructed costs are rejected.
6. Holdout and leave-one-family-out failures prevent sealing without modifying raw observations.
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
5. Implement the eight-parameter block solver and reconstruction gates.
6. Reconnect precompile, spawned-wrapper, candidate sealing, and bridge sampling to the
   reconstructed opcode table.
7. Create a fresh calibration identity and rerun the frozen relation cohort.
8. Run controlled block calibration and all holdouts, then seal the review-only candidate.
9. Only after sealing, run the separately frozen final proposal validation corpus.
