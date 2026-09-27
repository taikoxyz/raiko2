# SP1 Composite Block Estimator Implementation Plan

**Goal:** Combine the sealed opcode registry and controlled higher-layer measurements into one
coverage-qualified SP1 estimator, then measure its accuracy on real Shasta blocks/proposals without
changing the production ZKGas schedule.

**Spec:** `docs/plans/2026-09-26-zkgas-calibration-design.md`

**Progress:** `docs/plans/2026-09-26-zkgas-calibration-progress.md`

## Status

In progress. The opcode registry, operation-coverage ledger, and controlled higher-layer evidence
are sealed. Proposal validation has not opened.

## Global Constraints

- V1 predicts SP1 `proverGas`; it does not change a production multiplier table or block limit.
- Historical derivations remain immutable and replayable with their original semantics.
- Registry parameters are already production-scaled. A new estimator must not apply `body_scale`
  again.
- The estimator owns each contribution exactly once: proposal startup, block base, started
  non-Anchor transaction base, committed native-transfer approximation, and executed operations.
- Structured opcode models consume typed execution features. Missing or incompatible features are
  explicit coverage gaps; they never fall back to current-schedule multipliers.
- Unsupported opcodes, direct precompiles, and unmeasured wrappers remain explicit gaps.
- A partial trace may emit a modeled subtotal and coverage report, but not a complete prediction.
- Final proposal rows cannot tune the sealed estimator. Integration smoke rows must be outside the
  final corpus.
- Use canonical `Decimal` strings and content-address every promoted artifact.
- Keep all tracked paths repository-relative and all source artifacts immutable.

## Task 1: Separate Historical Replay From Canonical Units

- Add an exact equivalence regression between a static typed-registry event and the canonical
  higher-layer operation resolver.
- Preserve the legacy double-scaled resolver only for replaying the historical higher-layer
  package.
- Add a canonical resolver that consumes the production-scaled registry coefficient exactly once.
- Recompute corrected fixed costs and state-holdout verdicts from the sealed raw evidence without
  rerunning SP1. Bind the old source package and the new implementation identity.

## Task 2: Emit Typed Structured-Opcode Inputs

- Version the operation-trace schema.
- Emit exact model input for static opcodes, EXP, KECCAK256, MLOAD/MSTORE/MSTORE8, and MCOPY from
  interpreter state at execution time.
- Capture EXP exponent byte length, KECCAK input length, MCOPY word count, and memory-growth
  features required by the sealed registry.
- Reject missing, extra, or model-incompatible fields. Do not infer them after execution.
- Preserve CALL/CREATE spawn substitution and trace A/B parity.

## Task 3: Seal The Composite Estimator

- Build a strict registry loader and deterministic estimator artifact from the exact opcode core,
  coverage ledger, corrected higher-layer replay, trace schema, ownership policy, and source hashes.
- Sum proposal startup once; block base per block; transaction base per started non-Anchor
  transaction; the frozen native-transfer approximation only for committed native EOA transfers;
  and typed executed-operation costs.
- Emit per-layer contributions, independent coverage denominators, and exact gap records.
- Emit `predicted_prover_gas` only for complete coverage; otherwise emit only `modeled_subtotal`.
- Add create-only seal and exact replay verification commands.

## Task 4: Validate On Real Inputs

- Build release preflight and guest-launcher artifacts and verify the sealed ELF/VK identities.
- Run one Hoodi and one Mainnet integration smoke outside the final corpus. Smoke validates trace,
  joins, feature extraction, and estimator execution only; it cannot tune coefficients.
- Freeze the final 40 Hoodi plus 20 Mainnet corpus only after the estimator is sealed.
- Run host trace and SP1 gas estimation for every final row, join exact GuestInput/public-output
  identities, and report per-proposal APE, MAPE, maximum APE, coverage, and gaps.
- Keep corpus publication as an explicit later operation requiring a supplied immutable object URI.

## Verification

- Run focused Python tests for canonical units, source replay, aggregation, coverage, tamper
  rejection, and report math.
- Run focused Rust tests for trace feature capture, serialization, memory boundaries, and spawn
  behavior.
- Run the complete opcode-gas Python suite, `cargo test -p raiko2-zkgas-trace`, formatting checks,
  and `git diff --check` before each pushed checkpoint.
- Require independent adversarial review of every non-trivial implementation task and an independent
  end-to-end verification before opening the final corpus.
