# Proof Identity Compatibility Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Gate proof reuse on the active local guest or remote SGX identity.

**Architecture:** Keep local ZK identities immutable and derived from loaded
artifacts. Keep remote SGX identity as an optional configured or process-learned
pair. Separate validation of new backend inputs from compatibility checks for
persisted artifacts.

**Tech Stack:** Rust, Tokio synchronization, existing runtime artifact store,
RISC0, SP1, and remote SGX proof headers.

---

### Task 1: Specify Pure Identity Behavior

**Files:**
- Create: `bin/raiko2/src/server/proof_identity.rs`
- Modify: `bin/raiko2/src/server/mod.rs`
- Test: `bin/raiko2/src/server/proof_identity.rs`

1. Add failing unit tests for a configured SGX mismatch, unknown-lane cache
   miss, first successful identity learning, immutable learned identity, and
   static RISC0/SP1 identity comparisons.
2. Run the focused test and confirm the missing behavior fails.
3. Implement the smallest identity parser and state holder to pass the tests.
4. Re-run the focused test.

### Task 2: Derive Active Expected Identities

**Files:**
- Modify: `bin/raiko2/src/config/prover.rs`
- Modify: `bin/raiko2/src/server/state/mod.rs`
- Modify: `config.example.toml`
- Test: touched unit tests

1. Add the optional complete remote SGX pair to each SGX lane configuration.
2. Derive ZK expected identities from the active backend ELF or verifying key
   material at startup.
3. Construct one identity registry per active pipeline without writing a
   durable expected record.
4. Run focused configuration and state tests.

### Task 3: Gate Reuse and Learn After Activation

**Files:**
- Modify: `bin/raiko2/src/server/handlers/proof_api.rs`
- Modify: `bin/raiko2/src/server/state/runtime_observer.rs`
- Test: corresponding focused tests

1. Add failing regressions for stale cache suppression, no leaked `proof_uri`,
   and no learning before root activation.
2. Apply compatibility only where completed artifacts are read or delivered.
3. Serialize unknown remote SGX finalization through root activation, then
   learn the winning pair without allowing mismatches to mutate state.
4. Treat a configured/learned mismatch as a non-retryable returned-proof
   error, rather than a durable publication retry.
5. Re-run focused tests.

### Task 4: Preserve Backend-Specific Aggregate Contracts

**Files:**
- Modify: `bin/raiko2/src/server/handlers/proof_api.rs`
- Test: corresponding focused tests

1. Add a failing Boundless receipt-only aggregate-input regression.
2. Add a failing mixed-instance remote SGX aggregate-input regression.
3. Preserve receipt-based RISC0 validation and reject only mixed SGX pairs in
   one aggregate request.
4. Re-run focused tests.

### Task 5: Verify and Document

Run `cargo fmt --all`, focused package tests, `cargo clippy -p raiko2 -- -D
warnings`, and the relevant no-default-features check. Update API and example
configuration documentation only for the new optional SGX expected pair.
