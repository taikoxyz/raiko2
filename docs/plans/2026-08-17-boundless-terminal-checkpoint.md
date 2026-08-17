# Boundless Terminal Checkpoint Reset Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make a Boundless proof task that exhausts its no-lock rebid budget discard only its matching provider submission checkpoint so the next client retry starts a new provider request at attempt one.

**Architecture:** Add a compare-and-clear checkpoint operation to the prover observer boundary and forward it through the engine to the runtime observer. The runtime observer will atomically verify the Boundless backend, provider request ID, and attempt before clearing the stage-local remote submission fields. The Boundless prover will invoke this operation only for the final no-lock-abort outcome; all other fatal outcomes retain their checkpoint for recovery.

**Tech Stack:** Rust, Tokio, async-trait, raiko2 prover/engine/runtime state, Cargo tests and Clippy.

---

### Task 1: Define Terminal Checkpoint Identity And Observer Contract

**Files:**
- Modify: `crates/prover/src/lib.rs`
- Test: `crates/prover/src/lib.rs`

**Step 1: Write the failing tests**

Add tests for a helper that clears a checkpoint through a mock observer:

```rust
#[tokio::test]
async fn clear_pending_checkpoint_forwards_exact_identity() {
    let identity = PendingProofCheckpointIdentity {
        backend: NetworkProverBackend::Boundless,
        provider_request_id: "request-1".to_string(),
        attempt: NonZeroU32::new(5).unwrap(),
    };
    clear_pending_proof_checkpoint(Some(&observer), &identity, &permit)
        .await
        .unwrap();
    assert_eq!(observer.cleared(), vec![identity]);
}
```

Also assert that no observer is a no-op and that permanent observer failures are returned.

**Step 2: Run the tests to verify they fail**

Run: `cargo test -p raiko2-prover --features boundless clear_pending_checkpoint`

Expected: FAIL because the identity type and clear operation do not exist.

**Step 3: Implement the minimal contract**

Add:

```rust
pub struct PendingProofCheckpointIdentity {
    pub backend: NetworkProverBackend,
    pub provider_request_id: String,
    pub attempt: NonZeroU32,
}
```

Extend `ProverProgressObserver` with `clear_pending_proof_checkpoint`, and add a helper that uses the existing submission checkpoint permit and persistence retry policy. The default observer implementation must return a permanent unsupported error rather than silently succeeding.

**Step 4: Run the tests to verify they pass**

Run: `cargo test -p raiko2-prover --features boundless clear_pending_checkpoint`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/prover/src/lib.rs
git commit -m "feat(prover): add terminal checkpoint reset contract"
```

### Task 2: Add Compare-And-Clear Runtime Metadata Semantics

**Files:**
- Modify: `bin/raiko2/src/server/task_metadata.rs`
- Test: `bin/raiko2/src/server/task_metadata.rs`

**Step 1: Write the failing tests**

Add tests proving that:

1. A matching Boundless request ID and attempt clears every remote submission field.
2. A different request ID leaves metadata unchanged and returns an error.
3. A different attempt leaves metadata unchanged and returns an error.
4. An SP1 checkpoint cannot be cleared through the Boundless operation.

**Step 2: Run the tests to verify they fail**

Run: `cargo test -p raiko2 task_metadata::tests::clear_boundless`

Expected: FAIL because the metadata clear method does not exist.

**Step 3: Implement the minimal metadata method**

Add `TaskRuntimeMetadata::clear_remote_submission` that validates the stored backend and exact identity before replacing only the runtime submission metadata with a default value carrying the new `updated_at` timestamp.

**Step 4: Run the tests to verify they pass**

Run: `cargo test -p raiko2 task_metadata::tests::clear_boundless`

Expected: PASS.

**Step 5: Commit**

```bash
git add bin/raiko2/src/server/task_metadata.rs
git commit -m "feat(runtime): compare and clear provider checkpoints"
```

### Task 3: Wire Checkpoint Reset Through Engine And Runtime Observer

**Files:**
- Modify: `crates/engine/src/lib.rs`
- Modify: `bin/raiko2/src/server/state/runtime_observer.rs`
- Test: `bin/raiko2/src/server/state/runtime_observer.rs`
- Test: `bin/raiko2/src/server/state/runtime_observer_state_tests.rs`

**Step 1: Write the failing tests**

Add proposal and aggregate tests that persist a Boundless checkpoint, clear it with the exact identity, and assert `load_pending_proof_checkpoint` returns `None`. Add a mismatch test that asserts the checkpoint remains loadable.

**Step 2: Run the tests to verify they fail**

Run: `cargo test -p raiko2 clear_pending_proof_checkpoint`

Expected: FAIL because the engine observer has no reset operation.

**Step 3: Implement the engine forwarding and durable mutation**

Extend `EngineObserver` and `EngineProgressObserver` with the reset operation. In `RuntimeObserver`, obtain the runtime submission checkpoint permit, locate the active root owner records, select the proposal or aggregate stage from `publication_source`, compare the stored identity, and clear the matching stage metadata atomically. Reject missing, terminal, mismatched, or conflicting owners.

**Step 4: Run the tests to verify they pass**

Run: `cargo test -p raiko2 clear_pending_proof_checkpoint`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/engine/src/lib.rs bin/raiko2/src/server/state/runtime_observer.rs bin/raiko2/src/server/state/runtime_observer_state_tests.rs
git commit -m "feat(engine): clear terminal provider checkpoints"
```

