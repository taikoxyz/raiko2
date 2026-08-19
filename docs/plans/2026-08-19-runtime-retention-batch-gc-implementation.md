# Runtime Retention Batch GC Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Bound authoritative runtime-state growth with configurable six-hour terminal retention and constant-write batch garbage collection.

**Architecture:** Keep the existing single-writer runtime authority and immutable proof object store. Add exact batch lifecycle transitions to `RuntimeManager`, drive them from the existing maintenance loop under the lifecycle gate, and expose metrics needed to validate the rollout before shortening retention further.

**Tech Stack:** Rust, Tokio, Serde/TOML, Prometheus, GCS generation CAS, existing raiko2 runtime and engine lifecycle APIs.

---

### Task 1: Add the retention configuration

**Files:**
- Modify: `bin/raiko2/src/config/runtime.rs`
- Modify: `config.example.toml`
- Modify: `docs/API.md`

**Step 1: Write failing configuration tests**

Add tests asserting that `RuntimeConfig::default().terminal_task_ttl_secs == 21_600`, TOML can
override it, and zero is rejected by `validate()`.

**Step 2: Run the focused tests and verify failure**

Run: `cargo test -p raiko2 config::runtime`

Expected: FAIL because the field does not exist.

**Step 3: Implement the configuration**

Add a Serde-defaulted `terminal_task_ttl_secs: u64`, validate that it is non-zero, and document the
six-hour terminal inactivity semantics.

**Step 4: Run the focused tests**

Run: `cargo test -p raiko2 config::runtime`

Expected: PASS.

**Step 5: Commit**

```bash
git add bin/raiko2/src/config/runtime.rs config.example.toml docs/API.md
git commit -m "feat(runtime): configure terminal task retention"
```

### Task 2: Add exact batch retirement and removal primitives

**Files:**
- Modify: `crates/runtime/src/lib.rs`

**Step 1: Write failing runtime tests**

Cover multiple exact terminal records retired in one authoritative mutation, stale or changed
records skipped, active artifacts retained while referenced by another live task, and newly unowned
artifacts marked invalidated.

**Step 2: Run the focused tests and verify failure**

Run: `cargo test -p raiko2-runtime --lib batch_retention`

Expected: FAIL because the batch APIs do not exist.

**Step 3: Implement minimal typed batch APIs**

Add typed prepare/finalize results. The prepare mutation retires unchanged terminal tasks and marks
only unowned artifacts invalidated. The finalize mutation removes exact retired task lifetimes,
removes only successfully finalized artifact descriptors, and releases pending publication owners.

**Step 4: Run focused runtime tests**

Run: `cargo test -p raiko2-runtime --lib batch_retention`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/runtime/src/lib.rs
git commit -m "feat(runtime): batch terminal retention transitions"
```

### Task 3: Drive batch GC from the maintenance loop

**Files:**
- Modify: `bin/raiko2/src/server/task_cleanup.rs`
- Modify: `bin/raiko2/src/server/lifecycle.rs`

**Step 1: Write failing cleanup tests**

Cover a multi-root cleanup pass, shared artifact retention, exact artifact finalization, detach
failure retry, proof-object deletion failure retry, pending-publication deletion retry, and a slow
object store that does not hold the execution lifecycle gate. Assert that cleanup performs a
constant number of authoritative state writes per batch using the runtime probe store.

**Step 2: Run the focused tests and verify failure**

Run: `cargo test -p raiko2 server::task_cleanup`

Expected: FAIL because cleanup still removes one root at a time.

**Step 3: Implement batch cleanup**

Pass `config.runtime.terminal_task_ttl_secs` into the loop. Hold the lifecycle gate only through the
batch prepare transition and exact queue detachment. Release it before bounded-concurrent artifact
and pending-publication finalization, then call the exact batch finalize transition. If the task seen
by a request disappears during this process, atomically retry normal registration instead of
returning an internal error.

**Step 4: Run focused cleanup tests**

Run: `cargo test -p raiko2 server::task_cleanup`

Expected: PASS.

**Step 5: Commit**

```bash
git add bin/raiko2/src/server/task_cleanup.rs bin/raiko2/src/server/lifecycle.rs
git commit -m "feat(runtime): garbage collect terminal tasks in batches"
```

### Task 4: Add runtime-state and cleanup observability

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `bin/raiko2/src/server/telemetry.rs`
- Modify: `bin/raiko2/src/server/task_cleanup.rs`

**Step 1: Write failing metric/state-stat tests**

Cover loaded and mutated serialized-byte length plus task, artifact, and pending-publication counts.
Cover cleanup counter labels without task IDs or other unbounded dimensions.

**Step 2: Run focused tests and verify failure**

Run: `cargo test -p raiko2-runtime --lib runtime_state_stats`

Run: `cargo test -p raiko2 server::telemetry`

Expected: FAIL because the stats and metrics do not exist.

**Step 3: Implement metrics**

Track the last installed serialized length in `RuntimeManager`, expose bounded state stats, and record
gauges/counters after initialization and cleanup passes.

**Step 4: Run focused tests**

Run the two focused commands from Step 2.

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/runtime/src/lib.rs bin/raiko2/src/server/telemetry.rs bin/raiko2/src/server/task_cleanup.rs
git commit -m "feat(metrics): observe runtime retention"
```

### Task 5: Verify the integrated change

**Files:**
- Verify only.

**Step 1: Format**

Run: `cargo fmt --all`

Expected: PASS with no remaining diff after a second run.

**Step 2: Run focused package tests**

Run: `cargo test -p raiko2-runtime --lib`

Run: `cargo test -p raiko2 server::task_cleanup`

Expected: PASS.

**Step 3: Run workspace lint**

Run: `cargo clippy --workspace -- -D warnings`

Expected: PASS.

**Step 4: Review configuration documentation**

Verify `config.example.toml` and `docs/API.md` describe the same default and terminal-time semantics.

**Step 5: Commit any verification-only fixes**

```bash
git add -u
git commit -m "chore(runtime): finalize retention rollout checks"
```
