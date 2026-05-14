# TEE Provider Release Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `raiko2`-owned TEE provider release flow that builds pinned provider images,
captures pushed image digests plus attestation measurements, and writes one release manifest.

**Architecture:** Keep provider pinning in a checked-in `release/providers.toml`, add focused `xtask`
commands for lock updates and TEE provider release/export, and extend the public operations docs so
the TEE flow sits beside the existing runtime image and zk guest-digest release flows.

**Tech Stack:** Rust `xtask`, TOML parsing with `serde`, Docker CLI orchestration, JSON manifest
serialization, Markdown docs

---

### Task 1: Define the checked-in provider lock file

**Files:**
- Create: `release/providers.toml`

**Step 1: Add the first external provider entry**

Create a TOML file with a `providers.gaiko2` entry containing:

- `repo`
- `commit`
- `provider`
- `lane`
- `image_name`
- `repository`
- `dockerfile`
- `context`
- `attestation_path`

Use the current known `gaiko2` release inputs and keep the file scoped to external TEE providers.

**Step 2: Add inline comments for stable expectations**

Document in comments that:

- commits must be exact release inputs
- branch tips must not be used at release time
- `attestation_path` must point at a baked metadata file inside the built image

**Step 3: Review file shape**

Confirm the file reads cleanly as a checked-in lock artifact and is not mixed with runtime config.

### Task 2: Add typed provider lock parsing

**Files:**
- Create: `xtask/src/tee_provider_lock.rs`
- Modify: `xtask/Cargo.toml`

**Step 1: Write the failing parser test**

Add focused unit tests that load a small TOML snippet and assert:

- provider names deserialize correctly
- required fields are present
- missing required fields are rejected

Run:

```bash
cargo test -p xtask tee_provider_lock
```

Expected: fail until the parser types exist.

**Step 2: Implement minimal parsing types**

Add:

- top-level lock struct
- provider entry struct
- file loader helper

Use `serde` + `toml` and return `anyhow::Result`.

**Step 3: Re-run the targeted tests**

Run:

```bash
cargo test -p xtask tee_provider_lock
```

Expected: pass.

### Task 3: Add explicit lock update command

**Files:**
- Create: `xtask/src/update_tee_provider_lock.rs`
- Modify: `xtask/src/main.rs`
- Test: `xtask/src/update_tee_provider_lock.rs`

**Step 1: Write the failing update test**

Add a temp-file based unit test that:

- writes a small `providers.toml`
- runs the update helper against one provider
- asserts only the targeted `commit` field changes

Run:

```bash
cargo test -p xtask update_tee_provider_lock
```

Expected: fail until the updater exists.

**Step 2: Implement the minimal updater**

Add a command surface:

```bash
cargo run -r -p xtask -- update-tee-provider-lock gaiko2 --commit <sha>
```

Update only the named provider entry and preserve deterministic file formatting as much as
practical.

**Step 3: Re-run targeted tests**

Run:

```bash
cargo test -p xtask update_tee_provider_lock
```

Expected: pass.

### Task 4: Add TEE attestation manifest model

**Files:**
- Create: `xtask/src/release_tee_manifest.rs`
- Test: `xtask/src/release_tee_manifest.rs`

**Step 1: Write serialization tests**

Add a test that constructs a manifest in memory and asserts:

- top-level keys exist
- provider entries include `lane`, `provider`, `source`, `image`, and `attestation`
- JSON output is deterministic and ends with a newline

Run:

```bash
cargo test -p xtask release_tee_manifest
```

Expected: fail until the model exists.

**Step 2: Implement manifest structs and writer**

Add:

- typed manifest structs
- deterministic JSON writer helper
- simple path helper for `target/releases/<tag>/tee-attestation-manifest-<tag>.json`

**Step 3: Re-run targeted tests**

Run:

```bash
cargo test -p xtask release_tee_manifest
```

Expected: pass.

### Task 5: Add local and external provider build helpers

**Files:**
- Create: `xtask/src/release_tee_providers.rs`
- Modify: `xtask/src/util.rs`
- Test: `xtask/src/release_tee_providers.rs`

**Step 1: Write focused helper tests**

Add unit tests for pure helpers that:

- build local provider image refs from tag/repository inputs
- construct temporary external source checkout paths
- validate required attestation path strings

Run:

```bash
cargo test -p xtask release_tee_providers
```

