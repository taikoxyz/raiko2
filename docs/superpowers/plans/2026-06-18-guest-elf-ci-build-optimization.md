# Guest-ELF-Consistency CI Build Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the `guest-elf-consistency` CI job pass and run far faster (~28 min failing → warm ~6–8 min) by removing a duplicate debug compile, feature-gating the prover SDKs out of the guest builder, and adding a guarded disk safety net — without changing any guest ELF bytes or the digest summary output.

**Architecture:** Three independent changes. (1) The CI digest step runs in **release** with a new `digests` feature so it reuses the builder step's artifacts instead of recompiling the heavy SDK tree in debug (which also caused the disk OOM). (2) In `xtask-build-guest`, `risc0-zkvm`/`sp1-sdk`/`raiko2-guests`/`alloy-primitives` and the `guest_digests` module move behind a non-default `digests` feature, so `just build-guest` (the builder) no longer compiles them. (3) The self-hosted runner's disk-free step gains a guarded cargo-cache-volume eviction that only fires under disk pressure. Cache-warming is automatic once the job stops failing (the `Post rust-cache` save then runs).

**Tech Stack:** Rust (Cargo workspace, Cargo features, `cargo tree`), GitHub Actions YAML, bash, Docker named volumes.

**Spec:** `docs/superpowers/specs/2026-06-18-guest-elf-ci-build-optimization-design.md`

**Landing constraint:** Task 1 and Task 2 must ship on the **same branch** and land together. Task 1 adds `required-features = ["digests"]` to the `guest-digests` bin, which makes the *current* CI digest step (no `--features digests`) fail to build — Task 2 updates that step to match. The local workspace stays green after Task 1 (only the CI YAML is stale), so **do not trigger CI between Task 1 and Task 2**; the end-to-end CI run happens in Task 4 with both in place. Tasks 3 and 4 are otherwise independent.

---

### Task 1: Feature-gate the digest tooling and update the `xtask` consumer (WS2)

Move the prover-SDK dependencies and the `guest_digests` module behind a non-default `digests` feature in `xtask-build-guest`, and enable that feature from the `xtask` umbrella crate's `guest-tools` feature so its `GuestDigests` subcommand still compiles. This makes the builder binary stop compiling `sp1-sdk`/`risc0-zkvm`.

**Files:**
- Modify: `xtask/build-guest/Cargo.toml`
- Modify: `xtask/build-guest/src/lib.rs:13`
- Modify: `xtask/Cargo.toml` (the `guest-tools` feature list)

- [ ] **Step 1: Capture the baseline (the problem state)**

Run:
```bash
cargo tree -p xtask-build-guest 2>/dev/null | grep -E 'risc0-zkvm|sp1-sdk|raiko2-guests' || echo "NONE"
```
Expected: lines for `risc0-zkvm`, `sp1-sdk`, and `raiko2-guests` are printed (they are currently pulled into the builder). This is what we are removing.

- [ ] **Step 2: Make the heavy deps optional and add the `digests` feature + bin guard**

Edit `xtask/build-guest/Cargo.toml`. Replace the `[dependencies]` block and append a `[features]` and `[[bin]]` section so the file's dependency/feature/bin sections read exactly:

```toml
[dependencies]
alloy-primitives = { workspace = true, optional = true }
anyhow = { workspace = true }
clap = { workspace = true }
raiko2-guests = { workspace = true, optional = true }
risc0-binfmt = "=3.0.4"
risc0-zkvm = { workspace = true, optional = true }
risc0-zkos-v1compat = "=2.2.2"
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
sha2 = { workspace = true }
sp1-sdk = { workspace = true, optional = true }
toml = "0.9.8"

[features]
digests = ["dep:alloy-primitives", "dep:raiko2-guests", "dep:risc0-zkvm", "dep:sp1-sdk"]

[[bin]]
name = "guest-digests"
path = "src/bin/guest-digests.rs"
required-features = ["digests"]
```

