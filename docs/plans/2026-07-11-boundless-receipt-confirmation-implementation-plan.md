# Boundless Receipt Confirmation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make an onchain Boundless submission delivery-confirmed only when its successful receipt contains a strictly decoded, internally consistent `RequestSubmitted` event and the receipt block timestamp is before the request's lock deadline.

**Architecture:** Keep the existing submission, persistence, balance-reservation, and retry flows intact. Harden the pure receipt-verdict helper to validate the complete ABI event, and enrich the receipt path with one provider lookup for the mined block header so the verdict uses chain time rather than the host clock. Every decode, identity, metadata, or provider failure remains an unconfirmed delivery and therefore preserves the existing same-id retry behavior.

**Tech Stack:** Rust 1.94, Tokio, Alloy 2.x through `boundless-market` 2.0.0, Cargo, GitHub Actions/Docker Linux verification

## Global Constraints

- Work only in `/Users/cai/.config/superpowers/worktrees/raiko2/pr164-receipt-fix` on `codex/boundless-bidding-lifecycle`.
- Do not change request pricing, persistence ordering, balance reservation, ceiling/final-rung decisions, or retry policy.
- Treat a reverted receipt, missing receipt metadata, block lookup error, malformed/duplicate/mismatched event, or late chain inclusion as an unconfirmed delivery.
- Use strict ABI decoding (`decode_log_validate`) and compare both `RequestSubmitted.requestId` and `RequestSubmitted.request.id` to the expected request id.
- Keep host time only for bounding how long to wait for a receipt; it must not decide whether a mined receipt was timely.
- Follow Conventional Commits and record every verification command and result in the final handoff.
- Native `raiko2-prover` test binaries cannot link on this macOS host: with `RISC0_SKIP_BUILD_KERNELS=1` they miss RISC0 native symbols, while without it the installed Xcode toolchain lacks `metal`. Run red/green unit tests in a Linux container or equivalent Linux runner; continue using native `cargo check`/`fmt`/`clippy` for fast compile feedback.

---

### Task 1: Strictly Decode and Validate `RequestSubmitted`

**Files:**

- Modify: `crates/prover/src/boundless/mod.rs:2632-2669`
- Modify tests: `crates/prover/src/boundless/mod.rs:4415-4540`

- [ ] **Step 1: Replace the synthetic positive fixture with a real ABI-encoded event**

  Change `request_submitted_log` so it builds a complete `ProofRequest` whose `id` is the requested id, constructs `IBoundlessMarket::RequestSubmitted`, and uses `SolEvent::encode_log_data()` for the log body and topics. Preserve a separate small helper for deliberately malformed raw logs.

  ```rust
  fn request_submitted_log(emitter: Address, request_id: U256) -> Log {
      let mut request = market_request(50, 5);
      request.id = request_id;
      let event = IBoundlessMarket::RequestSubmitted {
          requestId: request_id.into(),
          request,
          clientSignature: Bytes::new(),
      };
      Log { address: emitter, data: event.encode_log_data() }
  }
  ```

  If the generated user-defined value type does not implement `From<U256>`, construct the generated wrapper explicitly as indicated by the compiler; do not fall back to hand-built topics.

- [ ] **Step 2: Add failing regression cases**

  Extend `onchain_delivery_requires_matching_request_submitted_event` with these cases before changing production code:

  - correct emitter, signature, and indexed id but empty/truncated ABI data;
  - a valid event whose decoded body `request.id` differs from the indexed/expected id;
  - two matching candidate events in one receipt (ambiguous/duplicate);
  - the complete valid event still confirms.

  Assert malformed and inconsistent events return an error mentioning decode or id mismatch, not merely a generic absence when a candidate event was present.

