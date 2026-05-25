# Raiko2 SGX Runtime Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a dedicated `sgx` runtime binary to `raiko2` with `bootstrap`, `check`, and `serve`
modes, plus the Docker and compose assets needed to operate it. The runtime serves the same remote
API in `tee` and `native` modes, while keeping the main `raiko2` service unchanged.

**Architecture:** The main `raiko2` service remains the orchestrator and remote client. A new
`raiko2-sgx-prover` binary hosts the SGX runtime and HTTP server, reusing shared `raiko2`
Shasta/proof types. Proposal serving is made protocol-compatible with the current `sgx/remote`
request contract, while aggregation serving is implemented server-side and documented as waiting on
separate main-service wiring. Historical compatibility still expects `proof_type=sgx | sgxgeth`,
but this plan implements only the `sgx` lane; `sgxgeth` remains an external `gaiko2` service.

**Tech Stack:** Rust workspace binaries/crates, clap, axum, serde, Docker, Docker Compose,
Gramine/Intel SGX runtime packages.

---

### Task 1: Scaffold the new SGX runtime workspace pieces

**Files:**
- Create: `bin/raiko2-sgx-prover/Cargo.toml`
- Create: `bin/raiko2-sgx-prover/src/main.rs`
- Create: `crates/sgx-runtime/Cargo.toml`
- Create: `crates/sgx-runtime/src/lib.rs`
- Modify: `Cargo.toml`

**Step 1: Add the new workspace members**

- Register the binary crate and runtime crate in the workspace.
- Keep names explicit: binary `raiko2-sgx-prover`, library `raiko2-sgx-runtime`.

**Step 2: Add a minimal CLI skeleton**

- Add `serve`, `bootstrap`, and `check` subcommands.
- Wire the binary to call stubbed library functions.

**Step 3: Verify the new binary builds**

Run: `cargo check -p raiko2-sgx-prover`
Expected: PASS with stubbed handlers.

### Task 2: Define SGX runtime config and bootstrap artifacts

**Files:**
- Create: `crates/sgx-runtime/src/config.rs`
- Create: `crates/sgx-runtime/src/bootstrap.rs`
- Create: `crates/sgx-runtime/src/check.rs`
- Modify: `crates/sgx-runtime/src/lib.rs`

**Step 1: Model runtime configuration**

- Add config structs for:
  - runtime mode (`tee | native`)
  - config dir
  - secrets dir
  - bootstrap output path
  - bind address/port
  - SGX instance ids for Shasta
- Keep the SGX path Gramine-only; do not add `ego`/`dev` branches.

**Step 2: Define bootstrap artifact schema**

- Follow the old `raiko` operator flow closely enough that bootstrap outputs are recognizable and
  usable by existing registration practices.
- Persist bootstrap JSON in a stable operator-managed path.
- Keep the artifact scoped to the `sgx` lane only; do not add `sgxgeth` fields here.
- In `native` mode, make `bootstrap` a no-op success and do not persist SGX artifacts.

**Step 3: Add `check` validation**

- Validate bootstrap file readability, secret presence, and basic runtime prerequisites.
- In `native` mode, allow `check` to succeed without SGX lifecycle files.

**Step 4: Verify unit coverage**

Run: `cargo test -p raiko2-sgx-runtime`
Expected: PASS for config/bootstrap/check parsing tests.

### Task 3: Implement proposal protocol compatibility

**Files:**
- Create: `crates/sgx-runtime/src/protocol.rs`
- Create: `crates/sgx-runtime/src/proposal.rs`
- Modify: `crates/sgx-runtime/src/lib.rs`

**Step 1: Reuse the existing proposal request/response types**

- Reuse `raiko2_prover::gaiko2::protocol::{Gaiko2ShastaRequest, Gaiko2ProofResponse}` for
  proposal serving.
- Do not modify the existing `gaiko2` client code.
- Do not expand this task to handle `sgxgeth`; that remains external.

**Step 2: Implement proposal proof execution**

- Decode `Gaiko2ShastaRequest`.
- Reconstruct the runtime input needed by the SGX prover.
- Produce a proof envelope containing proof bytes, quote, instance address, and input hash.
- In `native` mode, omit the quote while preserving the rest of the response schema.

**Step 3: Add compatibility tests**

- Test decode/encode compatibility against the existing protocol structs.

