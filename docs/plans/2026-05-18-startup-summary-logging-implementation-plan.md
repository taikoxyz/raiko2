# Startup Summary Logging Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add safe, route-aware startup summary logs for `raiko2` hosts without changing proving behavior.

**Architecture:** Build a small summary helper from `Config`, emit one structured startup summary
after config load, emit one readiness-passed log after `ensure_startup_ready`, and keep the existing
listening log as the final startup milestone.

**Tech Stack:** Rust, `tracing`, existing `raiko2` config types, focused unit tests

---

### Task 1: Add the failing startup summary tests

**Files:**
- Modify: `bin/raiko2/src/main.rs`

**Step 1: Write the failing test**

Add focused tests that build a sample `Config` and assert a startup summary contains:

- `listen`
- `route`
- at least one `pair`

Add a remote-SGX-specific test that asserts:

- `remote_sgx_base_url`
- `remote_sgx_sgxgeth_base_url`

are present when configured.

Add a safety test that asserts secret-like values are not exposed.

**Step 2: Run the focused tests to verify they fail**

Run:

```bash
cargo test -p raiko2 --bin raiko2 startup_summary -- --nocapture
```

Expected: fail because the summary builder does not exist yet.

### Task 2: Implement the minimal startup summary builder

**Files:**
- Modify: `bin/raiko2/src/main.rs`

**Step 1: Add a typed summary helper**

Implement a small serializable helper that derives safe startup fields from `Config` plus
`cli.json_logs`.

**Step 2: Emit the startup summary log**

Replace the weak `Loaded configuration: ...` log with a structured startup summary log that avoids
raw config dumping.

**Step 3: Re-run the focused tests**

Run:

```bash
cargo test -p raiko2 --bin raiko2 startup_summary -- --nocapture
```

Expected: pass.

### Task 3: Emit a startup readiness-passed milestone

**Files:**
- Modify: `bin/raiko2/src/server/run.rs`

**Step 1: Add the readiness-passed log**

Log a startup milestone immediately after `ensure_startup_ready(&config)` succeeds and before app
state construction/bind.

**Step 2: Keep the final listening log unchanged**

Do not replace the existing listening log; keep it as the final startup milestone.

### Task 4: Verify and commit

**Files:**
- Modify: `docs/plans/2026-05-18-startup-summary-logging-design.md`
- Modify: `docs/plans/2026-05-18-startup-summary-logging-implementation-plan.md`

**Step 1: Run focused verification**

Run:

```bash
cargo test -p raiko2 --bin raiko2 startup_summary -- --nocapture
cargo test -p raiko2 --bin raiko2 remote_sgx -- --nocapture
cargo fmt --all
git diff --check
```

**Step 2: Smoke-check the startup log shape**

Run a local startup with a remote-SGX-oriented config and confirm the log order is:

- startup summary
- startup readiness passed
- server listening

**Step 3: Commit**

Use a Conventional Commit message summarizing the startup logging improvement.
