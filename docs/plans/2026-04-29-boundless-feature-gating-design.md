# Boundless Feature Gating Design

**Date:** 2026-04-29

## Goal

Make Boundless support optional so the default `raiko2` workspace build, clippy, and tests do not
compile `boundless-market` or require the Boundless prover stack.

## Problem

`boundless-market = 1.0.0` currently generates nondeterministic Solidity bindings in CI. The
resulting `boundless_market_generated.rs` sometimes ends with dangling NatSpec comments, causing
`alloy::sol!` to fail at compile time. The failure blocks:

- `raiko2-prover`
- `raiko2-engine`
- `bin/raiko2`
- any CI lane that compiles those packages with Boundless enabled

Local builds can appear stable because `cargo clean` does not reset the unpacked cargo registry
source tree, while GitHub runners rebuild from fresh cargo registry contents each time.

## Constraints

- Do not vendor `boundless-market` into this repository.
- Do not change the public config file shape unless necessary.
- Keep the Boundless route model (`risc0/boundless`, `ShastaRisc0Boundless`) in the canonical
  route/pipeline enums so persisted runtime records and API payloads continue to round-trip.
- When the `boundless` feature is disabled, do not register the Boundless engine in `raiko2`.
- When the feature is disabled, allow config files to contain Boundless fields without failing
  validation solely because the feature is off.

## Selected Approach

Introduce a new `boundless` cargo feature and make it default-off.

### Always-on surface

Keep these pieces available without the feature:

- Boundless config and offer parameter types used by config parsing
- Boundless progress/resume data structures used by runtime/task metadata
- Boundless aggregation input adapter logic that does not depend on `boundless-market`
- Canonical pipeline/route enums and string forms

### Feature-gated surface

Compile these only with `--features boundless`:

- `boundless-market` dependency
- `BoundlessProver` implementation and external client logic in `raiko2-prover`
- Boundless engine registration/builders in `raiko2`
- Boundless-specific tests that require the prover implementation

## Runtime Behavior

### Feature enabled

Behavior remains unchanged:

- `config.prover.runner = "boundless"` is valid for RISC0
- `ShastaRisc0Boundless` engine is created
- explicit `risc0/boundless` requests resolve to the Boundless engine

### Feature disabled

Behavior changes in a narrow way:

- Boundless config fields may still be present and parse successfully
- startup does not register a Boundless engine
- the default RISC0 route selection in HTTP handlers falls back to `risc0/local` even if
  `config.prover.runner = "boundless"`
- explicit `risc0/boundless` requests still parse as a canonical route, but engine resolution
  returns the existing `404 pipeline not available: shasta-risc0-boundless`

This matches the requested behavior: “not registered” rather than “config rejected”.

## Code Structure

### `crates/prover`

Refactor the current `boundless` module into:

- always-on types/helpers module
- feature-gated prover/client module

`boundless-market` becomes an optional dependency behind a `boundless` crate feature.

### `crates/engine`

Propagate the `boundless` feature to `raiko2-prover`. Keep common observer/progress types
available. Boundless-specific tests are gated.

### `bin/raiko2`

Propagate the feature to `raiko2-engine` and `raiko2-prover`. Gate:

- Boundless engine type aliases
- engine construction/registration
- Boundless-ready checks that require actual prover wiring
- Boundless e2e/fixture tests

Keep config parsing types available without the feature so existing config files remain accepted.

## Testing Strategy

### Default feature set

Verify that these compile and test without `boundless-market` in the graph:

- `cargo test -p raiko2-prover`
- `cargo test -p raiko2-engine`
- `cargo test -p raiko2`
- `cargo clippy -p raiko2 --no-default-features -- -D warnings` is not required if `boundless` is
  default-off, but targeted clippy on the default build should pass

Add focused tests for:

- default RISC0 route falls back to local when the feature is off
- explicit Boundless route returns pipeline-not-available behavior when no engine is registered

### Boundless-enabled path

At minimum, ensure feature-gated packages still compile when enabled:

- `cargo test -p raiko2-prover --features boundless --no-run`
- `cargo test -p raiko2-engine --features boundless --no-run`
- `cargo test -p raiko2 --features boundless --no-run`

The heavy GitHub workflow can continue to exercise the enabled path manually.

## Risks

- Boundless types are currently intermixed with the prover implementation; splitting them may touch
  more files than the dependency graph suggests.
- Some tests may assume Boundless engines are always present and will need explicit feature gates.
- CI and docs may still mention old assumptions after the code compiles cleanly.

## Non-Goals

- Fixing `boundless-market` upstream in this change
- Restoring Boundless-heavy CI lanes as required checks
- Changing request/response schema for proof routes
