# Shasta Regression Discovery Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a main-targeted Shasta regression flow that discovers a proposal tuple from one L2 block, runs `preflight`, and runs native verification.

**Architecture:** Reuse the existing Python stress discovery logic as the discovery source of truth. Add chain spec resolution and discover-only JSON output to the stress script, then document the deterministic agent workflow in a repo-local skill.

**Tech Stack:** Python `argparse`/`unittest`, existing chain spec JSON, Rust `preflight`, Rust `guest-launcher`, Codex repo-local skills.

---

### Task 1: Bring Stress Discovery Files Into The Main-Targeted Branch

**Files:**
- Create: `scripts/regression/stress_shasta_proposal.py`
- Create: `scripts/regression/shasta_event_decoder.py`
- Create: `scripts/regression/shasta/IInbox.json`
- Create: `scripts/regression/shasta/Anchor.json`
- Create: `scripts/regression/tests/test_stress_shasta_proposal.py`

**Steps:**
1. Copy the files from `enable-gaiko2`.
2. Keep ABI files under `scripts/regression/shasta/`.
3. Run the copied stress tests and record the starting behavior.

### Task 2: Add Chain Spec Resolution Tests

**Files:**
- Modify: `scripts/regression/tests/test_stress_shasta_proposal.py`

**Steps:**
1. Add tests for resolving `taiko_hoodi` and `hoodi` from `config/chain_spec_list_default.json`.
2. Add tests that explicit RPC/contract/ABI overrides win over chain spec defaults.
3. Run the tests and verify they fail before implementation.

### Task 3: Implement Chain Spec Resolution

**Files:**
- Modify: `scripts/regression/stress_shasta_proposal.py`

**Steps:**
1. Add helpers to load chain specs, resolve network specs, resolve RPC URLs, and resolve Shasta contract address.
2. Add CLI args: `--chain-spec-list`, `--network`, and `--l1-network`.
3. Default ABI paths to `scripts/regression/shasta/IInbox.json` and `scripts/regression/shasta/Anchor.json`.
4. Keep existing `-e/-l/-c/-i/-b` args as overrides.
5. Run stress tests and verify they pass.

### Task 4: Add Discover-Only Output

**Files:**
- Modify: `scripts/regression/stress_shasta_proposal.py`
- Modify: `scripts/regression/tests/test_stress_shasta_proposal.py`

**Steps:**
1. Add pure helper tests for the proposal tuple JSON shape.
2. Add CLI args `--discover-only` and `--proposal-out`.
3. In range mode, emit discovered proposal tuples and return without submitting to `raiko2`.
4. Run tests.

### Task 5: Add Regression Skill

**Files:**
- Create: `.codex/skills/shasta-proposal-regression/SKILL.md`

**Steps:**
1. Document required inputs: `network` and an L2 block height.
2. Document commands: stress discover-only, preflight validate, guest-launcher native prove.
3. Document expected artifacts and failure triage.

### Task 6: Verify And Commit

**Commands:**

```bash
python -m unittest scripts/regression/tests/test_stress_shasta_proposal.py
cargo fmt --all -- --check
cargo test -p preflight
cargo run -p preflight -- --help
cargo run -p guest-launcher -- --help  # optional when SP1 build artifacts are available
```

**Commit:**

```bash
git add scripts/regression/stress_shasta_proposal.py scripts/regression/shasta_event_decoder.py scripts/regression/shasta/IInbox.json scripts/regression/shasta/Anchor.json scripts/regression/tests/test_stress_shasta_proposal.py scripts/regression/README.md .codex/skills/shasta-proposal-regression/SKILL.md docs/plans/2026-04-23-shasta-regression-discovery-design.md docs/plans/2026-04-23-shasta-regression-discovery.md
git commit -m "feat: add shasta proposal discovery regression flow"
```
