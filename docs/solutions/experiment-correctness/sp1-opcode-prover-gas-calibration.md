---
title: SP1 Opcode ProverGas Calibration Pitfalls
date: 2026-09-21
category: experiment-correctness
module: opcode gas experiment
problem_type: measurement_correctness
component: experiments/opcode-gas
symptoms:
  - Increasing an opcode count produced a smaller proverGas result
  - Most opcode slopes were negative or dominated by fixture setup
  - Zero-operand and non-zero-operand runs disagreed for the same opcode
  - A calibration run became invalid after experiment implementation changed
root_cause: confounded_measurement
resolution_type: fixture_isolation
severity: high
related_components:
  - guest-launcher
  - sp1-revm-opcode-lab
  - zkgas-calibration
tags:
  - sp1
  - provergas
  - opcode
  - matched-control
  - operand-profile
  - calibration
---

# SP1 Opcode ProverGas Calibration Pitfalls

## Problem

An opcode sweep can look precise while measuring the wrong delta. Transaction gas, bytecode size,
helper instructions, operand-dependent branches, guest runtime startup, and output handling can all
change SP1 `proverGas`. A linear slope is meaningful only after those dimensions are held constant or
represented as explicit scenarios.

The opcode lab has two different roles:

- matched-control diagnostics quickly reveal fixture and execution-path problems;
- the formal controlled workflow applies repeat, isolation, fit, required-case, and out-of-fit gates
  before producing a review-only candidate.

A diagnostic result must not bypass the formal gates or become a production multiplier.

## Symptoms And Root Causes

### ProverGas Decreased As ADD Count Increased

The original sweep changed transaction gas and the amount/layout of helper bytecode together with the
target count. SP1 executed a different transaction/program shape for each point, so the difference
was not the marginal cost of ADD. Fixing only the fit cannot repair a confounded fixture.

Use one fixed `generator_max_count`, fixed transaction gas, fixed serialized bytecode length, and
fixed microprogram slots. Only the first `x` slots switch from control to target. Validate the actual
execution trace, not only the generator's declared counts.

### A STOP-Based Unary Control Produced A Large Negative Signal

Placing STOP before an unreachable unary opcode changed bytecode analysis and dispatch behavior. It
did not represent a zero-cost version of the same execution slot. Unary diagnostics now use NOT as a
same-shape reference: target slots execute `OP`, control slots execute `NOT`, and NOT itself is the
zero-delta self-control.

The result is therefore `OP - NOT`, not an absolute unary opcode cost.

The broader 41-case diagnostic showed that fixed byte footprint and transaction gas are still
insufficient when an early `STOP` changes the executed program or its final effects: all 41 sweeps
were non-monotonic and 40 produced negative deltas. Increasing the count cannot repair that
confound. Matched controls must instead bind and validate the exact executed reference program,
including any padding and shared suffix, while preserving the intended final stack effect.

### Zero Operands Hid Or Selected Special Paths

The compatibility profile used `[0, 0]` for binary operations. That selects special behavior for
equality, signed comparison, zero divisors, and related arithmetic paths. A fixed small-nonzero
profile showed large changes for EQ and SLT/SGT, material changes for DIV/MOD/SDIV/SMOD, and smaller
changes for EXP and LT/GT.

Do not generalize one successful scenario to every event sharing the opcode. Predeclare required
scenarios for an operation key. If any required scenario is missing, rejected, confounded, or outside
the consistency gate, the entire key remains unmeasured unless the trace schema can distinguish and
resolve those scenarios exactly.

### Fast Execution Was Mistaken For A New Cost Model

The `gas-estimator` engine runs the actual SP1 opcode-lab program with `GasEstimatingVM`; it does not
predict gas from opcode counts. Focused parity runs showed the same public values, instruction
counts, and normalized `ExecutionReport::gas()` as the standard engine for sampled workloads while
reducing individual executions from tens of seconds to roughly two-tenths of a second on the test
machine.

