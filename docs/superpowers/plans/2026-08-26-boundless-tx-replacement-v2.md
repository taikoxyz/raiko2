# Boundless Transaction Replacement V2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every on-chain Boundless `submitRequest` use a bounded, same-nonce EIP-1559 replacement lifecycle before market polling or market-level rebidding begins.

**Architecture:** One process-local signer gate serializes all on-chain submissions. Each market offer owns one transaction lifecycle: allocate one nonce from `max(latest, pending, local high-water)`, persist the exact request digest before broadcast, retry the same calldata/value/nonce with bounded fee increases, and accept any known hash that reaches three confirmations. Market polling starts only after confirmation; every later market rebid creates a new transaction lifecycle through the same gate.

**Tech Stack:** Rust, Tokio, Alloy provider APIs, Boundless Market SDK, serde-backed runtime metadata.

**Spec:** Approved conversation design for PR #222: signer submissions are serialized; request 2 replacement blocks request 3/4 submission; market rebids occur only after the preceding transaction confirms and independently use the same replacement policy.

## Global Constraints

- Work from current `origin/main`; the old PR branch is reference material only.
- Preserve the one-active-writer deployment invariant for each Boundless signer.
- Never allocate a fresh nonce while an earlier nonce has an ambiguous broadcast outcome.
- A replacement keeps request, signature, calldata, attached value, gas limit, and nonce unchanged.
- Stop new broadcasts once the offer lock deadline has elapsed, while still reconciling known hashes and exact events.
- Require three confirmations before market polling or market-level rebidding.
- Never rotate a market request ID from status-RPC errors or from failure to fetch a payload after fulfillment was observed.
- Keep off-chain Boundless behavior unchanged.
- Do not expose RPC credentials in errors or logs.

---

### Task 1: Transaction Policy And Fee Ladder

**Files:**
- Modify: `crates/prover/src/boundless_config.rs`
- Modify: `crates/prover/src/boundless/mod.rs`
- Test: unit tests in both files

**Interfaces:**
- Produces: `BoundlessTransactionConfig`, `BoundlessTxFees`, `next_boundless_tx_fees`, and the bounded transaction runner.
- Consumes: existing Boundless RPC and request types.

- [ ] **Step 1: Add failing tests for config bounds and fee progression**

  Cover default retry knobs, the hard fee ceiling, the ten-percent txpool replacement floor, attempt-duration product bounds, and `max_replacements = 0`.

- [ ] **Step 2: Run focused tests and verify they fail because the policy and ladder do not exist**

  Run: `cargo test -p raiko2-prover boundless_transaction --lib -- --nocapture`

- [ ] **Step 3: Implement the minimal policy and pure fee helpers**

  Parse `max_fee_per_gas_wei` as a positive decimal `u128`; default only the bounded timing/bump fields. Never emit a cap-clamped rung below the standard ten-percent replacement floor.

- [ ] **Step 4: Run focused tests and verify they pass**

  Run the command from Step 2.

### Task 2: Same-Nonce Replacement And Signer Serialization

**Files:**
- Modify: `crates/prover/src/boundless/mod.rs`
- Test: `crates/prover/src/boundless/mod.rs`

**Interfaces:**
- Consumes: `BoundlessTransactionConfig` and existing `BoundlessBalanceGate`.
- Produces: one bounded transaction lifecycle per offer and conservative uncertain-state recovery.

- [ ] **Step 1: Add failing transaction-runner tests**

  Prove that all replacement attempts reuse one nonce, known hashes are observed even after a later send error, any confirmed hash wins, confirmed revert is terminal, attempts stop at the configured cap, and four concurrent callers are serialized by the signer gate.

- [ ] **Step 2: Run focused tests and verify the current unbounded same-fee recovery fails them**

  Run: `cargo test -p raiko2-prover boundless_ --lib -- --nocapture`

- [ ] **Step 3: Implement fixed gas/fee broadcast and bounded observation**

  Complete fee/gas preparation and durable checkpointing first, then record the funding reservation and uncertain nonce immediately before the first send. Store every acknowledged transaction hash. Release a nonce only when no hash exists and every outcome is a definitive pre-broadcast rejection; otherwise preserve uncertainty and block later signer work. Reject an RPC pending nonce ahead of the latest mined nonce so a restart cannot queue work behind an unknown predecessor.

- [ ] **Step 4: Run focused tests and verify they pass**

  Run the command from Step 2.

