# Explicit Prover Routes Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the implicit global prover route with an explicit per-proof-type route table so one
host can independently serve any configured combination of RISC0, SP1, SGX, and SGXGETH.

**Architecture:** Add a strongly typed `ProverRoutesConfig` keyed by concrete proof type. Make
validation, request routing, pipeline registration, readiness, and startup reporting consume this
single route table. Remove legacy global route fields and make the CLI/environment override replace
the whole route table atomically.

**Tech Stack:** Rust, serde/TOML, clap, Tokio, raiko2 pipeline/engine abstractions, Docker Compose.

---

### Task 1: Define And Parse Explicit Routes

**Files:**
- Modify: `bin/raiko2/src/config/prover.rs`
- Modify: `bin/raiko2/src/config/mod.rs`
- Modify: `bin/raiko2/src/cli.rs`

**Step 1: Write failing configuration tests**

Add tests that parse:

```toml
[prover.routes]
risc0 = "network"
sp1 = "network"
sgx = "remote"
sgxgeth = "remote"
```

Assert each concrete proof type resolves to its configured runner. Add rejection tests for an empty
route table, unsupported runner combinations, duplicate CLI entries, and the removed
`guest_system`/`runner` fields. Add CLI tests proving `RAIKO2_PROVER_ROUTES`/`--prover-routes`
replace the complete table.

**Step 2: Run tests and confirm failure**

Run:

```bash
cargo test -p raiko2 config::
```

Expected: FAIL because `ProverRoutesConfig` and the new CLI option do not exist.

**Step 3: Implement the route model**

Create a serde-deny-unknown-fields struct equivalent to:

```rust
pub struct ProverRoutesConfig {
    pub risc0: Option<RunnerKind>,
    pub sp1: Option<RunnerKind>,
    pub native: Option<RunnerKind>,
    pub sgx: Option<RunnerKind>,
    pub sgxgeth: Option<RunnerKind>,
}
```

Add helpers to:

- return the runner for a `ProofType`,
- iterate enabled `(ProofType, RunnerKind)` pairs in stable order,
- test whether a proof type is enabled,
- parse a comma-separated `<proof_type>/<runner>` override, and
- validate allowed proof type/runner combinations and non-empty configuration.

Replace `ProverConfig.guest_system` and `runner` with `routes`. Remove `route()`,
`is_remote_sgx_route()`, and route normalization. Replace `Cli.prover` with `prover_routes` using
`RAIKO2_PROVER_ROUTES`; the override replaces `config.prover.routes` atomically.

**Step 4: Run focused tests**

Run:

```bash
cargo test -p raiko2 config::
```

Expected: PASS.

**Step 5: Commit**

```bash
git add bin/raiko2/src/config/prover.rs bin/raiko2/src/config/mod.rs bin/raiko2/src/cli.rs
git commit -m "refactor(config): add explicit prover routes"
```

### Task 2: Make Backend Validation Capability-Driven

**Files:**
- Modify: `bin/raiko2/src/config/prover.rs`
- Modify: `bin/raiko2/src/config/validation.rs`
- Modify: `bin/raiko2/src/server/ready.rs`

**Step 1: Write failing validation and readiness tests**

Cover these cases:

- `risc0/network` requires Boundless URL and signer credentials.
- Disabled RISC0 does not require Boundless credentials.
- `sgx/remote` requires only `remote_sgx.base_url`.
- `sgxgeth/remote` requires only `remote_sgx.sgxgeth_base_url`.
- SGX timeout is required when either lane is enabled.
- `zk_any.sp1` and `zk_any.risc0` require the matching route to be enabled.
- Readiness checks only enabled ZK capabilities.

**Step 2: Run tests and confirm failure**

Run:

```bash
cargo test -p raiko2 config:: server::ready::
```

Expected: FAIL where validation still consults the old global route.

**Step 3: Implement capability-driven checks**

Replace global route branches with `routes.runner(ProofType::...)` lookups. Preserve all existing
backend-specific safety checks, but invoke them only when their route is enabled. Reject a
`zk_any` target whose concrete route is absent.

**Step 4: Run focused tests**

Run:

```bash
cargo test -p raiko2 config:: server::ready::
```

Expected: PASS.

**Step 5: Commit**

```bash
git add bin/raiko2/src/config/prover.rs bin/raiko2/src/config/validation.rs bin/raiko2/src/server/ready.rs
git commit -m "fix(config): validate enabled prover capabilities"
```

### Task 3: Route Requests Through The Explicit Table

**Files:**
- Modify: `bin/raiko2/src/server/handlers/proof_route.rs`
- Modify: `bin/raiko2/src/server/handlers/proof_api.rs`
- Modify: `bin/raiko2/src/server/handlers/proof_api/v4.rs`
- Test: `bin/raiko2/src/server/e2e.rs`

**Step 1: Write failing route tests**

Add tests proving:

- RISC0 does not imply SP1.
- SP1 does not imply RISC0.
- a host with all four production routes resolves each concrete request correctly,
- a disabled concrete proof type returns `unsupported_proof_type`,
- proposal and aggregate requests use the same configured route, and
- request-scoped SP1 settings cannot change the configured runner to an unregistered route.

