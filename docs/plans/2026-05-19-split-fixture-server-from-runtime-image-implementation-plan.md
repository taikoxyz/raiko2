# Split Fixture Server From Runtime Image Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove fixture-server code and `tests/fixtures/` from the default `raiko2` runtime image build path while preserving the local fixture-backed harness behind an explicit feature.

**Architecture:** Add a non-default `fixture-server` feature to `bin/raiko2`, gate the CLI/module wiring behind it, and remove `tests/fixtures` from the default Docker build context. Keep the local harness available through `cargo run -p raiko2 --features fixture-server -- fixture-server ...`.

**Tech Stack:** Rust, clap, Cargo features, Docker multi-stage build, repo docs.

---

### Task 1: Add failing tests for fixture-server CLI gating

**Files:**
- Modify: `bin/raiko2/src/cli.rs`

**Step 1: Write the failing tests**

Add:

- a default-build test asserting `fixture-server` is rejected when the feature is disabled
- a feature-gated test asserting `fixture-server` parses successfully when the feature is enabled

**Step 2: Run test to verify the expected behavior**

Run:

```bash
cargo test -p raiko2 fixture_server_command_is_rejected_without_feature -- --nocapture
cargo test -p raiko2 --features fixture-server fixture_server_command_parses_with_feature -- --nocapture
```

Expected:

- default path passes only after the feature gating is implemented
- feature-enabled path initially fails until the gated command is wired correctly

**Step 3: Commit**

```bash
git add bin/raiko2/src/cli.rs
git commit -m "test: cover fixture server cli gating"
```

### Task 2: Gate fixture-server code behind a non-default feature

**Files:**
- Modify: `bin/raiko2/Cargo.toml`
- Modify: `bin/raiko2/src/cli.rs`
- Modify: `bin/raiko2/src/main.rs`
- Modify: `bin/raiko2/src/server/mod.rs`

**Step 1: Add the feature**

Add:

```toml
[features]
fixture-server = []
redis-queue = ["raiko2-queue/redis"]
```

**Step 2: Gate CLI and server wiring**

Add `#[cfg(feature = "fixture-server")]` to:

- `Command::FixtureServer`
- `FixtureServerArgs`
- `use crate::server::run_fixture_server`
- the `run_fixture_server(...)` dispatch branch
- `mod fixture`
- `pub use fixture::run_fixture_server`

**Step 3: Run focused tests**

Run:

```bash
cargo test -p raiko2 fixture_server_command_is_rejected_without_feature -- --nocapture
cargo test -p raiko2 --features fixture-server fixture_server_command_parses_with_feature -- --nocapture
```

Expected:

- both tests pass

**Step 4: Commit**

```bash
git add bin/raiko2/Cargo.toml bin/raiko2/src/cli.rs bin/raiko2/src/main.rs bin/raiko2/src/server/mod.rs
git commit -m "fix: gate fixture server behind non-default feature"
```

### Task 3: Remove fixture assets from the default runtime image build path

**Files:**
- Modify: `Dockerfile`

**Step 1: Remove fixture copies**

Delete:

- `COPY tests/fixtures ./tests/fixtures` from the planner stage
- `COPY tests/fixtures ./tests/fixtures` from the builder stage

**Step 2: Verify default build path no longer needs fixtures**

Run:

```bash
cargo check -p raiko2
```

Expected:

- default `raiko2` still checks successfully without enabling `fixture-server`

**Step 3: Commit**

```bash
git add Dockerfile
git commit -m "fix: remove fixture assets from default image build"
```

### Task 4: Update developer-facing docs and commands

**Files:**
- Modify: `README.md`
- Modify: `docs/development.md`

**Step 1: Update fixture-server command examples**

Change:

```bash
cargo run -p raiko2 -- fixture-server ...
```

to:

```bash
cargo run -p raiko2 --features fixture-server -- fixture-server ...
```

**Step 2: Verify no other direct fixture-server command references remain**

Run:

```bash
rg -n "cargo run -p raiko2 -- fixture-server|fixture-server --host" README.md docs
```

Expected:

- no stale command examples remain

**Step 3: Commit**

```bash
git add README.md docs/development.md
git commit -m "docs: document fixture server feature gate"
```

### Task 5: Run final verification

**Files:**
- Verify only

**Step 1: Run focused verification**

Run:

```bash
cargo test -p raiko2 fixture_server_command_is_rejected_without_feature -- --nocapture
cargo test -p raiko2 --features fixture-server fixture_server_command_parses_with_feature -- --nocapture
cargo check -p raiko2
cargo check -p raiko2 --features fixture-server
cargo fmt --all --check
git diff --check
```

Expected:

- all commands pass

**Step 2: Commit**

```bash
git add .
git commit -m "chore: verify fixture server runtime split"
```
