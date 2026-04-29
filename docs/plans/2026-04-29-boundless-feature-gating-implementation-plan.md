# Boundless Feature Gating Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make Boundless support optional and default-off so normal `raiko2`, `raiko2-engine`, and `raiko2-prover` builds/tests do not compile `boundless-market`.

**Architecture:** Split Boundless into an always-on data/config layer and a feature-gated prover implementation layer. Keep canonical route/pipeline enums intact, but only register the Boundless engine when the `boundless` cargo feature is enabled. When the feature is disabled, default RISC0 route selection falls back to local and explicit Boundless requests fail through the existing “pipeline not available” path.

**Tech Stack:** Rust workspace features, Cargo optional dependencies, Axum server wiring, existing runtime/task metadata flow, targeted cargo test/clippy verification.

---

### Task 1: Revert the temporary Boundless dependency experiment

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `.github/workflows/rust-heavy.yml`

**Step 1: Restore the workspace dependency declaration**

Change `boundless-market` in `Cargo.toml` back to the crates.io pin:

```toml
boundless-market = { version = "=1.0.0" }
```

**Step 2: Restore the lockfile**

Run:

```bash
cargo update -p boundless-market --precise 1.0.0
```

Expected: `Cargo.lock` returns to the crates.io package instead of the git revision.

**Step 3: Drop the uncommitted rust-heavy diagnostic tweak if it is no longer needed**

Keep `rust-heavy.yml` aligned with the last committed workflow unless the new feature work still
needs the extra diagnostics.

**Step 4: Verify the worktree only contains intended changes**

Run:

```bash
git status --short
```

Expected: only planned feature-gating edits remain.

### Task 2: Add the `boundless` cargo feature to the prover crate

**Files:**
- Modify: `crates/prover/Cargo.toml`

**Step 1: Write the failing compile target expectation**

Target condition: default `raiko2-prover` should stop pulling `boundless-market`.

**Step 2: Make `boundless-market` optional**

Change the dependency to optional and add:

```toml
[features]
default = []
boundless = ["dep:boundless-market"]
```

Add any other required feature propagation entries here.

**Step 3: Verify the dependency graph changes**

Run:

```bash
cargo tree -p raiko2-prover
```

Expected: default tree no longer shows `boundless-market`.

### Task 3: Split always-on Boundless types from the feature-gated prover implementation

**Files:**
- Modify: `crates/prover/src/lib.rs`
- Modify: `crates/prover/src/boundless/mod.rs`
- Create or Modify: additional files under `crates/prover/src/boundless/` as needed
- Test: `crates/prover/tests/aggregation_adapter_risc0.rs`

**Step 1: Write/adjust a failing test or compile target**

Use the existing aggregation adapter test plus a default-feature crate test target to prove the
always-on pieces still compile without the prover implementation.

**Step 2: Move always-on types/helpers out of the feature-gated implementation**

Keep available without feature:

- `BoundlessConfig`
- `BoundlessOfferParams`
- `OfferParamsConfig`
- `DeploymentConfig`
- `BatchQuoteStrategy`
- `validate_offer_spec`
- aggregation adapter code

Put behind `#[cfg(feature = "boundless")]`:

- `BoundlessProver`
- external client logic using `boundless-market`

**Step 3: Add feature-gated exports in `lib.rs`**

Export boundless types unconditionally and the prover implementation conditionally.

**Step 4: Run focused prover tests**

Run:

```bash
cargo test -p raiko2-prover
```

Expected: PASS on default features without compiling `boundless-market`.

### Task 4: Propagate the feature through engine and gate Boundless-only tests

**Files:**
- Modify: `crates/engine/Cargo.toml`
- Modify: `crates/engine/src/lib.rs`

**Step 1: Add feature propagation**

Add:

```toml
[features]
default = []
boundless = ["raiko2-prover/boundless"]
```

**Step 2: Gate Boundless-only tests**

Wrap tests that instantiate `PipelineKey::ShastaRisc0Boundless` or depend on the Boundless prover
implementation in `#[cfg(feature = "boundless")]`.

**Step 3: Run engine tests**

Run:

```bash
cargo test -p raiko2-engine
```

Expected: PASS on default features.

