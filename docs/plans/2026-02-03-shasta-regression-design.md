# Shasta Regression Harness — Design

> Historical design document. It may not match the current implementation. Use `README.md`,
> `docs/API.md`, and `config.example.toml` as the current source of truth.

## Goal

Create a regression harness that validates Shasta proposal proof generation via **preflight → guest-launcher (native or SP1) → proof JSON**, and supports **SP1 aggregation** using proof JSON outputs. The harness is file-based, deterministic, and independent of HTTP API testing.

## Scope

- Python script: `scripts/regression/shasta_regression.py` (in raiko2 repo).
- Output directory: `test/regression/shasta/`.
- Serial execution only (for determinism and easier debugging).
- Discovery and grouping are Python-only; no Rust event discovery.
- Aggregation is per proof type (no cross-type aggregation).

Non-goals: API testing, parallel execution, and Rust-based discovery.

## Data Flow

1. **Discovery** (Python): collect proposal IDs by scanning L2 blocks and extracting proposal IDs from `extraData` (range or count).
2. **Preflight**: run `preflight` for each proposal and store `proposal_<id>.json` under `test/regression/shasta/`.
3. **Proof generation**: run `guest-launcher --mode prove --proof-mode compressed --output <path>` to produce `proposal_<id>.proof.json` (Proof JSON).
4. **Aggregation**: run `guest-launcher --aggregate <proof...> --mode prove --output <path>` to produce aggregation proof JSON per group.
5. **Summary report**: write `run_summary.json` with per‑proposal status, timings, and errors.

## CLI / UX

- `scripts/regression/shasta_regression.py`
  - Required: `--config <json>` (chain spec list + rpc + chain ids)
  - Discovery: `--range <start:end>` or `--count <N>` (count capped)
- Proof: `--proof-type native` (default; set to `sp1` for SP1 proofs/aggregation)
- Aggregation: `--aggregate N`
- Output: `--out-dir test/regression/shasta`

## `guest-launcher` usage

- Proposal: `--input <path> --mode prove --proof-mode compressed --proof-type <native|sp1> --output <path>`
- Aggregation: `--aggregate <proof...> --mode prove --proof-type sp1 --output <path>`

## Proof Format

- `Proof` JSON (raiko2 primitives)
- `proof`: hex‑encoded bincode of `SP1ProofWithPublicValues`
- `input`: 32‑byte hash from public values
- `uuid`: verifying key hash (bytes32)
- `extra_data`: JSON‑encoded `ProofCarryData`

## Error Handling

- Fail fast on missing or malformed proof files.
- Validate proof backend types match `native`.
- Record subprocess exit codes and stderr in `run_summary.json`.

## Deliverables

- `scripts/regression/shasta_regression.py`
- `guest-launcher` support for `--output` and `--aggregate <proof...>`
- `test/regression/shasta/` output layout and report schema

## Rollout

- Land as a separate PR after the native proof generator changes.
- Expand later to support other proof types.
