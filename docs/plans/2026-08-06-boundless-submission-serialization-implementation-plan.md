# Boundless Submission Serialization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make every in-process Boundless on-chain submission use one recoverable account order, three-confirmation receipts, and bounded external waits without allowing checkpoint storage to extend the account critical section.

**Architecture:** `BoundlessBalanceGate` remains the process-wide account controller. Its semaphore serializes one on-chain submission through three-confirmation receipt observation, while its short-lived state mutex stores funding reservations, a nonce high-water mark, and at most one uncertain signed submission. Required request-id persistence happens before the account permit; after account admission the path reacquires a lifecycle permit through the bounded broadcast outcome; optional transaction-hash persistence happens after both permits and is bounded.

**Tech Stack:** Rust, Tokio synchronization and timeouts, Alloy provider and transaction APIs, Boundless Market SDK.

---

### Task 1: Model Explicit Nonce Allocation And Uncertain Recovery

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs:1630-1800`
- Test: `crates/prover/src/boundless/mod.rs` unit test module

**Step 1: Write the failing tests**

Add pure state tests proving that:

- nonce allocation uses `max(latest, pending, local_high_water)`;
- allocating nonce `n` advances the local high-water mark to `n + 1`;
- an uncertain submission blocks allocation for a different request;
- reconciliation that observes a chain nonce greater than `n` clears the uncertain submission;
- unresolved reconciliation retains the exact request digest, signature, value, and nonce.

Use neutral fixture names and in-memory values only.

**Step 2: Run the focused tests to verify they fail**

```bash
cargo test -p raiko2-prover boundless_nonce_ --lib -- --nocapture
```

Expected: FAIL because funding state has no explicit nonce controller or uncertain-submission model.

**Step 3: Add the minimal state model**

Extend `BoundlessFundingState` with a local high-water mark and one uncertain submission. Keep the
network-independent transitions in methods that can be unit tested without an RPC client:

```rust
struct BoundlessUncertainSubmission {
    request: ProofRequest,
    signature: Bytes,
    request_digest: B256,
    value: U256,
    nonce: u64,
}

fn allocate_nonce(&mut self, latest: u64, pending: u64) -> RaikoResult<u64>;
fn record_uncertain(&mut self, submission: BoundlessUncertainSubmission);
fn reconcile_consumed_nonce(&mut self, chain_nonce: u64) -> bool;
```

Reject a fresh allocation while an uncertain submission remains. Do not hold the state mutex while
querying chain state or sending a transaction.

**Step 4: Run the focused tests**

Run the command from Step 2. Expected: PASS.

### Task 2: Bound Indexer, Balance, Nonce, And Checkpoint Waits

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs:65-75`
- Modify: `crates/prover/src/boundless/mod.rs:1190-1225`
- Modify: `crates/prover/src/boundless/mod.rs:1580-1630`
- Modify: `crates/prover/src/boundless/mod.rs:1890-1970`
- Test: `crates/prover/src/boundless/mod.rs` unit test module

**Step 1: Write failing timeout tests**

Add paused-time or injected-future tests showing that:

- the full indexer retry loop stops at one outer total timeout;
- RPC head and `balanceOf` calls cannot wait forever inside one retry attempt;
- optional transaction-hash persistence returns after its total timeout;
- a timed-out optional checkpoint does not retain the account submission permit.

**Step 2: Run the timeout tests to verify they fail**

```bash
cargo test -p raiko2-prover boundless_ --lib -- --nocapture
```

Expected: at least the new timeout tests FAIL because current timeouts cover individual attempts or
are absent.

**Step 3: Implement nested timeout boundaries**

Wrap each external attempt where useful, then wrap `retry_external(...)` itself in a total timeout.
Apply this pattern to indexer snapshots, RPC head and market balance reads, latest and pending nonce
reads, and optional checkpoint persistence:

```rust
tokio::time::timeout(TOTAL_TIMEOUT, retry_external(label, || async {
    tokio::time::timeout(ATTEMPT_TIMEOUT, operation()).await
        .map_err(|_| timeout_error())?
}))
.await
.map_err(|_| total_timeout_error())?
```

Keep fail-closed behavior: a balance or nonce timeout aborts the submission; an optional tx-hash
checkpoint timeout only logs a warning.

**Step 4: Run the timeout tests**

Run the command from Step 2. Expected: PASS.