Leave the existing `[package]` section (including `default-run = "xtask-build-guest"`) unchanged. Do **not** set `autobins = false` — the `xtask-build-guest` binary at `src/main.rs` stays auto-discovered.

- [ ] **Step 3: Gate the `guest_digests` module**

Edit `xtask/build-guest/src/lib.rs:13`. Change:
```rust
pub mod guest_digests;
mod util;
```
to:
```rust
#[cfg(feature = "digests")]
pub mod guest_digests;
mod util;
```
Leave `mod util;` ungated — the builder's non-digest code (e.g. `guest_fingerprint_path`, the Docker cargo-cache-volume helpers) uses it on every build.

- [ ] **Step 4: Verify the builder no longer pulls the heavy deps**

Run:
```bash
cargo tree -p xtask-build-guest 2>/dev/null | grep -E 'risc0-zkvm|sp1-sdk|raiko2-guests' || echo "NONE (good)"
```
Expected: `NONE (good)`.

- [ ] **Step 5: Verify the builder binary still compiles (light path)**

Run:
```bash
cargo build -p xtask-build-guest --bin xtask-build-guest
```
Expected: PASS. This compiles only the builder's light deps (`clap`, `serde`, `risc0-binfmt`, `risc0-zkos-v1compat`, `sha2`, `toml`, …) — no `sp1-sdk`/`risc0-zkvm`.

- [ ] **Step 6: Wire the `digests` feature into the `xtask` umbrella crate**

Edit `xtask/Cargo.toml`. In the `[features]` section, change the `guest-tools` list from:
```toml
guest-tools = [
    "dep:raiko2-guests",
    "dep:risc0-zkvm",
    "dep:sp1-sdk",
    "dep:xtask-build-guest",
]
```
to:
```toml
guest-tools = [
    "dep:raiko2-guests",
    "dep:risc0-zkvm",
    "dep:sp1-sdk",
    "dep:xtask-build-guest",
    "xtask-build-guest/digests",
]
```
This makes the gated `guest_digests` module available wherever `xtask` uses it (`xtask/src/main.rs:84`, the `GuestDigests` subcommand).

- [ ] **Step 7: Verify the `xtask` crate type-checks with default features**

Run:
```bash
cargo check -p xtask
```
Expected: PASS. (Without Step 6 this fails with an error that `xtask_build_guest::guest_digests` does not exist.) Note: this compiles the heavy SDK deps for `xtask` and may take several minutes cold.

- [ ] **Step 8: Verify the gated digest binary still type-checks**

Run:
```bash
cargo check -p xtask-build-guest --bin guest-digests --features digests
```
Expected: PASS. (Heavy/slow on a cold cache — it compiles `sp1-sdk`/`risc0-zkvm`.)

- [ ] **Step 9: Run the digest-tool unit tests behind the feature**

Run:
```bash
cargo test -p xtask-build-guest --features digests guest_digests
```
Expected: PASS (`guest_digests_cover_expected_objects_and_sources`, `guest_digests_run_writes_json_summary`). These tests live in `xtask/build-guest/src/guest_digests.rs` and only exist with the `digests` feature.

- [ ] **Step 10: Commit**

```bash
git add xtask/build-guest/Cargo.toml xtask/build-guest/src/lib.rs xtask/Cargo.toml
git commit -m "build(xtask): feature-gate guest-digests prover SDKs

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Run the CI digest step in release with the `digests` feature (WS1)

Switch the `export guest digests` step to `cargo run -r … --features digests` so it reuses the release artifacts already built by the `rebuild guest elfs` step, instead of building a second debug copy of the SDK tree (the ~12-min waste and the source of the `No space left on device` failure).

**Files:**
- Modify: `.github/workflows/ci.yml:342`

- [ ] **Step 1: Edit the digest step**

In `.github/workflows/ci.yml`, find the step:
```yaml
      - name: export guest digests
        run: cargo run -p xtask-build-guest --bin guest-digests -- --output target/guest-digests/summary.json
```
Change the `run:` line to:
```yaml
      - name: export guest digests
        run: cargo run -r -p xtask-build-guest --bin guest-digests --features digests -- --output target/guest-digests/summary.json