### Task 4: Reset Only Exhausted Boundless No-Lock Cycles

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs`
- Test: `crates/prover/src/boundless/mod.rs`

**Step 1: Write the failing tests**

Add tests proving that:

1. `NoLockAbortTimeout` produces a terminal-reset outcome carrying the exact request ID and attempt.
2. `NoLockRebidTimeout`, expiry, poll timeout, malformed status, and fulfillment errors do not clear the checkpoint.
3. The terminal-reset path calls the observer once before returning the proof-task failure.
4. A host clock past the lock deadline cannot terminalize the final attempt while the latest chain
   timestamp is before or equal to that deadline.
5. A status RPC error cannot terminalize the final attempt without a chain timestamp.

**Step 2: Run the tests to verify they fail**

Run: `cargo test -p raiko2-prover --features boundless terminal_checkpoint`

Expected: FAIL because final no-lock abort returns a generic fatal error.

**Step 3: Implement the terminal outcome**

Add a dedicated `BoundlessAttemptError` variant for exhausted no-lock rebids. In the main proof loop, acquire a checkpoint permit, compare-and-clear the exact persisted submission, log the terminalization, then return the original task failure. Preserve all existing behavior for every other error variant.

**Step 4: Run the tests to verify they pass**

Run: `cargo test -p raiko2-prover --features boundless terminal_checkpoint`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/prover/src/boundless/mod.rs
git commit -m "fix(boundless): reset exhausted no-lock submissions"
```

### Task 5: Verify Retry Semantics And Regression Surface

**Files:**
- Modify if required: `bin/raiko2/src/server/e2e.rs`
- Verify: `docs/API.md`
- Verify: `config.example.toml`

**Step 1: Add the retry regression test**

Cover the lifecycle sequence: persist attempt five, terminalize it, mark the execution failed, re-enqueue the same deterministic task, and confirm the next prover run sees no pending checkpoint and therefore begins at attempt one with a fresh provider request ID.

**Step 2: Run focused tests**

Run:

```bash
cargo test -p raiko2-prover --features boundless boundless
cargo test -p raiko2 clear_pending_proof_checkpoint
```

Expected: PASS.

**Step 3: Run formatting and lint verification**

Run:

```bash
cargo fmt --all
cargo clippy -p raiko2-prover --features boundless -- -D warnings
cargo clippy -p raiko2 -- -D warnings
git diff --check
```

Expected: all commands PASS with no warnings or whitespace errors.

**Step 4: Verify documentation compatibility**

Confirm that this is an internal lifecycle change with no HTTP response or configuration schema change. Do not edit `docs/API.md` or `config.example.toml` unless the implementation changes those contracts.

**Step 5: Commit final test or formatting changes**

```bash
git add bin/raiko2/src/server/e2e.rs
git commit -m "test(boundless): cover fresh retry after terminal reset"
```