**Step 4: Verify**

Run: `cargo test -p raiko2-sgx-runtime proposal`
Expected: PASS

### Task 4: Implement Shasta aggregation serving on the SGX side

**Files:**
- Create: `crates/sgx-runtime/src/aggregation.rs`
- Modify: `crates/sgx-runtime/src/protocol.rs`
- Modify: `crates/sgx-runtime/src/lib.rs`

**Step 1: Define the SGX-side aggregation request/response contract**

- Keep this contract local to the SGX runtime for now.
- Do not modify the main `raiko2` remote client in this task.
- Keep the contract scoped to the `sgx` runtime lane only.

**Step 2: Implement aggregation proof execution**

- Accept Shasta aggregation inputs.
- Produce the SGX aggregation proof envelope needed for later on-chain verification.

**Step 3: Add tests**

- Verify aggregation request parsing and response encoding.

**Step 4: Verify**

Run: `cargo test -p raiko2-sgx-runtime aggregation`
Expected: PASS

### Task 5: Add the HTTP server

**Files:**
- Create: `crates/sgx-runtime/src/server.rs`
- Modify: `bin/raiko2-sgx-prover/src/main.rs`
- Modify: `crates/sgx-runtime/src/lib.rs`

**Step 1: Add the Shasta-only routes**

- `POST /prove/shasta`
- `POST /prove/shasta-aggregate`
- `GET /health`

**Step 2: Keep lifecycle commands out of HTTP**

- Do not add `/bootstrap`.
- Do not add old block/batch routes.

**Step 3: Add server tests**

- Test request handling for success and validation failures.

**Step 4: Verify**

Run: `cargo test -p raiko2-sgx-prover`
Expected: PASS

### Task 6: Add Gramine image build assets

**Files:**
- Create: `Dockerfile.sgx`
- Create: `docker/sgx-entrypoint.sh`
- Create: `docker/sgx-bootstrap.sh`
- Modify: `.gitignore` if needed for SGX output directories

**Step 1: Build the SGX runtime image**

- Base on a Gramine runtime image.
- Install Intel SGX/DCAP packages needed by the runtime.
- Copy the new SGX binary and required manifests/configs.

**Step 2: Add entrypoint helpers**

- Support choosing `bootstrap`, `check`, or `serve` from compose/scripts.
- Support choosing `tee` or `native` mode from compose/scripts.
- Default command should be `serve`.

**Step 3: Verify**

Run: `docker build -f Dockerfile.sgx -t raiko2-sgx:local .`
Expected: PASS

### Task 7: Add compose and env templates

**Files:**
- Create: `docker/docker-compose.sgx.yml`
- Create: `docker/.env.sgx.sample`
- Modify: `docs/operations.md`
- Optionally create: `docs/operations-sgx.md`

**Step 1: Add SGX compose orchestration**

- Mount SGX devices.
- Mount PCCS/QCNL config.
- Mount config/secrets/bootstrap output directories.
- Provide a one-shot bootstrap service and a long-running serve service, or a single service with a
  mode switch if that proves simpler.
- Document that this compose file is for `sgx` only and does not launch the external `sgxgeth`
  service.

**Step 2: Add operator env template**

- Document all required env vars and host mount expectations.

**Step 3: Update operator docs**

- Document bootstrap, check, serve, and compose flows.
- Explicitly note that proposal serving is directly compatible now, while main-service aggregation
  wiring is a separate dependency.
- Explicitly note that `sgxgeth` is served by external `gaiko2` infrastructure and is not built by
  this plan.

**Step 4: Verify**

Run: `docker compose --env-file docker/.env.sgx.sample -f docker/docker-compose.sgx.yml config`
Expected: PASS

### Task 8: Final verification and handoff

**Files:**
- Modify: none

**Step 1: Format**

Run: `cargo fmt --all`
Expected: PASS

**Step 2: Run focused Rust verification**

Run:
- `cargo check -p raiko2-sgx-prover`
- `cargo test -p raiko2-sgx-runtime`
- `cargo test -p raiko2-sgx-prover`

Expected: PASS

**Step 3: Run Docker verification**

Run:
- `docker build -f Dockerfile.sgx -t raiko2-sgx:local .`
- `docker compose --env-file docker/.env.sgx.sample -f docker/docker-compose.sgx.yml config`

Expected: PASS