Expected: fail until helpers exist.

**Step 2: Implement provider build metadata helpers**

Add helpers for:

- local `raiko2-sgx` provider config
- external provider source checkout metadata
- Docker image ref assembly
- validation of required metadata fields

Keep actual subprocess orchestration separate from pure helper logic where possible.

**Step 3: Re-run targeted tests**

Run:

```bash
cargo test -p xtask release_tee_providers
```

Expected: partial pass for helper coverage.

### Task 6: Implement the `release-tee-providers` command

**Files:**
- Modify: `xtask/src/release_tee_providers.rs`
- Modify: `xtask/src/main.rs`
- Modify: `xtask/Cargo.toml`

**Step 1: Add the CLI surface**

Expose:

```bash
cargo run -r -p xtask -- release-tee-providers --tag <tag>
```

Add optional:

```bash
--no-push
```

Reject invalid combinations early.

**Step 2: Implement release orchestration**

Sequence the command to:

1. ensure Docker/buildx is available
2. ensure the local worktree is clean
3. load `release/providers.toml`
4. build local `raiko2-sgx`
5. optionally push it and resolve the pushed digest
6. clone each external provider into `target/tee-release/<tag>/sources/<provider>/`
7. check out the pinned commit
8. build the external provider image
9. optionally push it and resolve the pushed digest
10. read attestation metadata from each image
11. emit the unified manifest only after all entries validate

**Step 3: Add fail-fast behavior**

Return an error immediately if any provider fails to:

- clone
- build
- push
- resolve a digest
- provide readable attestation metadata

### Task 7: Add command-level tests for manifest generation

**Files:**
- Modify: `xtask/src/release_tee_providers.rs`

**Step 1: Add tempdir-based tests for output paths and validation**

Test:

- manifest output path generation
- `--no-push` command-path validation
- missing attestation field rejection

Avoid tests that require real Docker pushes; keep command-level tests focused on deterministic
helper and writer behavior.

**Step 2: Run the focused command tests**

Run:

```bash
cargo test -p xtask release_tee_providers
```

Expected: pass.

### Task 8: Document the public TEE pre-release flow

**Files:**
- Modify: `docs/operations.md`
- Modify: `.codex/skills/raiko2-image-release/SKILL.md`

**Step 1: Add a `TEE Provider Release Metadata` section**

Document:

- when to run `release-tee-providers`
- the role of `release/providers.toml`
- why this is separate from zk `guest-digests`
- where the final manifest is written

**Step 2: Add exact commands**

Include examples for:

- local smoke:

```bash
cargo run -r -p xtask -- release-tee-providers --tag <tag> --no-push
```

- formal release:

```bash
cargo run -r -p xtask -- release-tee-providers --tag <tag>
```

**Step 3: Update the image-release skill boundary**

Clarify in the skill that:

- runtime image release and zk digest capture remain separate
- TEE provider release metadata now has its own explicit `xtask` command

### Task 9: Validate the new flow

**Files:**
- Verify: `release/providers.toml`
- Verify: `xtask/src/*`
- Verify: `docs/operations.md`
- Verify: `.codex/skills/raiko2-image-release/SKILL.md`

**Step 1: Run formatting and diff hygiene**

Run:

```bash
cargo fmt --all
git diff --check
```

Expected: pass.

**Step 2: Run focused `xtask` tests**

Run:

```bash
cargo test -p xtask tee_provider_lock
cargo test -p xtask update_tee_provider_lock
cargo test -p xtask release_tee_manifest
cargo test -p xtask release_tee_providers
```

Expected: pass.

**Step 3: Run a local smoke release**

Run:

```bash
cargo run -r -p xtask -- release-tee-providers --tag local-smoke --no-push
```

Expected:

- local provider image builds
- external provider pin is cloned and built
- manifest file is written under `target/releases/local-smoke/`

### Task 10: Finalize the branch

**Files:**
- Stage all files from Tasks 1-9

**Step 1: Review final diff**

Confirm:

- exact provider pinning lives in `release/providers.toml`
- release orchestration is in `xtask`
- docs treat TEE release metadata as a peer to zk guest digests

**Step 2: Commit**

Suggested commit:

```bash
git commit -m "feat: add tee provider release manifest flow"
```

**Step 3: Push branch**

Push the feature branch and summarize:

- provider lock file
- release command
- manifest output
- public release docs
