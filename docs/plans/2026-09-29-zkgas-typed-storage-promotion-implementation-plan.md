# zkGas Typed Storage Promotion Implementation Plan

## Task 1: Trace-schema contract tests

- Add failing serde and live-inspector tests for schema-3 storage inputs.
- Cover warm/cold `SLOAD` and the six execution-time `SSTORE` branches.
- Cover access-list warmth, uncached slots, dirty slots, rollback, nested frames, instruction-level
  failure gaps, and outer zkGas-limit ordering.

## Task 2: REVM journal feature capture

- Extend `crates/zkgas-trace/src/inspector.rs` with typed storage access and branch enums.
- Capture slot/value and pre-step journal metadata without mutation.
- Finalize the feature from the post-step journal; fail closed when execution did not reach it.
- Bump `OPERATION_TRACE_SCHEMA_VERSION` to 3 and update reconstruction/serialization tests.

## Task 3: Composite estimator contract tests

- Add failing tests for exact sealed-source loading and the nine `M_typed` parameters.
- Add trace-estimation tests for every storage access/branch combination and malformed inputs.
- Assert `SLOAD/SSTORE` are measured while persistent state/trie work remains outside their owner.

## Task 4: Promote the sealed typed-storage model

- Bind the exact sealed result directory and file hashes.
- Add a separate typed-storage model to the estimator artifact; do not rewrite the historical opcode
  registry.
- Promote only coverage rows `opcode:0x54` and `opcode:0x55`.
- Evaluate storage typed inputs with Decimal arithmetic and preserve negative cold modifiers.
- Bump the estimator artifact schema and trace schema binding.

## Task 5: Verification and sealing

- Run formatting, focused Rust tests, focused Python tests, and the full opcode experiment suite.
- Build/check the affected guest contract if trace integration changes its build input.
- Obtain independent adversarial review and independent behavioral verification; fix and recheck all
  material findings.
- Commit the coherent implementation, verify from a clean tree, seal a new review-only composite
  estimator, and verify exact replay.

## Task 6: Diagnostic block replay

- Recreate schema-3 traces for the two already-open ad-hoc proposal fixtures.
- Apply the sealed estimator once without tuning.
- Record old/new gaps, coverage, predicted proverGas, actual proverGas, and APE.
- Keep the final 60-proposal corpus closed.
