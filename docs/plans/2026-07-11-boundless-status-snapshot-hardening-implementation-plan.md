# Boundless Status and Snapshot Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Boundless expiry classification use one coherent chain block and make current exact-price snapshots fail closed.

**Architecture:** Split latest-block discovery from the block-pinned status batch, then classify with that block's timestamp. Keep legacy missing exact prices as the zero sentinel, but reject malformed present values.

**Tech Stack:** Rust 1.94, Tokio, JSON-RPC, Alloy primitives, Boundless Market 2.0.0, Cargo

## Global Constraints

- Preserve final-rung payable-window behavior.
- Rotate IDs only from a coherent successful read proving market or lock expiry.
- Preserve legacy snapshots with `max_price_wei = None`.
- Do not change snapshot schema version or generated guest artifacts.

---

### Task 1: Pin market status reads to one block and query lock expiry

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs`
- Test: `crates/prover/src/boundless/mod.rs`

**Interfaces:**
- Consumes: latest block number and timestamp returned by `eth_getBlockByNumber`.
- Produces: `build_status_batch(submissions, block_number)` whose `eth_call` entries all use the same block tag and whose third selector is `requestLockDeadline(uint256)`.

- [ ] **Step 1: Write failing request-construction and lock-expiry tests**

Add a unit test that builds a status batch for block `0x2a`, asserts every call's second parameter is `"0x2a"`, and asserts the third call uses the `requestLockDeadline(uint256)` selector. Extend status classification coverage with `lock_deadline < block_timestamp < expires_at` and expect `LockExpired`.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test -p raiko2-prover --no-default-features --features boundless \
  boundless_status_batch_pins_market_reads_to_one_block -- --nocapture
```

Expected: FAIL because status calls still use `latest` and the request deadline selector.

- [ ] **Step 3: Implement block-pinned polling**

Fetch the latest block in a first request, parse both number and timestamp, then build the status
batch with this explicit block number. Change `eth_call_request` to accept a block tag and change
the deadline selector to `requestLockDeadline(uint256)`.

- [ ] **Step 4: Run focused tests and verify GREEN**

Re-run the test from Step 2 plus `classify_boundless_status_covers_timeout_actions`. Expected: PASS.

### Task 2: Reject malformed present exact prices

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs`
- Test: `crates/prover/src/boundless/mod.rs`

**Interfaces:**
- Consumes: `BoundlessSubmissionSnapshot.max_price_wei: Option<String>`.
- Produces: `None => U256::ZERO`; valid `Some` => parsed `U256`; malformed `Some` => `RaikoError::Guest`.

- [ ] **Step 1: Write the failing malformed-price test**

Construct a version-1 snapshot with `max_price_wei = Some("not-wei")` and assert conversion returns an error containing `max_price_wei` and the invalid value. Retain the existing missing-field compatibility test.

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test -p raiko2-prover --no-default-features --features boundless \
  resumed_submission_rejects_malformed_exact_price -- --nocapture
```

Expected: FAIL because conversion currently succeeds with the zero sentinel.

- [ ] **Step 3: Implement fail-closed parsing**

Match on `max_price_wei`: parse present values with a descriptive error and use zero only when the field is absent.

- [ ] **Step 4: Run focused tests and verify GREEN**

Re-run the malformed-price and legacy snapshot tests. Expected: PASS.

### Task 3: Verify and publish

**Files:**
- Verify all modified files.

**Interfaces:**
- Consumes: Tasks 1 and 2.
- Produces: a reviewed commit on `codex/boundless-bidding-lifecycle`.

- [ ] **Step 1: Run formatting, check, clippy, and focused tests**

```bash
cargo fmt --all -- --check
RISC0_SKIP_BUILD_KERNELS=1 cargo check -p raiko2-prover --tests
RISC0_SKIP_BUILD_KERNELS=1 cargo clippy -p raiko2-prover --no-default-features --features boundless --tests -- -D warnings
```

Run the full scoped Boundless test lane on Linux when native linking hits the documented RISC0 macOS limitation.

- [ ] **Step 2: Review the final diff and request code review**

Confirm only the planned polling, selector, snapshot parsing, tests, and design/plan documentation changed.

- [ ] **Step 3: Commit and push**

```bash
git add crates/prover/src/boundless/mod.rs docs/plans/2026-07-11-boundless-status-snapshot-hardening-*.md
git commit -m "fix(prover): harden boundless status snapshots"
git push origin codex/boundless-bidding-lifecycle
```
