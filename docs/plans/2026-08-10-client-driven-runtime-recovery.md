# Client-Driven Runtime Recovery Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Prevent process startup from automatically executing persisted proof tasks while allowing a matching client request to resume the same deterministic root and provider checkpoint.

**Architecture:** Replace startup engine attachment with read-only runtime-state reconciliation, retaining only the existing legacy route migration mutation. Move stale-engine detection entirely to the duplicate-request path so client POST requests remain the sole trigger for reattachment.

**Tech Stack:** Rust, Tokio, raiko2 runtime state, engine lifecycle CAS, focused unit tests.

---

### Task 1: Lock Startup Behavior With Tests

**Files:**
- Modify: `bin/raiko2/src/server/handlers/proof_api.rs`

**Step 1:** Replace the existing startup requeue tests with tests asserting that unsubmitted and checkpointed roots are not attached to the recording engine.

**Step 2:** Run `cargo test -p raiko2 startup_recovery -- --nocapture` and verify the new assertions fail against the current implementation.

### Task 2: Make Duplicate Requests Client-Driven

**Files:**
- Modify: `bin/raiko2/src/server/handlers/proof_api.rs`

**Step 1:** Add a test showing that a nonterminal root with durable provider progress is reattachable when the engine has no active execution.

**Step 2:** Run the focused test and verify it fails because provider progress currently suppresses stale-engine inspection.

**Step 3:** Remove the provider-progress exclusion from stale nonterminal detection.

### Task 3: Remove Startup Execution Attachment

**Files:**
- Modify: `bin/raiko2/src/server/handlers/proof_api.rs`
- Modify: `bin/raiko2/src/server/state/mod.rs`

**Step 1:** Change startup recovery to validate and count persisted nonterminal roots without preparing or attaching execution plans.

**Step 2:** Preserve legacy route migration cancellation.

**Step 3:** Update startup logging to describe restored state rather than recovered queue work.

### Task 4: Verify and Publish

**Files:**
- Verify: `bin/raiko2/src/server/handlers/proof_api.rs`
- Verify: `bin/raiko2/src/server/state/mod.rs`

**Step 1:** Run `cargo fmt --all -- --check`.

**Step 2:** Run focused startup and duplicate-recovery tests.

**Step 3:** Run `cargo clippy -p raiko2 --all-targets -- -D warnings` and `cargo test -p raiko2`.

**Step 4:** Commit with a Conventional Commit message, push the branch, and open a PR with exact verification commands.
