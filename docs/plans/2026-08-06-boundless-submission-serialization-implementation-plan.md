# Boundless Submission Serialization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Serialize every Boundless market submission made by one Raiko2 process, while retaining conservative funding reservations across uncertain transaction outcomes.

**Architecture:** Split `BoundlessBalanceGate` into a process-wide submission semaphore and a short-lived funding-state mutex. Hold the semaphore from the aligned indexer/balance reads through broadcast and a bounded receipt wait, but never hold the state mutex across network I/O. Reconcile only a confirmed reverted receipt by removing its reservation; all uncertain outcomes retain it.

**Tech Stack:** Rust, Tokio synchronization/timeouts, Alloy transaction receipts, Boundless Market SDK.

---

### Task 1: Split Submission Serialization From Funding State

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs:1595-1717`
- Test: `crates/prover/src/boundless/mod.rs` unit test module

**Step 1: Write the failing test**

Add an async test that acquires the gate's submission permit, verifies a second permit cannot be
acquired concurrently, drops the first permit, and verifies the second acquisition succeeds.

**Step 2: Run the test to verify it fails**

Run:

```bash
cargo test -p raiko2-prover boundless_submission_gate_serializes_callers -- --exact
```

Expected: FAIL because the gate has no independent submission permit.

**Step 3: Write the minimal implementation**

Replace the tuple gate with a cloneable structure containing:

```rust
submission: Arc<tokio::sync::Semaphore>,
state: Arc<tokio::sync::Mutex<BoundlessFundingState>>,
```

Add `acquire_submission` and `lock_state` helpers. Keep the semaphore capacity at one and keep
`BoundlessFundingState` private.

**Step 4: Run the test to verify it passes**

Run the same focused command. Expected: PASS.

**Step 5: Commit**

```bash
git add crates/prover/src/boundless/mod.rs
git commit -m "refactor(boundless): serialize account submissions"
```

### Task 2: Reconcile Confirmed Receipt Outcomes

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs:1606-1695`
- Test: `crates/prover/src/boundless/mod.rs` unit test module

**Step 1: Write the failing tests**

Add tests proving that:

- a confirmed reverted transaction removes only its matching request digest;
- a confirmed successful transaction retains the reservation until indexer reconciliation;
- an uncertain outcome retains the reservation.

**Step 2: Run the tests to verify they fail**

Run:

```bash
cargo test -p raiko2-prover boundless_receipt_ -- --nocapture
```

Expected: FAIL because receipt reconciliation is not implemented.

**Step 3: Write the minimal implementation**

Add a private receipt-outcome enum and a `BoundlessFundingState` reconciliation method. Remove the
matching digest only for the confirmed-reverted outcome, removing the request-id entry when its
digest map becomes empty. Leave successful and uncertain outcomes unchanged.

**Step 4: Run the tests to verify they pass**

Run the same focused command. Expected: PASS.

**Step 5: Commit**

```bash
git add crates/prover/src/boundless/mod.rs
git commit -m "fix(boundless): reconcile reverted submissions"
```

### Task 3: Bound Broadcast And Receipt Waiting

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs:2200-2275`
- Test: `crates/prover/src/boundless/mod.rs` existing and new unit tests

**Step 1: Write the failing structural test**

Add focused tests for the pure receipt reconciliation path from Task 2, then use compilation and the
submission-gate test as the integration boundary for the SDK transaction type. Full market calls are
not mocked because the SDK client is concrete and a mock would duplicate its transaction behavior.

**Step 2: Implement the submission sequence**

In the Boundless submit path:

1. Acquire the shared submission permit.
2. Fetch the indexer snapshot and market balance without the funding-state mutex.
3. Lock funding state only to calculate the deposit, then release it.
4. Persist request identity, record the local reservation under a short state lock, and broadcast
   under a 30-second Tokio timeout.
5. Persist the transaction hash immediately after broadcast.
6. Wait up to 10 seconds for the receipt while retaining the submission permit.
7. Remove the reservation only when the receipt is confirmed reverted; retain it on success,
   broadcast error/timeout, or receipt error/timeout.

**Step 3: Run focused tests and compilation**

Run:

```bash
cargo test -p raiko2-prover boundless_ --lib
cargo check -p raiko2-prover
```

Expected: PASS.

**Step 4: Commit**

```bash
git add crates/prover/src/boundless/mod.rs
git commit -m "fix(boundless): bound serialized submission waits"
```

### Task 4: Verify And Update PR 218

**Files:**
- Verify: `crates/prover/src/boundless/mod.rs`
- Verify: `docs/plans/2026-08-06-boundless-submission-serialization-design.md`
- Verify: `docs/plans/2026-08-06-boundless-submission-serialization-implementation-plan.md`

**Step 1: Run formatting**

```bash
cargo fmt --all -- --check
```

Expected: PASS.

**Step 2: Run the full prover tests**

```bash
cargo test -p raiko2-prover
```

Expected: PASS.

**Step 3: Run prover clippy**

```bash
cargo clippy -p raiko2-prover --all-targets -- -D warnings
```

Expected: PASS.

**Step 4: Check path hygiene and diff**

Inspect the complete PR diff for hardcoded local paths, personal names, generated ELF changes, and
unrelated modifications.

**Step 5: Push and report**

Push `fix/boundless-outstanding-funding`, update PR 218 with the concurrency/timeout behavior and
exact verification commands, then re-read current review comments and checks.
