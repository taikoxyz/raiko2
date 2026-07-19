# SP1 Retry Budget Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the SP1 paid-request budget durable across queue retries, preserve valid request-scoped network-mode overrides, and document the resulting operator contract.

**Architecture:** Persist the one-based SP1 request attempt next to each provider request ID and resume both values together through the prover/engine observer boundary. Keep legacy metadata compatible by treating a missing or zero attempt as attempt one. Resolve a request override that switches to reserved mode by dropping the inherited mainnet-only auction timeout before validation.

**Tech Stack:** Rust 1.94, Tokio, Serde, raiko2 prover/engine/runtime crates, Markdown operator documentation.

## Global Constraints

- Keep `max_request_attempts` operator-only; do not add it to `Sp1ConfigOverrides`.
- Preserve proposal and aggregation request metadata separately.
- Use Serde defaults for records written before the attempt field exists.
- Do not modify generated guest ELF or verification-key artifacts.
- Follow Conventional Commits and report exact verification commands.

---

### Task 1: Preserve reserved request overrides

**Files:**
- Modify: `crates/prover/src/sp1_config.rs`
- Test: `crates/prover/src/sp1_config.rs`

**Interfaces:**
- Consumes: `Sp1Config::resolve_request_config` and `Sp1ConfigOverrides`.
- Produces: an effective reserved config whose `auction_timeout_secs` is `None` when the request switches away from a global mainnet config.

- [x] **Step 1: Write the failing test**

```rust
#[test]
fn sp1_reserved_override_clears_inherited_auction_timeout() {
    let base = Sp1Config {
        network_mode: Sp1NetworkMode::Mainnet,
        fulfillment_strategy: Sp1FulfillmentStrategy::Auction,
        auction_timeout_secs: Some(300),
        ..Sp1Config::default()
    };
    let overrides = Sp1ConfigOverrides {
        network_mode: Some(Sp1NetworkMode::Reserved),
        fulfillment_strategy: Some(Sp1FulfillmentStrategy::Reserved),
        ..Sp1ConfigOverrides::default()
    };

    let effective = base
        .resolve_request_config(
            Some(&overrides),
            Sp1RequestContext::ProposalBatch { aggregate: false },
        )
        .expect("reserved override should discard a mainnet-only timeout");
    assert_eq!(effective.network_mode, Sp1NetworkMode::Reserved);
    assert_eq!(effective.auction_timeout_secs, None);
}
```

- [x] **Step 2: Run the test to verify it fails**

Run: `cargo test -p raiko2-prover --no-default-features --features chain-spec-json sp1_reserved_override_clears_inherited_auction_timeout`

Expected: FAIL because `resolve_request_config` returns `AuctionTimeoutRequiresMainnet`.

- [x] **Step 3: Implement the minimal merge rule**

After merging fields, clear `effective_config.auction_timeout_secs` when `overrides.network_mode == Some(Sp1NetworkMode::Reserved)` and no request auction timeout was supplied. Keep global reserved configurations invalid at startup.

- [x] **Step 4: Run the focused and full config tests**

Run: `cargo test -p raiko2-prover --no-default-features --features chain-spec-json sp1_config`

Expected: 12 tests pass.

### Task 2: Persist and restore the SP1 request attempt

**Files:**
- Modify: `crates/prover/src/sp1_config.rs`
- Modify: `crates/prover/src/lib.rs`
- Modify: `crates/prover/src/sp1/mod.rs`
- Modify: `crates/engine/src/lib.rs`
- Modify: `bin/raiko2/src/server/task_metadata.rs`
- Modify: `bin/raiko2/src/server/state/runtime_observer.rs`
- Test: `bin/raiko2/src/server/state/runtime_observer.rs`

**Interfaces:**
- Produces: `Sp1NetworkSubmissionResume { provider_request_id: String, request_attempt: u64 }`.
- Produces: `ProverProgressObserver::load_sp1_network_submission()` and `EngineObserver::load_sp1_network_submission()` with backward-compatible defaults through the existing request-ID method.
- Persists: `TaskRuntimeMetadata::sp1_request_attempt: Option<u64>`.

- [x] **Step 1: Add a failing runtime-observer regression test**

Extend the existing SP1 proposal progress/resume test to emit `request_attempt: 3`, assert that task metadata contains `Some(3)`, and assert that the loaded resume value contains both `provider_request_id == "0xsp1"` and `request_attempt == 3`.

- [x] **Step 2: Run the test to verify it fails**

Run: `RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 --no-default-features --features host runtime_observer_records_sp1_network_submission_metadata`

Expected: compilation fails because the progress/resume attempt fields do not exist yet.

- [x] **Step 3: Add the resume type and compatibility defaults**

Add a Serde-defaulted `request_attempt` to `Sp1NetworkSubmissionProgress`, add `Sp1NetworkSubmissionResume`, and export it. A missing or zero legacy value must normalize to one on load.

- [x] **Step 4: Carry the resume value through engine observers**

Add `load_sp1_network_submission` default methods that adapt the existing `load_sp1_network_request_id` result to attempt one. Override the new method in the runtime observer so it reads the persisted attempt for proposal and aggregation metadata independently.

- [x] **Step 5: Persist the attempt and initialize the prover loop from it**

Store `request_attempt` in task runtime metadata on every SP1 submission progress event. Pass the one-based attempt into `notify_sp1_network_submission`, and initialize both `stored_request_id` and `request_attempt` from `load_sp1_network_submission`.

- [x] **Step 6: Run focused runtime and prover checks**

Run: `RISC0_SKIP_BUILD_KERNELS=1 cargo test -p raiko2 --no-default-features --features host runtime_observer_`

Run: `RISC0_SKIP_BUILD_KERNELS=1 cargo check -p raiko2-prover --tests --no-default-features --features chain-spec-json,sp1`

Expected: all selected tests and compilation pass.

### Task 3: Document and publish the fix

**Files:**
- Modify: `docs/API.md`
- Modify: `docs/superpowers/plans/2026-07-19-sp1-retry-budget-review-fixes.md`

**Interfaces:**
- Documents: default `max_request_attempts = 3`, retryable terminal accounting, durable resume behavior, operator-only scope, and mainnet-only auction timeouts.

- [x] **Step 1: Update `docs/API.md`**

Add SP1 configuration bullets next to the existing cycle-limit and queue-retry documentation. Explain that the durable submission budget spans queue retries for one stage and that switching a request to reserved mode discards an inherited mainnet-only auction timeout.

- [x] **Step 2: Run repository verification**

Run: `cargo fmt --all --check`

Run: `RISC0_SKIP_BUILD_KERNELS=1 cargo clippy --workspace -- -D warnings`

Run the focused test commands from Tasks 1 and 2,
`cargo test -p raiko2-queue -p raiko2-runtime`, and
`RISC0_SKIP_BUILD_KERNELS=1 cargo run -p xtask-build-guest --bin xtask-build-guest -- sp1 --check`.

Expected: every command exits zero; only the two documented pre-existing missing-doc warnings may appear in the no-default-feature prover test lane.

- [x] **Step 3: Review, commit, and push**

Inspect `git diff --check`, `git diff`, and `git status --short`. Stage only the planned files, commit with `fix(prover): persist SP1 request budget`, and push `perf/sp1-guest-bn254-and-network-budget` to `origin`.
