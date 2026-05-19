# Raiko2 SGX Provider Logging Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add minimal startup and lightweight success/failure logs to the dedicated `raiko2-sgx` provider without changing proof behavior.

**Architecture:** Initialize tracing in the SGX provider binary, add a small startup summary helper in `raiko2-sgx-runtime`, and emit explicit startup and request logs from the server layer. Keep the logged surface safe and avoid introducing metrics or schema changes.

**Tech Stack:** Rust, `tracing`, `tracing-subscriber`, `axum`, existing `raiko2-sgx-runtime` request/response types

---

### Task 1: Add failing startup summary tests

**Files:**
- Modify: `crates/sgx-runtime/src/server.rs`
- Test: `crates/sgx-runtime/src/server.rs`

**Step 1: Write the failing test**

Add focused tests for a new startup summary helper that assert:

- mode, listen, fork, and instance id are captured
- config and secret directory paths are present
- no secret payload fields are emitted

**Step 2: Run test to verify it fails**

Run: `cargo test -p raiko2-sgx-runtime startup_summary -- --nocapture`
Expected: FAIL because the helper does not exist yet.

**Step 3: Write minimal implementation**

Add a small serializable startup summary type plus builder in the runtime crate.

**Step 4: Run test to verify it passes**

Run: `cargo test -p raiko2-sgx-runtime startup_summary -- --nocapture`
Expected: PASS

### Task 2: Initialize tracing in the SGX provider binary

**Files:**
- Modify: `bin/raiko2-sgx-prover/Cargo.toml`
- Modify: `bin/raiko2-sgx-prover/src/main.rs`

**Step 1: Write the failing test**

Use the startup summary tests from Task 1 as the red step for this behavior boundary.

**Step 2: Write minimal implementation**

Add `tracing-subscriber` to the binary and initialize plain-text logging with `with_ansi(false)`,
using `RUST_LOG` / default `info`.

**Step 3: Run targeted verification**

Run: `cargo test -p raiko2-sgx-runtime startup_summary -- --nocapture`
Expected: PASS

### Task 3: Add startup and listening logs

**Files:**
- Modify: `crates/sgx-runtime/src/lib.rs`
- Modify: `crates/sgx-runtime/src/server.rs`

**Step 1: Write the failing test**

Extend the startup summary tests if needed to pin the helper shape before the log call sites use it.

**Step 2: Write minimal implementation**

Emit:

- `starting raiko2 sgx provider`
- `raiko2 sgx provider listening`

The first log should carry the startup summary fields. The second log should confirm the bound
address, fork, and instance id.

**Step 3: Run targeted verification**

Run: `cargo test -p raiko2-sgx-runtime startup_summary -- --nocapture`
Expected: PASS

### Task 4: Add request success/failure logs

**Files:**
- Modify: `crates/sgx-runtime/src/server.rs`
- Reference: `crates/sgx-runtime/src/proposal.rs`
- Reference: `crates/sgx-runtime/src/aggregation.rs`
- Reference: `crates/sgx-runtime/src/protocol.rs`

**Step 1: Write the failing test**

Keep this task TDD-light by first pinning the startup helper and reusing existing request tests as
behavior guards, since handler logging itself is side-effect-only.

**Step 2: Write minimal implementation**

For proposal requests, log success/failure with:

- `schema`
- `chain_id`
- `block_count`
- `instance_id`

For aggregate requests, log success/failure with:

- `schema`
- `proof_count`
- `instance_id`

Use:

- `info!` for successful requests
- `warn!` for bad JSON / invalid request / prover failures

**Step 3: Run targeted verification**

Run: `cargo test -p raiko2-sgx-runtime --lib -- --nocapture`
Expected: PASS

### Task 5: Final verification

**Files:**
- No code changes

**Step 1: Run crate verification**

Run: `cargo test -p raiko2-sgx-runtime --lib -- --nocapture`
Expected: PASS

**Step 2: Run binary verification**

Run: `cargo test -p raiko2-sgx-prover -- --nocapture`
Expected: PASS

**Step 3: Run formatting and diff checks**

Run: `cargo fmt --all --check`
Expected: PASS

Run: `git diff --check`
Expected: PASS

### Task 6: Commit

**Step 1: Commit the logging change**

```bash
git add \
  bin/raiko2-sgx-prover/Cargo.toml \
  bin/raiko2-sgx-prover/src/main.rs \
  crates/sgx-runtime/src/lib.rs \
  crates/sgx-runtime/src/server.rs \
  docs/plans/2026-05-19-raiko2-sgx-provider-logging-design.md \
  docs/plans/2026-05-19-raiko2-sgx-provider-logging-implementation-plan.md
git commit -m "fix: add raiko2 sgx provider logs"
```