This guarantee depends on the recorded engine and canonical gas-estimation cadence. Never mix rows
with different `sp1_execution_engine`, `gas_trace_chunk_threshold`, or `gas_trace_chunk_slots`.

The same estimator can consume the production proposal ELF and a production `GuestInput` for local
controlled-block calibration. A fixed-input parity check must precede the campaign. In the
fixed-limit spike, the standard engine and estimator both returned `159030265` proverGas,
`136594192` instructions, `229747` syscalls, and identical public values. The optimized estimator
took about 7.4 seconds wall time versus about 90 seconds for the standard engine on that machine.
This does not imply that every proposal is sub-second or fixed-time.

The CLI still uses `--mode execute` because the run executes locally without producing a proof. The
independent `--sp1-execution-engine gas-estimator` option selects the fast implementation. Do not
mistake the word `execute` in the mode for use of the standard executor, and do not require the
standard engine merely to obtain an `ExecutionReport` or public values; the estimator returns both.

### Old Calibration Artifacts Were Reused After Implementation Changed

Calibration identity binds the implementation revision, controlled manifest, Alethia revision, SP1
version, guest artifacts, and execution parameters. Continuing an old directory after any bound
input changes mixes incompatible cohorts even when the filenames still look correct.

Preserve the old directory. At a clean new revision, run `prepare-calibration` again and use its new
content-addressed ID for all generation and execution. Do not edit or overwrite an old run to make it
look current.

### Opcode And Block Overhead Sweeps Were Conflated

Opcode/precompile cases measure controlled operation deltas. Proposal startup, block base,
transaction base, and native value transfer use separate production-guest fixtures and residual
accounting. Trie, Merkle, system/Anchor, and fixed hashing work stays in the V1 block-base residual
unless a later experiment defines an independently controllable unit.

An opcode loop does not validate the complete block model, and a block/proposal failure does not by
itself prove that an opcode slope failed to converge. Report the exact phase, generator bound,
count prefix, checkpoint, and repeat before diagnosing the model.

### Gas Limit Was Accidentally Used As The Block-Loop Counter

One controlled-block prototype encoded the loop count by setting `gas_limit = base + count`, reading
`GAS` inside the contract, and subtracting a measured base. It produced the requested operation
counts, but the transaction envelope changed at every sample. The resulting proverGas slope mixed
the anchor operation with gas-limit-dependent guest work and was not an eligible anchor fit.

Keep the transaction gas limit fixed. Encode the count in a fixed-width bytecode immediate, keep
calldata empty, and pad every program to the same bytecode length. The fixed-limit spike used a
`PUSH3` count, `gas_limit = 100000`, and 256-byte code. Host-native tracing must prove the exact raw
gas map and unchanged declared feature/diagnostic shape before SP1 estimation.

Changing the immediate still changes the deployed bytecode hash, final state root, public output,
and serialized input bytes. A control that discarded the count and executed identical modeled EVM
work therefore did not have identical proverGas across different immediates: six counts spanned
`159029040..159029183`, a range of 143. Repeating the exact same input was bit-for-bit deterministic.
Treat that 143 as a measured cross-input data floor for this fixture revision, not as run-to-run
randomness and not as a coefficient. Required repeats of one row must still match exactly. Anchor
signal and residuals should be reported against the frozen control floor; changing the fixture,
guest, or input encoding requires measuring a new floor under a fresh calibration identity.

### Anchor Families Passed The Pre-Campaign Linearity Check

Before opening a formal calibration run, the fixed-limit production-guest fixtures were exercised
at counts `1, 2, 4, 8, 16`, with count `32` reserved as an out-of-fit checkpoint. Every input was
run three times through the estimator and all primary gas, instruction, syscall, public-output,
input-identity, trace, feature, diagnostic, and final-root fields matched exactly within a row.

The exploratory fit produced:

