# SP1 Prover Gas Research Design

## Goal

Make SP1 prover-gas research cheap to run locally by using execute-only SP1 guest execution,
without submitting proofs to the Succinct prover network.

## Current State

Raiko2 already has the important primitive:

- Hosted API requests can use `proof_type=sp1` with `sp1.mode=execute`.
- `sp1.mode=execute` rejects `aggregate=true` and rejects `sp1.prover=network`.
- The SP1 prover implementation calls `ProverClient::execute()` and stores the
  `ExecutionReport` under `proposals[].extra_data.sp1`.
- `guest-launcher --proof-type sp1 --mode execute --json-out <path>` also calls
  `ProverClient::execute()` and writes a local JSON execution report.
- `xtask bench-guest sp1` wraps preflight, optional guest rebuild, repeated
  `guest-launcher` runs, and summary output.

This matches Succinct's recommended profiling flow: prover gas is read from the
`ExecutionReport` returned by `ProverClient::execute()`, so no proof generation or network prover
submission is required.

## Problem

The local path is present, but it is not yet ergonomic enough for an optimization loop:

- `guest-launcher` records `gas`, instruction counts, syscall counts, opcode counts, and memory
  snapshots, but `xtask bench-guest` only preserves wall time and cycle tracker fields in its
  aggregate report.
- `xtask bench-guest` is oriented around one input at a time. A research loop needs a stable report
  shape for a set of cases.
- Preflight can be expensive or RPC-sensitive. For repeated optimization, the fastest loop should
  accept checked-in or cached `GuestInput` JSON files directly.
- Prover gas calculation is useful but not free. SP1 SDK documents `calculate_gas(false)` as a
  faster execute mode when the caller only wants correctness, cycles, or instrumentation.

## Proposed Interface

Keep the existing single-input command working:

```bash
cargo run -r -p xtask -- bench-guest sp1 \
  --skip-build-guest \
  --input ./tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json \
  --repeat 3 \
  --json-out /tmp/sp1-prover-gas.json
```

Add a suite manifest for multi-case runs:

```json
{
  "cases": [
    {
      "name": "mainnet-proposal-2222",
      "input": "./tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json",
      "proof_type": "sp1"
    }
  ]
}
```

Run it with:

```bash
cargo run -r -p xtask -- bench-guest sp1 \
  --skip-build-guest \
  --suite ./bench/sp1-suite.json \
  --repeat 3 \
  --json-out /tmp/sp1-suite-report.json
```

The suite path intentionally starts with existing `GuestInput` files. This avoids live RPC and
keeps the loop deterministic. Proposal discovery or preflight generation can stay outside the suite:
use `scripts/regression/stress_shasta_proposal.py --discover-only --proposal-out` to capture the
full Shasta proposal tuple, run `preflight` once, then point the suite at the cached `GuestInput`.

## Report Shape

Single-input and suite reports should preserve the useful execute metadata:

- `gas`: SP1 prover gas from `ExecutionReport::gas()`
- `exit_code`
- `total_instruction_count`
- `total_syscall_count`
- `touched_memory_addresses`
- `cycle_tracker`
- `invocation_tracker`
- `opcode_counts`
- `syscall_counts`
- `memory_snapshots`
- `wall_time_ms`

Summaries should include stats for:

- wall time
- prover gas, when present
- total instruction count
- total syscall count
- touched memory addresses
- total cycle tracker cycles, when cycle tracking exists
- per-label cycle tracker medians

The report should not rename the underlying SP1 field away from `gas`, but summary text can call it
`prover_gas` so operators do not confuse it with EVM gas.

## Speed Model

There are three useful loops:

1. Rebuild guest once, then run a suite with `--skip-build-guest`.
2. Use checked-in or cached `GuestInput` files to avoid preflight and live RPC.
3. Optionally add a no-gas execution mode later by wiring SP1's `calculate_gas(false)` through
   `guest-launcher` and `xtask bench-guest`.

For the initial change, keep prover gas enabled by default because the purpose is to measure
prover gas. The implementation should leave room for a later `--calculate-gas=false` option, but it
does not need to add it unless the current loop is still too slow.

## Non-Goals

- Do not submit SP1 network proofs.
- Do not add aggregation benchmarking in this pass.
- Do not change hosted proof semantics or proof formats.
- Do not add autoresearch parameter mutation logic in raiko2 yet. The first step is a stable local
  report surface that an external loop can consume.

## Testing Strategy

- Add focused unit tests around `xtask bench-guest` JSON report parsing and summarization so `gas`
  and execution counters are not dropped.
- Add focused unit tests for suite manifest parsing and case-level report grouping.
- Keep `guest-launcher` execution behavior unchanged unless a missing field requires a small
  report-struct update.
- Run `cargo test -p xtask bench_guest`.
- Run `cargo fmt --all` after Rust edits.

## Success Criteria

- Existing `sp1.mode=execute` remains software-only and still rejects network prover execution.
- `xtask bench-guest --json-out` preserves SP1 prover gas and execution metadata from
  `guest-launcher`.
- A suite manifest can run multiple existing `GuestInput` files in one command and emit one
  aggregate JSON report.
- The checked-in development docs show the command an operator should run for a local prover-gas
  research batch.

## Implementation Notes

The initial implementation keeps the single-input command and adds `--suite <path>` for JSON
manifests whose cases point at existing `GuestInput` files. The top-level report keeps its existing
`runs` and `summary` fields for compatibility and adds `cases[]` for per-case runs and summaries.
