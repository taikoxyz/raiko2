# PR 164 Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the two review gaps in PR #164 without changing its request-ID, pricing, or persistence contracts.

**Architecture:** Boundless status polling will carry the hash and timestamp of one fetched block, then issue every market `eth_call` with the EIP-1898 `{ blockHash, requireCanonical: true }` selector so a reorg becomes a transient RPC failure instead of a mixed snapshot. SP1 failed-task resume eligibility will use the provider tag plus a nonempty request ID as its source of truth, including provider-ID-only legacy records.

**Tech Stack:** Rust, Tokio, Reqwest JSON-RPC, Serde JSON, Alloy primitives, Cargo unit tests.

## Global Constraints

- Preserve all existing Boundless request-ID rotation, price escalation, and persistence behavior.
- Treat missing or noncanonical block-hash reads as transient polling failures.
- Preserve legacy Boundless/SP1 record deserialization and tagged dual-write behavior.
- Follow red-green TDD for each fix.

---

### Task 1: Canonical Boundless status snapshot

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs`
- Test: `crates/prover/src/boundless/mod.rs`

**Interfaces:**
- Consumes: `eth_getBlockByNumber` JSON containing `hash` and `timestamp`.
- Produces: `eth_call` requests whose second parameter is `{ "blockHash": <hash>, "requireCanonical": true }`.

- [ ] **Step 1: Write the failing tests**

Update the block-reference parser test to expect the fetched block hash, and update the batch-request test to expect the canonical EIP-1898 selector on every call.

```rust
let block = serde_json::json!({
    "hash": "0x000000000000000000000000000000000000000000000000000000000000002a",
    "number": "0x2a",
    "timestamp": "0x64",
});
let (block_hash, timestamp) = parse_block_reference(&block).expect("block reference");
assert_eq!(block_hash, block["hash"]);
assert_eq!(timestamp, 100);

let expected = serde_json::json!({
    "blockHash": block_hash,
    "requireCanonical": true,
});
assert!(batch.iter().all(|request| request.params.get(1) == Some(&expected)));
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
cargo test -p raiko2-prover --no-default-features --features boundless \
  boundless::tests::boundless_status_batch_pins_market_reads_to_one_block
```

Expected: the selector assertion fails because the current implementation serializes a block-number string.

- [ ] **Step 3: Implement the minimal canonical selector**

Make `parse_block_reference` return the required `hash` plus timestamp, pass that hash through `build_batch_request`, and serialize the EIP-1898 selector in `eth_call_request`.

```rust
serde_json::json!({
    "blockHash": block_hash,
    "requireCanonical": true,
})
```

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run the focused Boundless status tests and confirm the canonical selector and deadline classification tests pass.

---

### Task 2: Provider-ID-only SP1 failed-task resume

**Files:**
- Modify: `bin/raiko2/src/server/task_metadata.rs`
- Test: `bin/raiko2/src/server/task_metadata.rs`
- Test: `bin/raiko2/src/server/state/runtime_observer.rs`

**Interfaces:**
- Consumes: `RemoteSubmissionMetadata::Sp1` records with a nonempty `provider_request_id`.
- Produces: `has_resumable_remote_submission() == true` and the same ID from `load_sp1_network_request_id`, even when legacy mode/strategy/timeout fields are absent.

- [ ] **Step 1: Write the failing tests**

Extend the provider-ID-only metadata regression to assert actual resume eligibility, and add a failed-root observer regression that loads the same ID.

```rust
assert!(metadata.has_sp1_network_submission_progress());
assert!(metadata.has_resumable_remote_submission());
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
cargo test -p raiko2 legacy_provider_id_only_runtime_metadata_remains_sp1_resumable
```

Expected: the resume-eligibility assertion fails because the current predicate requires another SP1 field.

- [ ] **Step 3: Implement the minimal eligibility fix**

Change `has_sp1_network_submission_progress` to require only an SP1-tagged record with a nonempty provider request ID.

```rust
self.sp1_network_submission().is_some_and(|submission| {
    submission
        .provider_request_id
        .as_deref()
        .is_some_and(|request_id| !request_id.is_empty())
})
```

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run the metadata and runtime-observer resume tests and confirm both current tagged records and legacy provider-ID-only records resume the stored ID.

---

### Task 3: Verification and publication

**Files:**
- Verify all modified files above.

**Interfaces:**
- Produces: formatted, reviewed commits pushed to `origin/codex/boundless-bidding-lifecycle`.

- [ ] **Step 1: Run formatting and focused verification**

```bash
cargo fmt --all -- --check
cargo test -p raiko2 task_metadata
cargo test -p raiko2 runtime_observer
cargo check -p raiko2-prover --tests --no-default-features --features boundless
git diff --check
```

- [ ] **Step 2: Review the final diff**

Confirm only the canonical block selector, SP1 resume predicate/tests, and this plan are changed.

- [ ] **Step 3: Request a focused code review**

Review the new commit range against the two PR findings; fix any Critical or Important issue before publication.

- [ ] **Step 4: Commit and push**

```bash
git add docs/plans/2026-07-13-pr-164-review-fixes.md \
  crates/prover/src/boundless/mod.rs \
  bin/raiko2/src/server/task_metadata.rs \
  bin/raiko2/src/server/state/runtime_observer.rs
git commit -m "fix(prover): close pr 164 review gaps"
git push -u origin HEAD:codex/boundless-bidding-lifecycle
```
