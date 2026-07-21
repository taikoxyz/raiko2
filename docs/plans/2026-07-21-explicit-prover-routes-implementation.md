# Self-Contained Prover Configuration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the disconnected prover route and backend tables with self-contained per-proof-type
configuration while preserving all Boundless behavior and independent pipeline routing.

**Architecture:** `ProverConfig` owns one table per concrete proof type and derives enabled
`PipelineRoute` values from those tables. RISC0 nests the full Boundless global configuration;
pair-specific Boundless overrides remain scoped to RPC pairs and are applied after the global
configuration. All server consumers use `ProverConfig` helpers instead of reading a route table.

**Tech Stack:** Rust, serde/TOML, clap, Tokio, raiko2 pipeline/runtime abstractions, Docker Compose.

---

### Task 1: Define The Self-Contained Configuration Contract

**Files:**
- Modify: `bin/raiko2/src/config/prover.rs`
- Modify: `bin/raiko2/src/config/mod.rs`
- Modify: `bin/raiko2/src/cli.rs`

**Step 1: Write failing tests**

Add parsing and validation tests for `[prover.risc0]`, `[prover.sp1]`, `[prover.native]`,
`[prover.sgx]`, and `[prover.sgxgeth]`. Assert that `enabled` and each backend's native selector
produce the expected route. Assert that no enabled proof type is rejected and that the superseded
`[prover.routes]`, `[prover.boundless]`, and `[prover.remote_sgx]` tables fail parsing.

**Step 2: Verify red**

Run `cargo test -p raiko2 config::` and confirm the new TOML fails under the old schema.

**Step 3: Implement minimal structs and route helpers**

Enable only native local execution by default. Add `ProverConfig::runner`, `is_enabled`,
`iter_routes`, and the atomic route-override application. An explicit route override disables all
omitted proof types, including native. Derive SP1 runner from `Sp1Config.prover`; use fixed runners
for native and both SGX lanes. Keep serde `deny_unknown_fields` at every configuration boundary.

**Step 4: Verify green and commit**

Run `cargo test -p raiko2 config::`, then commit with
`refactor(config): colocate prover routes and backends`.

### Task 2: Move Boundless Under RISC0 Without Losing Fields

**Files:**
- Modify: `bin/raiko2/src/config/prover.rs`
- Modify: `bin/raiko2/src/config/mod.rs`
- Modify: `bin/raiko2/src/config/rpc.rs`
- Modify: `bin/raiko2/src/server/state/setup.rs`
- Modify: `bin/raiko2/src/server/ready.rs`

**Step 1: Write failing Boundless round-trip tests**

Parse a complete `[prover.risc0.boundless]` fixture containing credentials, deployment and
overrides, both quote strategies, both offer parameter blocks and timeout blocks, polling, global
timeout, and every rebid field. Assert every value survives deserialization. Add a test proving a
pair override replaces only its optional fields and preserves all other global values.

**Step 2: Verify red**

Run the focused config and setup tests and confirm the nested path is not accepted yet.

**Step 3: Move and validate the configuration**

Nest `BoundlessConfig` in RISC0 configuration. Parameterize validation error prefixes so errors name
`prover.risc0.boundless`. Preserve the effective merge order and all existing offer safety checks.
Validate pair overrides only when RISC0 network is enabled.

**Step 4: Verify green and commit**

Run `cargo test -p raiko2 config:: server::state::setup:: server::ready::` and commit with
`refactor(config): nest boundless under risc0`.

### Task 3: Migrate Runtime Consumers

**Files:**
- Modify: `bin/raiko2/src/server/handlers/proof_route.rs`
- Modify: `bin/raiko2/src/server/handlers/proof_api.rs`
- Modify: `bin/raiko2/src/server/state/mod.rs`
- Modify: `bin/raiko2/src/server/state/setup.rs`
- Modify: `bin/raiko2/src/server/startup.rs`
- Modify: `bin/raiko2/src/server/ready.rs`
- Modify: `bin/raiko2/src/server/e2e.rs`
- Modify: `bin/raiko2/src/server/fixture.rs`

**Step 1: Write failing routing/registration tests**

