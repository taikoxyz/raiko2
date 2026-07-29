# Runtime Namespace Reset Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an operator-controlled startup reset that clears all persistence for the configured runtime namespace before Raiko2 starts recovery or serving traffic.

**Architecture:** Add one namespace-reset operation to the runtime-store abstraction. The Memory implementation clears its complete in-process store; the GCS implementation lists only the encoded scope prefix and conditionally deletes every listed object. `AppState::new` calls the runtime reset before initialization when the new configuration flag is enabled.

**Tech Stack:** Rust, Tokio, `google-cloud-storage`, Serde TOML configuration, existing runtime store test seams.

---

### Task 1: Define and test the store reset contract

**Files:**
- Modify: `crates/runtime/src/artifact_store.rs`
- Modify: `crates/runtime/src/artifact_store/gcs.rs`
- Modify: `crates/runtime/src/artifact_store/gcs_tests.rs`
- Test: `crates/runtime/src/artifact_store.rs`
- Test: `crates/runtime/src/artifact_store/gcs_tests.rs`

**Step 1: Write the failing tests**

Add one Memory-store test proving that a reset removes runtime state, an active manifest/content pair, and invalidation state. Add one GCS seam test proving that reset deletes all objects under the exact scope prefix while retaining a sibling namespace. Add a GCS seam failure test that verifies a failed deletion returns an error and leaves startup work to retry on a later process start.

**Step 2: Run the focused tests to verify they fail**

Run: `cargo test -p raiko2-runtime namespace_reset`

Expected: FAIL because the runtime store has no namespace-reset operation.

**Step 3: Add the minimal store contract and implementations**

Extend the combined runtime-store trait with an async `reset_namespace` method. Clear all Memory-store maps and reset its generation counter. Extend the GCS transport seam with prefix listing, paginate `StorageControl::list_objects`, and conditionally delete every object returned under `scope_prefix() + "/"`. Treat a conditional deletion conflict as an error because a second writer would violate the namespace non-overlap invariant.

**Step 4: Run the focused tests to verify they pass**

Run: `cargo test -p raiko2-runtime namespace_reset`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/runtime/src/artifact_store.rs crates/runtime/src/artifact_store/gcs.rs crates/runtime/src/artifact_store/gcs_tests.rs
git commit -m "feat(runtime): add namespace reset store operation"
```

### Task 2: Wire reset into startup configuration

**Files:**
- Modify: `bin/raiko2/src/config/runtime.rs`
- Modify: `bin/raiko2/src/server/state/mod.rs`
- Test: `bin/raiko2/src/config/runtime.rs`
- Test: `bin/raiko2/src/server/state/mod.rs`

**Step 1: Write the failing tests**

Add a config test for the default `false` flag. Add an AppState startup test using a reusable test store: with reset disabled, existing state follows normal recovery; with reset enabled, the pre-existing task is absent before recovery and no worker-recovery submission is made.

**Step 2: Run the focused tests to verify they fail**

Run: `cargo test -p raiko2 reset_namespace_on_start`

Expected: FAIL because the configuration field and startup call do not exist.

**Step 3: Add the minimal configuration and startup wiring**

Add `RuntimeConfig::reset_namespace_on_start` with Serde default `false`. Immediately after building the runtime store and before `initialize`, call the reset when the flag is enabled. Log only backend, environment, namespace, and deleted-object count; never expose configuration secrets.

**Step 4: Run the focused tests to verify they pass**

Run: `cargo test -p raiko2 reset_namespace_on_start`

Expected: PASS.

**Step 5: Commit**

```bash
git add bin/raiko2/src/config/runtime.rs bin/raiko2/src/server/state/mod.rs
git commit -m "feat(server): reset configured runtime namespace at startup"
```

### Task 3: Document the destructive operator action

**Files:**
- Modify: `config.example.toml`
- Modify: `docs/API.md`
- Modify: `README.md`

**Step 1: Document the default and procedure**

Show the disabled default in the example configuration. Document that setting the flag removes all persisted state and proof objects for the current namespace before startup, requires a non-overlapping replacement, fails startup on any cleanup error, and must be manually turned off after the reset succeeds.

**Step 2: Verify documentation references**

Run: `rg -n "reset_namespace_on_start" README.md docs/API.md config.example.toml`

Expected: all three operator sources describe the same field and behavior.

**Step 3: Commit**

```bash
git add README.md docs/API.md config.example.toml
git commit -m "docs: describe runtime namespace reset"
```

### Task 4: Verify the complete change

**Files:**
- Verify: all modified files

**Step 1: Format**

Run: `cargo fmt --all -- --check`

Expected: PASS.

**Step 2: Run focused package tests**

Run: `cargo test -p raiko2-runtime`

Expected: PASS.

Run: `cargo test -p raiko2 reset_namespace_on_start`

Expected: PASS.

**Step 3: Run static checks**

Run: `cargo clippy -p raiko2-runtime -p raiko2 --all-targets -- -D warnings`

Expected: PASS.

**Step 4: Review the diff for portability and scope**

Run: `git diff --check origin/main...HEAD`

Expected: PASS with no absolute paths, secret values, or unrelated changes.

**Step 5: Commit any final fixes**

```bash
git add -A
git commit -m "test: cover runtime namespace reset startup"
```