- [ ] **Step 3: Run the focused test and observe RED**

  Run the test on Linux with Rust 1.94. A reusable command is:

  ```bash
  docker run --rm \
    -v "$PWD:/workspace" \
    -v raiko2-cargo-registry:/usr/local/cargo/registry \
    -v raiko2-linux-target:/workspace/target \
    -w /workspace \
    rust:1.94-bookworm \
    cargo test -p raiko2-prover --no-default-features --features boundless \
      onchain_delivery_requires_matching_request_submitted_event -- --nocapture
  ```

  Expected: at least the empty-data/body-id/duplicate regression fails against the current topic-only implementation. If the image lacks a native build prerequisite, install only the package named by the build error in the container and rerun; record the adjusted command.

- [ ] **Step 4: Implement strict candidate decoding**

  In `onchain_delivery_confirmation`:

  - reject reverted status first;
  - select candidate logs only by configured market address and the `RequestSubmitted` signature topic;
  - require exactly one candidate;
  - decode it with `IBoundlessMarket::RequestSubmitted::decode_log_validate`;
  - convert and compare the decoded indexed `requestId` with `market_request_id`;
  - compare decoded `request.id` with `market_request_id`;
  - return descriptive errors for absence, duplicates, decode failure, indexed mismatch, and body mismatch.

  Keep the deadline check in the helper for Task 2, but rename its time argument only there.

- [ ] **Step 5: Run the focused test and observe GREEN**

  Re-run the Linux command from Step 3. Expected: the full matching/malformed/mismatched/duplicate matrix passes.

- [ ] **Step 6: Run fast native compile feedback**

  ```bash
  RISC0_SKIP_BUILD_KERNELS=1 cargo check -p raiko2-prover --tests
  cargo fmt --all -- --check
  ```

  Expected: both pass.

- [ ] **Step 7: Commit the event-decoding fix**

  ```bash
  git add crates/prover/src/boundless/mod.rs
  git commit -m "fix(prover): validate boundless submission events"
  ```

---

### Task 2: Derive Receipt Timeliness from the Chain

**Files:**

- Modify: `crates/prover/src/boundless/mod.rs:20-35`
- Modify: `crates/prover/src/boundless/mod.rs:1768-1810`
- Modify: `crates/prover/src/boundless/mod.rs:2632-2685`
- Modify tests: `crates/prover/src/boundless/mod.rs:4542-4560`

- [ ] **Step 1: Make the pure deadline test explicitly chain-time based**

  Rename `onchain_delivery_rejects_receipts_after_the_payable_window` to `onchain_delivery_uses_chain_inclusion_time_for_the_payable_window`. Cover both boundaries:

  - inclusion timestamp `lock_expires_at - 1` confirms;
  - inclusion timestamp equal to `lock_expires_at` is rejected (the requirement is strict `<`).

  Name the helper argument and test values `included_at`/`block_timestamp`, never `now`.

- [ ] **Step 2: Add failing receipt-metadata/provider seam tests**

  Write focused tests against a planned small async helper or injected-closure seam proving missing block hash, provider error, and missing block all return `Err` and never reach a confirming verdict. Add the tests before defining the seam, so the initial compile failure is the RED result. The eventual seam must accept the receipt's block hash and return only a `u64` timestamp so business rules remain in `onchain_delivery_confirmation`.

- [ ] **Step 3: Run the focused tests and observe RED**

  Use the Linux container command from Task 1, changing the filter to the new chain-inclusion test/helper tests. Expected: the new seam or boundary behavior is absent before production changes.

- [ ] **Step 4: Fetch the mined block and pass its timestamp to the verdict**

  First define the small async helper or injected-closure seam referenced by the tests, choosing the form that keeps the concrete production call simplest.

  Import the Boundless Alloy traits needed by the concrete receipt/provider types:

  ```rust
  use boundless_market::alloy::{
      consensus::BlockHeader,
      network::BlockResponse,
      providers::Provider,
      sol_types::SolEvent,
  };
  ```

  After `pending_tx.get_receipt()` succeeds:

  - require `receipt.block_hash`;
  - call the market instance provider's `get_block_by_hash(block_hash)`;
  - map RPC failure and `None` to descriptive `Err(String)` values;
  - read `block.header().timestamp()`;
  - call `onchain_delivery_confirmation` with that timestamp.

  The operational `receipt_wait` may continue to use `now_secs()` to avoid waiting beyond the locally estimated window. Rename the verdict argument to `included_at` and update its error text to report the receipt block timestamp. Preserve `lock_expires_at == 0` as the existing no-deadline compatibility case.

