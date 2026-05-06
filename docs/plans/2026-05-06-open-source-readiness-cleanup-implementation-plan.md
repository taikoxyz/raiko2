# Open Source Readiness Cleanup Implementation Plan

> Execute this plan task-by-task, validating each step before moving to the next.

**Goal:** Remove the highest-risk blockers to publishing `raiko2` as a public repository and align public-facing docs with the current project workflow.

**Architecture:** Keep this PR scoped to repository hygiene, deterministic mock behavior, and public entry docs. Do not attempt full infrastructure/example sanitization here; that is the follow-up PR.

**Tech Stack:** Rust workspace crates, Markdown docs, git-tracked repository assets, `.gitignore`

---

### Task 1: Remove tracked runtime artifacts and ignore future copies

**Files:**
- Modify: `.gitignore`
- Delete: `guests/risc0/core.268699`
- Delete: `guests/risc0/core.280521`
- Delete: `guests/sp1/core.280557`
- Delete: `test/regression/shasta/regression.log`

**Steps:**
1. Add ignore rules for `core.*` and regression/runtime log artifacts.
2. Remove the tracked crash dumps and regression log from the repository.
3. Verify `git status --short` only shows the intended deletions and ignore updates.

### Task 2: Replace the hardcoded NativeProver private key with explicit mock behavior

**Files:**
- Modify: `crates/prover/src/native.rs`

**Steps:**
1. Replace the fixed private-key constant and ECDSA signing helpers with deterministic mock helpers.
2. Keep `native/local` proof output shape stable: same proof length, same mock instance id, deterministic mock instance address, deterministic mock signature bytes.
3. Update tests so they validate deterministic mock behavior instead of recovering a signer from a secret key.
4. Run focused `cargo test -p raiko2-prover native_ -- --nocapture` or exact test targets covering the updated native prover tests.

### Task 3: Clean up public-facing entry docs

**Files:**
- Modify: `README.md`
- Modify: `docs/README.md`
- Modify: `docs/development.md`
- Modify: `AGENTS.md`
- Add: `docs/issues/2026-05-06-open-source-readiness-review.md`

**Steps:**
1. Remove or soften public references to internal Codex/agent workflow from `README.md`.
2. Update `docs/README.md` so historical plans are clearly secondary and not the main public narrative.
3. Replace outdated `cargo nextest run --workspace` guidance in `docs/development.md` and `AGENTS.md` with the current CI/workspace checks.
4. Add the readiness review document to the repository as the cleanup baseline.

### Task 4: Add minimal public repository policy files

**Files:**
- Add: `SECURITY.md`
- Add: `CONTRIBUTING.md`

**Steps:**
1. Add a minimal `SECURITY.md` describing how to report security issues.
2. Add a minimal `CONTRIBUTING.md` covering setup, checks, and PR expectations.
3. Keep both documents short and consistent with current commands and docs.

### Task 5: Verify and commit

**Steps:**
1. Run:
   - `cargo fmt --all -- --check`
   - focused `cargo test` for `crates/prover/src/native.rs`
   - `git diff --check`
2. Manually review the changed docs for public-facing consistency.
3. Commit with a Conventional Commit message for the cleanup PR.
