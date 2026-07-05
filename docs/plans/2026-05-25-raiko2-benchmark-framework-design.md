# Raiko2 Benchmark Framework Design

## Status

Draft for discussion.

This document narrows the open prover platform work to comparable benchmarking across local zkVM,
remote zkVM, and TEE-backed providers.

## Goal

Create a benchmark framework where multiple providers can be measured against the same Shasta
execution statement with stable workload definitions, stable report schema, and reproducible case
metadata.

## Non-Goals

- Do not make benchmark support a blocker for the first provider registry refactor.
- Do not use synthetic opcode-only loops as the primary public comparison suite.
- Do not require every backend to expose identical low-level counters.

## Current Problem

`guest-launcher` already exposes useful execution metadata:

- wall time
- cycle tracker labels
- opcode counts
- syscall counts
- memory snapshots

But `bench-guest` is still effectively SP1-centric, and the existing fixture suites are not yet a
cross-provider benchmark framework with:

- stable case metadata
- tagged workloads
- expected outputs
- comparable JSON reports

## Benchmark Levels

Use four levels instead of one:

1. `L0` microbenchmarks:
   - encoding
   - blob decoding
   - hashing
   - witness materialization
   - trie operations
2. `L1` contract cases:
   - deterministic blocks generated from known contracts
3. `L2` proposal fixtures:
   - complete Shasta `GuestInput` cases with expected carry hash
4. `L3` end-to-end proving:
   - proof generation
   - proof verification
   - aggregation
   - remote latency

The public comparison surface should primarily report `L2` and `L3`. `L0` and `L1` should remain
diagnostic layers for investigating regressions.

## Case Manifest

Each benchmark case should carry checked-in metadata:

```toml
schema = "raiko2-benchmark-case-v1"
name = "storage-hot-cold"
fork = "shasta"
level = "proposal"
guest_input = "benchmarks/fixtures/storage-hot-cold.input.json"
expected_carry_hash = "0x..."
tags = ["storage", "sload", "sstore", "unzen"]
scale = "medium"
```

The manifest should be checked in with the fixture or generated reproducibly from checked-in source
material.

## First Contract Case Families

The first prebuilt contract suite should cover:

- arithmetic baseline
- Keccak-heavy execution
- storage hot/cold `SLOAD` and `SSTORE`
- contract create and `create2`
- `call`, `delegatecall`, `staticcall`, and failed call
- precompiles
- logs and bloom-heavy cases
- memory expansion and calldata-heavy cases
- revert and invalid transaction filtering
- anchor transaction and Shasta proposal metadata
- blob/data-source derivation

Opcode-level metrics are still useful, but they should be metadata collected from these contract
cases rather than the primary benchmark inputs.

## Report Shape

Benchmark reports should include:

- provider id
- provider version and SDK version
- guest digest or enclave measurement
- case name and case digest
- input hash and carry hash
- mode: execute, prove, aggregate, verify
- wall time
- peak RSS
- proof size
- verifier time
- cycle counts or backend-specific execution counters when available
- remote queue/submission time when applicable

Output should be JSON-first and Markdown-renderable.

## Benchmark And Security Boundary

Benchmarking should not define acceptance on its own.

It answers:

- how two providers compare
- where regressions happened
- which workloads stress different proving systems

It does not answer:

- whether a provider satisfies the Shasta proving statement
- whether a provider is safe to accept into production

Those concerns belong to the security invariant and mutation-suite workstream.

## Migration Strategy

### Phase 1: Schema And Fixture Layout

- define benchmark case manifest schema
- define report schema
- decide where checked-in benchmark fixtures live

### Phase 2: Runner Generalization

- generalize `bench-guest` beyond `sp1`
- support local zkVM, remote provider, and TEE-backed cases

### Phase 3: Public Comparison Suite

- add the first contract-case family set
- add stable `L2` and `L3` benchmark cases
- emit comparable JSON reports for release and regression use

## Likely Next-Level Split

This document should later split into narrower designs or implementation plans:

1. benchmark case schema and fixture layout
2. case generation and checked-in workload families
3. benchmark runner/reporting integration
4. public comparison suite and release reporting

## Open Questions

- which benchmark case set should become the required public comparison suite
- should remote queue time be reported separately from proof wall time by default
- which backend-specific counters are useful enough to standardize in the main report schema
- should benchmark output be checked into the repo, attached to releases, or both