### Task 5: Propagate the feature through `bin/raiko2` and stop registering Boundless by default

**Files:**
- Modify: `bin/raiko2/Cargo.toml`
- Modify: `bin/raiko2/src/server/state/mod.rs`
- Modify: `bin/raiko2/src/server/handlers/proof_route.rs`
- Modify: `bin/raiko2/src/config/prover.rs`
- Modify: `bin/raiko2/src/config/rpc.rs`
- Modify: any nearby setup files required by the compiler

**Step 1: Add feature propagation**

Add:

```toml
[features]
default = []
boundless = ["raiko2-engine/boundless", "raiko2-prover/boundless"]
redis-queue = ["raiko2-queue/redis"]
```

**Step 2: Gate Boundless engine wiring**

Compile Boundless engine type aliases, builders, and registration blocks only when
`feature = "boundless"`.

**Step 3: Implement feature-off route fallback**

Adjust `default_risc0_runner` so the default RISC0 route becomes `local` when the feature is off,
even if config says `runner = boundless`.

**Step 4: Preserve config parsing**

Do not reject configs merely because Boundless fields exist while the feature is off.

**Step 5: Verify compile/tests**

Run:

```bash
cargo test -p raiko2 test_server_config -- --nocapture
```

Expected: PASS and no Boundless compile failure.

### Task 6: Add explicit behavior tests for feature-off operation

**Files:**
- Modify: `bin/raiko2/src/server/handlers/proof_route.rs`
- Modify: `bin/raiko2/src/server/e2e.rs`
- Modify: `bin/raiko2/src/config/mod.rs`

**Step 1: Write the failing tests**

Add tests covering:

- default RISC0 route uses `local` when Boundless feature is off
- explicit `risc0/boundless` request fails through `pipeline not available`
- config validation does not reject Boundless fields solely because the feature is off

**Step 2: Run the tests to verify failure**

Run the narrowest test commands for the new tests.

Expected: FAIL until the implementation is complete.

**Step 3: Implement the minimal code to pass**

Use existing API behavior where possible rather than inventing new error layers.

**Step 4: Re-run the focused tests**

Expected: PASS.

### Task 7: Verify the default build and the feature-enabled compile path

**Files:**
- No new files expected

**Step 1: Run formatting**

Run:

```bash
cargo fmt --all
```

Expected: PASS.

**Step 2: Run default verification**

Run:

```bash
cargo test -p raiko2-prover
cargo test -p raiko2-engine
cargo test -p raiko2
cargo clippy -p raiko2 -- -D warnings
```

Expected: PASS without compiling `boundless-market`.

**Step 3: Run feature-enabled compile probes**

Run:

```bash
cargo test -p raiko2-prover --features boundless --no-run
cargo test -p raiko2-engine --features boundless --no-run
cargo test -p raiko2 --features boundless --no-run
```

Expected: compile reaches the Boundless-enabled path again for manual/heavy validation.

### Task 8: Update CI and docs only if needed

**Files:**
- Modify: `.github/workflows/*.yml` if the feature split changes command assumptions
- Modify: `README.md`, `docs/API.md`, or `config.example.toml` only if user-facing behavior changed

**Step 1: Check whether current CI commands assume Boundless is always on**

If the existing commands still make sense with default-off behavior, do not add extra churn.

**Step 2: Update docs minimally**

Document the `boundless` feature only where operators or developers need it.

**Step 3: Re-run any affected checks**

Run the smallest relevant command set for any CI/doc updates.

### Task 9: Commit the work in logical chunks

**Files:**
- All modified files from tasks 1-8

**Step 1: Commit the dependency/feature split**

```bash
git add Cargo.toml Cargo.lock crates/prover/Cargo.toml crates/prover/src/lib.rs crates/prover/src/boundless
git commit -m "feat: gate boundless prover integration"
```

**Step 2: Commit engine/server wiring**

```bash
git add crates/engine bin/raiko2
git commit -m "feat: disable boundless routes by default"
```

**Step 3: Commit any follow-up docs/CI cleanup**

```bash
git add .github/workflows docs README.md docs/API.md config.example.toml
git commit -m "docs: document boundless feature gating"
```
