# Top-level creation workload implementation plan

> **For agentic workers:** use subagent-driven-development for implementation and independent review/verification. Preserve the existing experimental worktree and source identities.

**Goal:** Construct and measure repeated top-level creations with the existing canonical GuestInput tools.

**Architecture:** One optional typed transaction descriptor extends the current constructor; no second input format or guest changes. Existing fixture observer/reconstruction and guarded SP1 harness remain authoritative.

**Tech Stack:** Rust fixture feature, existing Raiko/Reth native replay, Python stdlib producers and fixed SP1 SDK6.3 estimator.

**Spec:** `docs/research/2026-10-30-top-level-create.md`.

## Global constraints

- Preserve literal actual-SP1-PGU/node-known-zkGas ranking, full quotes and controls separately.
- Preserve omitted-descriptor canonical bytes/report, existing gas budgets, tariff and frozen guest identity.
- Host-only sources, no guest build, generated model edits or production changes.
- Python uses the existing configured virtual environment; serialize Cargo use.

## Review focus

- Declared per-tx gas and exact inclusion order must not silently drop later candidates.
- Empty initcode may execute an implicit STOP; derive the real observation rather than inventing a zero.
- Created account/code state and generated code in later witnesses must follow actual execution.
- Reverted/exceptional creations retain receipt gas, nonce consumption and observed attempted work.
- Parser, blob/gas bounds and unsupported composition fail without publishing outputs.

## Task1: Typed descriptor and native construction

Files: `bin/zkgas/src/fixture/{mod,constructor,observer}.rs`, new focused descriptor module,
`bin/zkgas/tests/fixture.rs`, `bin/zkgas/README.md`.

- [ ] Add failing tests for repeated call/creation, empty initcode, RETURN deployment, failure/state continuity, malformed/unsupported inputs and unchanged omission parity. Record RED before implementation.
- [ ] Implement the optional descriptor, sequential signed candidates, exact all-user gates, honest prestate/code continuity, all-user observation and optional detailed report.
- [ ] Run fixture suite, focused strict Clippy and fmt. Independently review the complete diff and independently replay canonical inputs before committing.

## Task2: Finite producer and measurements

Files: new `bin/zkgas/bench/cases/top_level_create.py`, matching Python tests,
`bin/zkgas/bench/README.md`, research checkpoint above.

- [ ] Add failing generator tests for deterministic matrix, bounds, routes, matching references and identities; implement the finite producer.
- [ ] Qualify every planned request natively, freeze source/binary/model/program identities, then run all plans under existing sequential SP1 guards. Retain exclusions and failures.
- [ ] Independently verify actual reports/raw joins/rankings, preserving full and matched-increment diagnostics.
- [ ] Change tariff only if measurements establish a gap; otherwise document the unchanged-model result. Publish data-only Atlas fixture and update existing tool/pricing PR evidence after review.