```
(Only `-r` and `--features digests` are added; the rest is identical.)

- [ ] **Step 2: Verify the exact command works locally against the checked-in ELFs**

The digest tool reads the checked-in ELFs under `crates/guests/elf` — it does not need a fresh guest build. Run the exact command from the step:
```bash
cargo run -r -p xtask-build-guest --bin guest-digests --features digests -- --output target/guest-digests/summary.json
```
Expected: ends with `Wrote guest digest summary to …/target/guest-digests/summary.json`. (Heavy/slow on a cold cache.)

- [ ] **Step 3: Verify the summary content is well-formed and unchanged in shape**

Run:
```bash
python3 -c "import json; d=json.load(open('target/guest-digests/summary.json')); print(d['guest_elf_dir']); print(len(d['digests']), 'entries'); print(sorted({e['proof_system'] for e in d['digests']}))"
```
Expected: prints the guest ELF dir, a non-zero entry count, and `['risc0', 'sp1']` — confirming the release build produces the same summary structure as before.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: run guest digests in release with digests feature

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Guard the self-hosted runner disk with a cargo-cache eviction (WS3)

Add a last-resort, threshold-gated prune of the per-repo cargo cache volumes to the `free self-hosted runner disk` step, so a near-full runner self-heals instead of OOMing. The volumes are preserved in the common case (they keep guest compiles at ~2 min).

**Files:**
- Modify: `.github/workflows/ci.yml` (the `free self-hosted runner disk` step, around lines 268–287)

- [ ] **Step 1: Insert the guarded eviction block**

In `.github/workflows/ci.yml`, locate the `free self-hosted runner disk` step. Insert the following block immediately **before** the step's final `df -h` line (after the existing `_diag` log-cleanup `if` block):

```bash
          # Guarded last-resort cache eviction: only when critically low on
          # disk, drop the per-repo cargo cache volumes so the runner
          # self-heals instead of OOMing mid-build. These volumes normally
          # persist to keep guest compiles fast (~2 min); sacrifice them only
          # under disk pressure.
          if command -v docker >/dev/null 2>&1; then
            threshold_kb=$((20 * 1024 * 1024))  # 20 GiB
            avail_kb="$(df -Pk "${GITHUB_WORKSPACE:-$PWD}" | awk 'NR==2 {print $4}')"
            if [ -n "${avail_kb:-}" ] && [ "${avail_kb}" -lt "${threshold_kb}" ]; then
              echo "Low disk: ${avail_kb} KiB available (< ${threshold_kb} KiB); pruning cargo cache volumes"
              docker volume ls --quiet --filter 'name=-cargo-' | xargs -r docker volume rm || true
            fi
          fi
```

The `name=-cargo-` filter matches the volumes named `<repo>-cargo-risc0` and `<repo>-cargo-sp1` created in `xtask/build-guest/src/util.rs`. `set -euo pipefail` is already active for this step; `xargs -r` skips the `rm` when no volume matches, and `|| true` keeps the step non-fatal.

- [ ] **Step 2: Syntax-check the resulting step script**

The heredoc below is the expected post-edit script body for the step — run it to confirm it parses cleanly:
```bash
bash -n <<'EOF'
set -euo pipefail
df -h
if command -v docker >/dev/null 2>&1; then
  docker container prune -f || true
  docker image prune -f || true
  docker builder prune -af || true
fi
if [ -n "${RUNNER_TEMP:-}" ]; then
  runner_root="$(dirname "$(dirname "${RUNNER_TEMP}")")"
  if [ -d "${runner_root}/_diag" ]; then
    find "${runner_root}/_diag" -type f -name '*.log' -mtime +1 -delete || true
  fi
fi
if command -v docker >/dev/null 2>&1; then
  threshold_kb=$((20 * 1024 * 1024))
  avail_kb="$(df -Pk "${GITHUB_WORKSPACE:-$PWD}" | awk 'NR==2 {print $4}')"
  if [ -n "${avail_kb:-}" ] && [ "${avail_kb}" -lt "${threshold_kb}" ]; then
    echo "Low disk: ${avail_kb} KiB available (< ${threshold_kb} KiB); pruning cargo cache volumes"
    docker volume ls --quiet --filter 'name=-cargo-' | xargs -r docker volume rm || true
  fi
