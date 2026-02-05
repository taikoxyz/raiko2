# Shasta Regression Tool Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a Python regression driver that discovers Shasta proposals, runs preflight + guest-launcher release binaries, and optionally aggregates proofs, writing artifacts to `test/regression/shasta/`.

**Architecture:** A single CLI script (`script/regression/shasta_regression.py`) loads JSON config + CLI overrides, discovers proposal IDs via L1 events/L2 lookup, then runs binaries in sequence. A helper shell script (`script/regression/prepare_regression.sh`) builds the release binaries. Outputs are deterministic per proposal ID.

**Tech Stack:** Python 3, `requests`, `web3`, `argparse`, `subprocess`, JSON; Rust binaries (`preflight`, `guest-launcher`).

---

## Usage

### 1) Build binaries

```bash
script/regression/prepare_regression.sh
```

### 2) Create config

```json
{
  "l1_rpc": "http://127.0.0.1:8545",
  "l2_rpc": "http://127.0.0.1:9545",
  "event_address": "0x0000000000000000000000000000000000000000",
  "event_abi": "path/to/event_abi.json",
  "anchor_abi": "path/to/anchor_abi.json",
  "timeout_sec": 3600,
  "poll_interval_sec": 3
}
```

### 3) Run regression

```bash
python script/regression/shasta_regression.py --config ./config.json --count 10 --aggregate 3
```

### Outputs

- `test/regression/shasta/proposal_<id>.json`
- `test/regression/shasta/proposal_<id>.proof.json`
- `test/regression/shasta/aggregation_<idx>.proof.json` (if aggregation enabled)
- `test/regression/shasta/run_summary.json`
- `test/regression/shasta/regression.log`

## Notes

- Use `--range <start:end>` to select a block range; it overrides `--count`.
- `--aggregate 0` disables aggregation.
- Script does not auto-build binaries; it will prompt if missing.