| family | fit slope | fit signal / data floor | R2 | count-32 delta APE |
| --- | ---: | ---: | ---: | ---: |
| `pop_family` | 895.13 | 94.02 | 0.999964 | 0.69% |
| `push_family` | 1005.32 | 105.69 | 0.999961 | 0.52% |
| `dup_family` | 882.48 | 92.41 | 0.999980 | 0.21% |
| `swap_family` | 1140.99 | 119.46 | 0.999717 | 0.74% |

The 32-count checkpoint signal was 191--245 times the 143-gas data floor. This is sufficient to
qualify the fixed-limit fixture shape for the frozen 40-fit plus eight-holdout block campaign; it
does not establish final coefficients. Each family program executes a trace-validated combination
of the named anchor and helper opcodes. The later exact relation basis and joint block fit separate
those components. Do not publish a row's family slope as the pure cost of its namesake opcode.

This qualification batch was intentionally throwaway evidence collected before the candidate path
was complete. Finish candidate construction and full implementation verification first, then create
one fresh calibration identity and rerun formal relations and all controlled block rows. Otherwise
the subsequent implementation commit changes the bound source revision and invalidates the data.

### Backend Metrics Were Treated As Interchangeable

SP1 `ExecutionReport::gas()`, SP1 total instruction count, and RISC0 user/padded cycles are different
metrics. Instruction count is a secondary SP1 cycle proxy; `proverGas` additionally represents
trace-area, complexity, syscall, and system-chip work. RISC0 cycles require independent measurement
and a separately validated bridge before scalar transport is used.

Do not place coefficients from different metrics into one table or reuse an SP1 normalization factor
as a RISC0/Boundless conversion.

## Safe Workflow

1. Start from a clean revision and create a new content-addressed calibration run.
2. Freeze the manifest, guest artifacts, SP1 version, execution engine, cadence, fixed transaction
   gas, and generator bound.
3. Run matched-control smoke diagnostics with at least zero and non-zero operand profiles.
4. Compare target and control using identical setup, bytecode footprint, final stack height, and gas
   limit. Treat `OP - reference` as contextual until an absolute anchor is established.
5. Keep operand-sensitive paths as explicit required scenarios; do not average them away after seeing
   results.
6. Run the formal controlled suite with three repeats, fixed count prefixes, isolation checks, and
   the frozen out-of-fit checkpoint.
7. Measure startup/block/transaction/transfer overhead separately through the production proposal
   guest using the estimator after a fixed-input standard/estimator parity check.
8. Seal the candidate before opening final proposal-validation results. Proposal rows validate; they
   never fit or repair coefficients.

## Prevention

- Reject mixed execution engines, cadence settings, revisions, manifests, and guest artifacts.
- Bind operand profile, resolved operands, exact GuestInput bytes, and target/control pair identity.
- Keep fixed-footprint and fixed-transaction-gas assertions in executable tests.
- Never derive a loop count from `GAS` or vary `gas_limit` with the count in a block-anchor cohort.
- Distinguish exact-repeat determinism from small deterministic differences between distinct input
  bytes; preserve and report a frozen data-control range instead of averaging it into a coefficient.
- Require count-zero target/control deltas to be zero and inspect monotonicity before fitting.
- Inspect instruction-count and proverGas deltas together; disagreement often reveals a runtime or
  system-chip boundary that an opcode-only model cannot express.
- Preserve every old run. A changed implementation always starts a new calibration identity.
- Label diagnostic outputs `contextual_relative` and keep them outside candidate construction.
- Do not claim that fast opcode-lab timing is a full-block estimation latency guarantee.

## Related Documentation

- [Opcode workload metric experiment](../../../experiments/opcode-gas/README.md)
- [SP1 opcode proverGas experiment implementation plan][implementation-plan]
- [zkGas multiplier recalibration design](../../plans/2026-09-06-zkgas-multiplier-recalibration-experiment-design.md)

[implementation-plan]: ../../plans/2026-06-08-sp1-opcode-prover-gas-experiment-implementation-plan.md