- [ ] **Step 5: Run focused Linux tests and observe GREEN**

  Run the event test plus all chain-inclusion/lookup tests. Expected: all pass, including equality-at-deadline rejection and all metadata/provider failures.

- [ ] **Step 6: Run native compile and formatting checks**

  ```bash
  RISC0_SKIP_BUILD_KERNELS=1 cargo check -p raiko2-prover --tests
  cargo fmt --all -- --check
  ```

  Expected: both pass.

- [ ] **Step 7: Commit the chain-time fix**

  ```bash
  git add crates/prover/src/boundless/mod.rs
  git commit -m "fix(prover): verify boundless receipt chain time"
  ```

---

### Task 3: Align Operator Documentation and Verify the Complete Fix

**Files:**

- Modify: `docs/API.md:1160-1170`
- Modify: `docs/operations.md:815-826`

- [ ] **Step 1: Document the exact confirmation contract**

  Update both docs to say an onchain dispatch is acknowledged only when:

  - the receipt status succeeds;
  - the configured market emits one strictly decodable `RequestSubmitted` event;
  - both the indexed id and decoded request-body id equal the expected request id;
  - the receipt block timestamp is strictly before `lock_expires_at`.

  State that malformed/ambiguous events, missing block metadata, block lookup failures, and late inclusion leave the dispatch unconfirmed and retain same-id rebidding.

- [ ] **Step 2: Run the complete scoped test lane on Linux**

  ```bash
  docker run --rm \
    -v "$PWD:/workspace" \
    -v raiko2-cargo-registry:/usr/local/cargo/registry \
    -v raiko2-linux-target:/workspace/target \
    -w /workspace \
    rust:1.94-bookworm \
    cargo test -p raiko2-prover --no-default-features --features boundless boundless::tests
  ```

  Expected: all Boundless unit tests pass. If disk or native prerequisites prevent the full filtered suite, run every newly touched test plus `cargo check --tests`, record the environmental limitation verbatim, and rely on the PR's Linux checks before declaring completion.

- [ ] **Step 3: Run repository-required static verification**

  ```bash
  cargo fmt --all -- --check
  RISC0_SKIP_BUILD_KERNELS=1 cargo check -p raiko2-prover --tests
  RISC0_SKIP_BUILD_KERNELS=1 cargo clippy -p raiko2-prover --no-default-features --features boundless --tests -- -D warnings
  ```

  Expected: all pass.

- [ ] **Step 4: Review the final diff for scope and behavior**

  ```bash
  git diff fc0d4e731e913b62df82433864e6bef2de650475 --stat
  git diff fc0d4e731e913b62df82433864e6bef2de650475 -- \
    crates/prover/src/boundless/mod.rs docs/API.md docs/operations.md
  git status --short
  ```

  Confirm no generated ELF, lockfile, pricing, persistence, balance, or retry-policy changes slipped in.

- [ ] **Step 5: Commit the documentation**

  ```bash
  git add docs/API.md docs/operations.md
  git commit -m "docs(prover): clarify boundless receipt confirmation"
  ```

- [ ] **Step 6: Push the reviewed branch**

  ```bash
  git push origin codex/boundless-bidding-lifecycle
  ```

- [ ] **Step 7: Verify the remote PR and checks**

  ```bash
  gh pr view 164 --repo taikoxyz/raiko2 --json headRefOid,url,mergeStateStatus,statusCheckRollup
  gh pr checks 164 --repo taikoxyz/raiko2 --watch
  ```

  Confirm `headRefOid` matches local `git rev-parse HEAD`. Do not report completion until all required PR checks finish successfully; if a relevant check fails, inspect it, fix in scope, re-run verification, and push again.