### Task 3: Deadline And Restart Identity

**Files:**
- Modify: `crates/prover/src/lib.rs`
- Modify: `crates/prover/src/boundless/mod.rs`
- Modify: `bin/raiko2/src/server/task_metadata.rs`
- Modify: `bin/raiko2/src/server/state/runtime_observer.rs`
- Test: unit tests in each touched module

**Interfaces:**
- Produces: optional persisted `request_digest` and `broadcast_from_block` fields for on-chain Boundless checkpoints.
- Consumes: `ProofRequest::signing_hash` and `RequestSubmitted` logs.

- [ ] **Step 1: Add failing tests for exact resume identity**

  Persist two rebid rungs sharing one request ID and prove that resume accepts only the event whose request signing digest matches the checkpoint. Also prove every replacement authorization rejects a newly expired lock deadline.

- [ ] **Step 2: Run focused tests and verify request-ID-only recovery and one-time deadline checks fail**

  Run: `cargo test -p raiko2 boundless -- --nocapture` and `cargo test -p raiko2-prover boundless --lib -- --nocapture`.

- [ ] **Step 3: Persist and validate the exact on-chain identity**

  Store a canonical digest and the request id's earliest pre-broadcast lower block in runtime metadata. Preserve that lower block across same-id rebids. On restart without a confirmed hash, scan only that bounded block range, recompute each event request digest for the configured market and chain, and require an exact match before three-confirmation observation.

  Persist an explicit `request_id_has_confirmed_submission` bit instead of inferring this state from the attempt number. A missing event remains fail-closed until its lock deadline unless that bit proves the same request id already has a confirmed rung. A legacy checkpoint without an exact digest waits until the deadline and then receives one final market-status lifecycle, recovering an already-paid fulfillment before replacing an unfulfilled checkpoint. Only a successful pinned status read may authorize that replacement; RPC errors retain the checkpoint. Restore every durable unconfirmed on-chain checkpoint as a process-local signer blocker before workers start, independently of the selected RPC's mempool view.

- [ ] **Step 4: Enforce the deadline on every broadcast authorization**

  An expired offer prevents another replacement send. Continue bounded known-hash observation, exact-event scanning, and follow-up receipt checks for an already-started or ambiguous send even when recovery crosses the lock deadline. Release local uncertainty only after that final recovery cannot find a confirmed transaction, then return the terminal task error.

- [ ] **Step 5: Run focused tests and verify they pass**

  Run the commands from Step 2.

### Task 4: Configuration And Operator Contract

**Files:**
- Modify: `bin/raiko2/src/config/prover.rs`
- Modify: `bin/raiko2/src/config/mod.rs`
- Modify: `bin/raiko2/src/server/state/setup.rs`
- Modify: `config.example.toml`
- Modify: `docker/config.compose.toml`
- Modify: `README.md`
- Modify: `docs/API.md`
- Modify: `docs/operations.md`
- Test: config and server tests

**Interfaces:**
- Produces: `[prover.risc0.boundless.transaction]` for on-chain mode.
- Consumes: existing Boundless route configuration.

- [ ] **Step 1: Add failing config tests**

  Require the transaction table only when `offchain = false`; allow old off-chain configurations; reject malformed bounds without echoing secret RPC URLs.

- [ ] **Step 2: Implement config plumbing and update canonical examples**

  Document the atomic image/config cutover, the distinction between transaction replacement and market rebid, and the one-writer signer invariant.

- [ ] **Step 3: Run config and server tests**

  Run: `cargo test -p raiko2 --bin raiko2`.

### Task 5: Full Verification And PR Update

**Files:**
- Review all modified files.

**Interfaces:**
- Produces: an updated, mergeable PR #222 based on current `main`.

- [ ] **Step 1: Run formatting and targeted suites**

  Run: `cargo fmt --all -- --check`, `cargo test -p raiko2-prover --lib`, and
  `cargo test -p raiko2 --bin raiko2`.

- [ ] **Step 2: Run workspace lint**

  Run: `cargo clippy --workspace -- -D warnings`.

- [ ] **Step 3: Review for stale implementations and path/secret hygiene**

  Run: `git diff --check` and inspect every changed config, log, and error path. Confirm no duplicate recovery implementation remains and no local path or credential appears.

- [ ] **Step 4: Commit and update PR #222**

  Use Conventional Commits, update the PR description with current commands/results, and respond to each still-applicable inline review thread with the exact fix and test.