**Step 2: Run tests and confirm failure**

Run:

```bash
cargo test -p raiko2 proof_route e2e_shasta
```

Expected: FAIL because request routing still derives behavior from one global route.

**Step 3: Implement table lookup routing**

Make `route_for_proof_type` require an enabled table entry. Build the canonical `PipelineKey` from
the concrete proof type plus its configured runner. Remove `validate_hosted_proof_type`,
`default_risc0_runner`, and any fallback to the former global route. Preserve request-scoped SP1
arguments only where they are compatible with the configured SP1 runner.

**Step 4: Run focused tests**

Run:

```bash
cargo test -p raiko2 proof_route e2e_shasta
```

Expected: PASS.

**Step 5: Commit**

```bash
git add bin/raiko2/src/server/handlers/proof_route.rs bin/raiko2/src/server/handlers/proof_api.rs bin/raiko2/src/server/handlers/proof_api/v4.rs bin/raiko2/src/server/e2e.rs
git commit -m "refactor(api): route proofs by enabled backend"
```

### Task 4: Register Exactly The Enabled Pipelines

**Files:**
- Modify: `bin/raiko2/src/server/state/mod.rs`
- Modify: `bin/raiko2/src/server/state/setup.rs`

**Step 1: Write failing pipeline registration tests**

Cover:

- host-only `risc0/network` registers RISC0 only,
- host-only `sp1/network` registers SP1 only,
- host-only combined ZK and SGX config registers all selected pipelines,
- omitted SGX URL/route does not register that lane,
- local routes fail clearly when `local-provers` is not compiled, and
- local-provers builds do not register omitted local pipelines.

**Step 2: Run both feature lanes and confirm failure**

Run:

```bash
cargo test -p raiko2 --no-default-features --features host server::state::
cargo test -p raiko2 server::state::
```

Expected: FAIL because registration is still grouped around the global route.

**Step 3: Implement independent registration**

Replace the early-return/grouped registration flow with one branch per enabled concrete route.
Construct SP1 only when SP1 is enabled. Construct Boundless only when `risc0/network` is enabled.
Register SGX and SGXGETH independently using their own URLs. In a `local-provers` build, construct
only explicitly enabled local engines.

**Step 4: Run both feature lanes**

Run:

```bash
cargo test -p raiko2 --no-default-features --features host server::state::
cargo test -p raiko2 server::state::
```

Expected: PASS.

**Step 5: Commit**

```bash
git add bin/raiko2/src/server/state/mod.rs bin/raiko2/src/server/state/setup.rs
git commit -m "refactor(server): register configured prover pipelines"
```

### Task 5: Update Startup Reporting And Examples

**Files:**
- Modify: `bin/raiko2/src/server/startup.rs`
- Modify: `config.example.toml`
- Modify: `docs/API.md`
- Modify: `docs/operations.md`
- Modify: `docker/.env.sample`
- Modify: `docker/.env.sgx.regression.sample`
- Modify: `docker/docker-compose.yml`
- Modify: `docker/docker-compose.sgx.regression.yml`

**Step 1: Write failing startup summary tests**

Assert the summary contains a stable list of enabled routes and only exposes sanitized SGX URLs
for enabled lanes. Assert legacy `route` output is absent.

**Step 2: Run tests and confirm failure**

Run:

```bash
cargo test -p raiko2 server::startup::
```

Expected: FAIL because the summary still reports one route.

**Step 3: Implement reporting and migration documentation**

Change startup reporting from `route` to `routes`. Replace every active example of
`guest_system`/`runner` and `RAIKO2_PROVER` with the explicit route table or
`RAIKO2_PROVER_ROUTES`. Put the combined production sample in `config.example.toml` while retaining
placeholder-only credentials and portable service names.

**Step 4: Verify tests and stale references**

Run:

```bash
cargo test -p raiko2 server::startup:: config::tests::test_config_example_validates
rg -n "RAIKO2_PROVER=|guest_system =|runner =" config.example.toml README.md docs docker
```

Expected: tests PASS; grep finds only historical design documents where explicitly appropriate.

**Step 5: Commit**

```bash
git add bin/raiko2/src/server/startup.rs config.example.toml docs/API.md docs/operations.md docker
git commit -m "docs(config): document explicit prover routes"
```

### Task 6: Full Verification

**Files:**
- Modify only files required by verification fixes.

**Step 1: Format**

Run:

```bash
cargo fmt --all
```

**Step 2: Run host-only checks**

Run:

```bash
cargo test -p raiko2 --no-default-features --features host
cargo clippy -p raiko2 --no-default-features --features host -- -D warnings
```

Expected: PASS.

**Step 3: Run default-feature checks**

Run:

```bash
cargo test -p raiko2
cargo clippy -p raiko2 -- -D warnings
```

Expected: PASS.

**Step 4: Verify repository hygiene**

Run:

```bash
git diff --check origin/main...HEAD
git status --short
```

Expected: no whitespace errors and only intentional changes.

**Step 5: Commit verification fixes if needed**

```bash
git add <verified-files>
git commit -m "fix(config): finish explicit prover route migration"
```