### Task 3: Move Checkpoints Outside The Account Critical Section

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs:1160-1225`
- Modify: `crates/prover/src/boundless/mod.rs:2300-2420`
- Test: `crates/prover/src/boundless/mod.rs` unit test module

**Step 1: Write a failing sequencing test**

Extract or instrument the sequencing boundary so a test proves:

- the required request-id checkpoint completes before `acquire_submission()`;
- the lifecycle `SubmissionCheckpointPermit` is dropped after that checkpoint;
- the optional transaction-hash checkpoint starts only after the account submission permit is
  dropped.

**Step 2: Run the sequencing test to verify it fails**

```bash
cargo test -p raiko2-prover boundless_checkpoint_does_not_hold_account_permit --lib -- --nocapture
```

Expected: FAIL because both checkpoints currently run while the account permit is held.

**Step 3: Reorder submission setup**

Create `Submission` and persist its request identity after signing but before taking the account
permit. An uncertain predecessor obtains a fresh lifecycle permit only around its bounded recovery
and possible rebroadcast. For the new request, complete read-only funding and nonce queries under the
account permit, then acquire a fresh lifecycle permit immediately before funding reservation or
broadcast. This rejects a task whose runtime began draining while it waited for the account or
completed read-only RPCs. Retain that permit through the bounded send and receipt result. Afterward,
explicitly drop the lifecycle and account permits before invoking the bounded best-effort tx-hash
checkpoint. Keep the required checkpoint fail-closed and the optional checkpoint fail-open.

**Step 4: Run the sequencing test**

Run the command from Step 2. Expected: PASS.

### Task 4: Send And Recover At An Explicit Nonce

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs:2250-2430`
- Test: `crates/prover/src/boundless/mod.rs` unit test module

**Step 1: Write failing recovery tests**

Add focused tests around extracted recovery decisions proving that:

- a send timeout first checks latest and pending nonces and the known request digest;
- if nonce `n` is not consumed, recovery retries the same request, signature, value, and explicit
  nonce `n`;
- `already known`, `replacement transaction underpriced`, and `nonce too low` trigger state
  reconciliation instead of a blind fresh submission;
- the next request cannot allocate `n + 1` while `n` remains unresolved.

**Step 2: Run the recovery tests to verify they fail**

```bash
cargo test -p raiko2-prover boundless_nonce_recovery_ --lib -- --nocapture
```

Expected: FAIL because the SDK currently chooses the nonce during each send and timeout drops the
request identity needed for same-nonce recovery.

**Step 3: Implement explicit nonce submission and bounded recovery**

Before broadcast, query latest and pending transaction counts for the signer and allocate a nonce
under the short state lock. Build `submitRequest` with `.nonce(nonce)`, record the uncertain payload,
and send it under the existing broadcast timeout. On an ambiguous result, reconcile chain nonce and
request visibility; if still absent, rebuild the identical call with `.nonce(nonce)` and retry within
the recovery deadline. Do not construct a different request at that nonce.

An accepted send or a chain nonce greater than `nonce` clears the uncertain slot. An unresolved
result retains it and returns the existing submission for polling; the next account-permit holder
must recover it before sending fresh work.

**Step 4: Run the recovery tests and compile the SDK integration**

```bash
cargo test -p raiko2-prover boundless_nonce_ --lib -- --nocapture
cargo check -p raiko2-prover
```

Expected: PASS.

### Task 5: Use One Three-Confirmation Receipt Outcome

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs:1630-1695`
- Modify: `crates/prover/src/boundless/mod.rs:2250-2305`
- Test: `crates/prover/src/boundless/mod.rs` unit test module

**Step 1: Write failing finality tests**

Add tests proving the receipt builder is configured for three confirmations and the only outcomes
after that wait are:

- `status = true`: retain the funding reservation until indexer catch-up;
- `status = false`: remove only the matching digest reservation;
- timeout or watcher error: retain funding and nonce uncertainty.

**Step 2: Run the receipt tests to verify they fail**

```bash
cargo test -p raiko2-prover boundless_receipt_ --lib -- --nocapture
```

Expected: FAIL because the pending transaction currently uses Alloy's one-confirmation default.

**Step 3: Configure one bounded three-confirmation wait**

Call `with_required_confirmations(3)` before `get_receipt()` and increase the outer receipt timeout
to 90 seconds so Alloy polling and three-confirmation Sepolia progression fit. Increase nonce
recovery's outer budget to 180 seconds so it cannot truncate the receipt wait. Do not inspect or
react to a one-confirmation intermediate receipt. A returned revert is therefore removed
immediately; only an error or total timeout remains uncertain.

**Step 4: Run the receipt tests**

Run the command from Step 2. Expected: PASS.

### Task 6: Verify And Update PR 218

**Files:**
- Verify: `crates/prover/src/boundless/mod.rs`
- Verify: `docs/plans/2026-08-06-boundless-submission-serialization-design.md`
- Verify: `docs/plans/2026-08-06-boundless-submission-serialization-implementation-plan.md`

**Step 1: Run formatting and focused tests**

```bash
cargo fmt --all -- --check
cargo test -p raiko2-prover boundless_ --lib
```

Expected: PASS.

**Step 2: Run package verification**

```bash
cargo test -p raiko2-prover
cargo clippy -p raiko2-prover --all-targets -- -D warnings
```

Expected: PASS.

**Step 3: Run workspace verification required for cross-path prover changes**

```bash
cargo clippy --workspace -- -D warnings
```

Run the applicable test lanes from `.github/workflows/ci.yml`. Expected: PASS.

**Step 4: Check path hygiene and diff**

Inspect every added line for hardcoded absolute paths, user-specific directories, human-identifying
fixture names, generated ELF changes, and unrelated modifications. Run `git diff --check`.

**Step 5: Commit, push, and answer the review threads**

Use Conventional Commits. Push `fix/boundless-outstanding-funding`, reply to each inline PR 218
review thread with the concrete fix and verification command, then re-read current comments and CI.