Update tests to configure proof types through their own sections. Cover all production routes on one
host, disabled request rejection, SP1 local/mock/network mapping, independent SGX URLs and timeouts,
and resource preparation for enabled backends only.

**Step 2: Verify red**

Run focused proof-route, state, readiness, and startup tests.

**Step 3: Replace route-table reads**

Use `ProverConfig` route helpers everywhere. Access Boundless through RISC0 and SGX settings through
their own lane. Preserve the existing persisted route compatibility behavior unchanged.

**Step 4: Verify green and commit**

Run `cargo test -p raiko2 --no-default-features` and
`cargo test -p raiko2 --no-default-features --features host`, then commit with
`refactor(server): consume self-contained prover config`.

### Task 4: Update Samples And Documentation

**Files:**
- Modify: `config.example.toml`
- Modify: `docker/config.compose.toml`
- Modify: `docker/.env.sample`
- Modify: `docker/.env.sgx.regression.sample`
- Modify: `docker/docker-compose.yml`
- Modify: `docker/docker-compose.sgx.regression.yml`
- Modify: `README.md`
- Modify: `docs/API.md`
- Modify: `docs/operations.md`
- Modify: `docs/development.md`
- Modify: `docs/hoodi-txlist-witness-rollout.md`

**Step 1: Update the canonical sample first**

Move every Boundless key and child table under `prover.risc0.boundless`, split SGX and SGXGETH
settings, and remove `[prover.routes]`. Keep placeholders generic and secret-free.

**Step 2: Update all references and examples**

Make the API and operations docs explain self-contained enablement and the retained atomic route
override. Update Docker examples to use only the new config model.

**Step 3: Verify and commit**

Run config example tests plus repository searches for superseded paths. Commit with
`docs(config): document self-contained prover setup`.

### Task 5: Remove The Obsolete Bonsai Selector

**Files:**
- Modify: `bin/raiko2/src/config/prover.rs`
- Modify: `bin/raiko2/src/server/state/setup.rs`
- Modify: `crates/prover/src/risc0/types.rs`
- Modify: `crates/prover/src/risc0/mod.rs`
- Modify: `crates/prover/examples/risc0_real_prove.rs`
- Modify: `config.example.toml`
- Modify: `docker/config.compose.toml`

**Step 1: Write the failing configuration test**

Add a strict TOML parsing test that includes `bonsai = true` under `[prover.risc0]` and expects an
unknown-field error. Keep the existing local RISC0 fixture free of that key.

**Step 2: Run the test to verify it fails**

Run `cargo test -p raiko2 config::prover::tests::rejects_removed_risc0_bonsai_setting` and confirm
the configuration still accepts the key.

**Step 3: Remove the selector and runtime branch**

Delete `bonsai` from both RISC0 configuration structs and their defaults. Remove the
`default_prover()` import and branch so `Risc0Prover` always uses `get_prover_server()` for real
local proofs. Keep `snark`, `mock`, and the workspace `risc0-zkvm` dependency features unchanged.

**Step 4: Update examples and documentation**

Delete `bonsai` from the canonical config, Docker config, prover example, and all current design or
operator examples. Historical references unrelated to live configuration need not be rewritten.

**Step 5: Run focused verification and commit**

Run `cargo test -p raiko2 config::`, `cargo test -p raiko2-prover risc0`, and
`cargo fmt --all -- --check`. Commit with `refactor(risc0): remove obsolete bonsai selector`.

### Task 6: Full Verification And Review

**Files:**
- Review all modified files.

**Step 1: Run formatting and static checks**

Run `cargo fmt --all -- --check`, `git diff --check origin/main...HEAD`, and
`cargo clippy --workspace -- -D warnings`.

**Step 2: Run feature and workspace test lanes**

Run all raiko2 default/no-default/host tests and the targeted primitives, protocol, provider,
pipeline, preflight, queue, and runtime lanes from `AGENTS.md`.

**Step 3: Review configuration coverage**

Search for old config paths, verify every Boundless field appears at the new path, and check changed
files for hardcoded machine paths, personal names, and credentials.

**Step 4: Request independent review and update PR**

Resolve all blocking or important findings, push the branch, and update PR #185 with the final
configuration and verification results.
