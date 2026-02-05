# Shasta Regression Harness — Design

## Goal
Create a regression harness that validates Shasta proposal proof generation via **preflight → guest-launcher → native proof**, and supports **native aggregation** using proof envelopes. The harness is file-based, deterministic, and independent of HTTP API testing.

## Scope
- Python script: `script/shasta_regression.py` (in raiko2 repo).
- Output directory: `test/regression/shasta/`.
- Serial execution only (for determinism and easier debugging).
- Discovery and grouping are Python-only; no Rust event discovery.
- Aggregation is per proof type (no cross-type aggregation).

Non-goals: API testing, parallel execution, zk aggregation, and Rust-based discovery.

## Data Flow
1. **Discovery** (Python): collect proposal IDs (and optional L2 ranges) using existing RPC/ABI logic.
2. **Preflight**: run `preflight` for each proposal and store `proposal_<id>.json` under `test/regression/shasta/`.
3. **Proof generation**: run `guest-launcher` with a new `--emit-proof <path>` option to produce `proposal_<id>.proof.json` (ProofEnvelope JSON) for native proofs.
4. **Manifest**: generate an aggregation manifest JSON containing grouped proof file paths (group size `--agg-size N`).
5. **Aggregation**: run `guest-launcher --aggregate-manifest <manifest.json>` to produce aggregation proof envelopes per group.
6. **Summary report**: write `report.json` with per‑proposal status, timings, and errors.

## CLI / UX
- `script/shasta_regression.py`
  - Required: `--l1-rpc`, `--l2-rpc`, `--l2-chain-id` (if needed for preflight)
  - Discovery: `--proposal-ids` or `--l2-range` or both (Python logic)
  - Proof: `--proof-type native`
  - Aggregation: `--agg-size N`
  - Output: `--out-dir test/regression/shasta`

- `guest-launcher` additions:
  - `--emit-proof <path>`: emit ProofEnvelope JSON for native runs
  - `--aggregate-manifest <path>`: read manifest and emit aggregation proof envelope
  - `--aggregate-dir <dir>`: convenience to read all `*.proof.json` in dir

## Proof Envelope (Native)
- `backend: "native"`
- `public_inputs`: `input_hash` and/or `instance_hash` (aligned with Proof.input)
- `payload.bytes`: opaque native proof bytes (SGX Shasta format)
- `carry_data`: PCD (from `proof_carry_data`)
- `metadata`: proposal_id, chain_id, timing metrics

Aggregation outputs use the same envelope format, with `payload.bytes` holding the SGX‑format aggregation proof bytes and `public_inputs` reflecting the aggregation hash.

## Error Handling
- Fail fast on missing or malformed proof files.
- Validate proof backend types match `native`.
- Record subprocess exit codes and stderr in `report.json`.

## Deliverables
- `script/shasta_regression.py`
- `guest-launcher` support for `--emit-proof` and aggregation manifest
- `test/regression/shasta/` output layout and report schema

## Rollout
- Land as a separate PR after the native proof generator changes.
- Expand later to support other proof types.