fi
df -h
EOF
echo "exit: $?"
```
Expected: `exit: 0` (no syntax errors). Full behavior is exercised only on the self-hosted runner in Task 4.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: guard self-hosted disk with cargo cache eviction

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: End-to-end CI validation

Trigger the real `guest-elf-consistency` job and confirm it passes faster with no ELF drift. Cache-warming needs no code change — once this run succeeds, the `Post rust-cache` save persists the SDK artifacts under `shared-key: ci-zk-build`, so the *next* run is the warm one.

**Files:** none (validation only).

- [ ] **Step 1: Push the branch**

```bash
git push -u origin "$(git branch --show-current)"
```

- [ ] **Step 2: Trigger the ci workflow on the branch**

```bash
gh workflow run ci.yml --ref "$(git branch --show-current)"
```
(The `guest-elf-consistency` job runs on `workflow_dispatch`. Alternatively, open a same-repo PR, which also triggers it.)

- [ ] **Step 3: Watch the run and confirm the job passes**

```bash
gh run list --workflow=ci.yml --branch "$(git branch --show-current)" --limit 1
gh run watch "$(gh run list --workflow=ci.yml --branch "$(git branch --show-current)" --limit 1 --json databaseId --jq '.[0].databaseId')"
```
Expected: the `guest-elf-consistency` job concludes **success**.

- [ ] **Step 4: Confirm the per-step timing improved and no ELF drift**

Find the job id and inspect step durations:
```bash
JOB_ID="$(gh run view "$(gh run list --workflow=ci.yml --branch "$(git branch --show-current)" --limit 1 --json databaseId --jq '.[0].databaseId')" --json jobs --jq '.jobs[] | select(.name=="guest-elf-consistency") | .databaseId')"
gh api "repos/{owner}/{repo}/actions/jobs/${JOB_ID}" --jq '.steps[] | {name: .name, started: .started_at, completed: .completed_at, conclusion: .conclusion}'
```
Expected:
- `export guest digests` is now seconds, not ~12 min, and **does not fail** on disk.
- `rebuild guest elfs` no longer spends ~11 min on a host SDK compile (builder path is light).
- The `verify guest elf consistency` step is **success** (no drift in `crates/guests/elf`).
- The `upload guest digest summary` / `publish guest digest summary` steps ran (summary still produced).

- [ ] **Step 5: Confirm warm-cache speedup on a second run (optional but recommended)**

Re-trigger the same workflow on the branch and confirm the second run is materially faster than the first (the SDK dependency artifacts are now restored from `ci-zk-build`):
```bash
gh workflow run ci.yml --ref "$(git branch --show-current)"
```
Expected: the second `guest-elf-consistency` run lands in the warm target range (~6–8 min) versus ~28 min before this work.

---

## Notes for the implementer

- **Why no change to the `rebuild guest elfs` step:** it invokes `just build-guest` → `cargo run -r -p xtask-build-guest --bin xtask-build-guest`, which builds the package **without** the `digests` feature. After Task 1 that path no longer compiles the prover SDKs — no flag change needed there.
- **`xtask` direct heavy deps left in place:** trimming `xtask`'s own `risc0-zkvm`/`sp1-sdk`/`raiko2-guests` is out of scope (`xtask` is not on the CI hot path); leave them.
- **If Step 4 of Task 1 still shows heavy deps:** check whether `risc0-binfmt` or `risc0-zkos-v1compat` unexpectedly pull `risc0-zkvm` (`cargo tree -p xtask-build-guest -i risc0-zkvm`). If so, gate those two behind `digests` as well and adjust `export_risc0_binary` accordingly — but the import analysis in the spec indicates they are independent, lightweight format crates.
- **Rollback:** each task is a single revertible commit.
