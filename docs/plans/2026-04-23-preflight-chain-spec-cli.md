# Preflight Chain Spec CLI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make standalone `preflight` derive chain IDs and RPC URLs from chain specs while keeping proposal tuple inputs explicit.

**Architecture:** Add a small argument-resolution layer in `bin/preflight/src/main.rs` that converts raw CLI args plus `SupportedChainSpecs` into resolved L1/L2 chain specs, chain IDs, and RPC URLs. Keep the existing pipeline construction unchanged after resolution.

**Tech Stack:** Rust, clap, `SupportedChainSpecs`, `NetworkProvider`, existing `preflight` binary tests.

---

### Task 1: Add Resolution Helpers

**Files:**
- Modify: `bin/preflight/src/main.rs`

**Step 1: Add tests for chain/RPC resolution**

Create focused unit tests in `bin/preflight/src/main.rs` for:
- `--network taiko_hoodi --l1-network hoodi` resolves IDs and RPC URLs from chain specs.
- Explicit `--rpc-url` and `--l1-rpc-url` override chain spec RPC URLs.
- Explicit `--l2-chain-id` conflicting with `--network` fails.
- Empty chain spec RPC without override fails.

Run:

```bash
cargo test -p preflight
```

Expected: new tests initially fail.

**Step 2: Implement raw optional args**

Change these fields:

```rust
rpc_url: Option<String>
l1_rpc_url: Option<String>
l2_chain_id: Option<u64>
l1_chain_id: Option<u64>
network: Option<String>
l1_network: Option<String>
```

Keep `l1_chain_id` backward-compatible by resolving to `1` only when neither `--l1-network` nor `--l1-chain-id` is supplied.

**Step 3: Implement resolved config struct**

Add an internal struct like:

```rust
struct ResolvedPreflightConfig {
    l1_chain_spec: ChainSpec,
    l2_chain_spec: ChainSpec,
    l1_chain_id: u64,
    l2_chain_id: u64,
    l1_rpc_url: String,
    l2_rpc_url: String,
}
```

Add helper functions that resolve by network first, by chain ID second, and reject explicit ID conflicts.

**Step 4: Use resolved values in main**

Replace direct access to `args.rpc_url`, `args.l1_rpc_url`, `args.l1_chain_id`, and `args.l2_chain_id` with the resolved struct.

**Step 5: Verify**

Run:

```bash
cargo test -p preflight
cargo fmt --all -- --check
```

Expected: all tests pass and formatting is clean.

### Task 2: Document Regression Command Boundary

**Files:**
- Modify: `scripts/regression/README.md`

**Step 1: Add direct proposal example**

Document that direct single-proposal verification should use `preflight` with explicit proposal
tuple inputs and chain spec network selectors.

**Step 2: Preserve script boundary**

Do not update `scripts/regression/shasta_regression.py` in this change. It still has older
file-regression assumptions and should be cleaned up separately from the standalone preflight CLI
UX.

**Step 3: Verify**

Run:

```bash
cargo test -p preflight
```

Expected: tests pass.

### Task 3: Documentation

**Files:**
- Modify: `scripts/regression/README.md`
- Modify: `docs/plans/2026-04-23-preflight-chain-spec-cli-design.md`

**Step 1: Update command examples**

Show the simplified direct proposal flow:

```bash
cargo run -r -p preflight -- \
  --l1-network hoodi \
  --network taiko_hoodi \
  --proposal-id 17771 \
  --l1-inclusion-block-number 2674375 \
  --last-anchor-block-number 2674326 \
  --l2-start 7225402 \
  --l2-end 7225593 \
  --proof-type native \
  --validate \
  --output /tmp/proposal-17771.json
```

**Step 2: Verify**

Run the Rust checks from Task 1.

### Task 4: Commit

**Files:**
- All changed files.

**Step 1: Inspect status**

Run:

```bash
git status --short
git diff --stat
```

**Step 2: Commit**

Run:

```bash
git add bin/preflight/src/main.rs scripts/regression/README.md docs/plans/2026-04-23-preflight-chain-spec-cli-design.md docs/plans/2026-04-23-preflight-chain-spec-cli.md
git commit -m "feat: derive preflight chain settings from specs"
```
