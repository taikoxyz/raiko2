# ZisK CPU Execution Implementation Plan

> **For agentic workers:** Use implementation ownership followed by independent code review and behavioral verification. Track the checks below; do not substitute mocked proving for CPU execution evidence.

**Goal:** Prepare ZisK proposal/aggregation guest code and run a complete saved proposal through a CPU-only execution benchmark, comparing its public output with the existing native implementation and recording weighted cost, elapsed time and memory.

**Architecture:** Keep the existing `GuestInput` and `raiko2-guest-common` proposal/aggregation implementation. Add a standalone ZisK guest workspace and a CPU benchmark adapter that uses the official `ziskemu` executable; keep the ZisK host SDK out of the production workspace dependency graph. Actual proving and genuine child-proof aggregation remain explicitly unverified until a prover is available.

**Tech Stack:** Current raiko2/reth pins (Rust minimum 1.95), ZisK SDK/runtime `v1.3.2-alpha` at `1e22630bb7e143c30705a1603ed87cfd3da8c6ec`, official compiler candidate at `c83bf43be864c5c444d446c38f47d250d97761ca` (source version 1.99.0), bincode 1.3.3, Rust CLI, CPU emulator.

**Spec:** [ZisK feasibility investigation](../../zisk-integration-feasibility.md), narrowed by the user to code readiness and benchmark execution; proving is not an acceptance requirement.

## Global Constraints

- Do not downgrade shared dependencies, add runtime SDK/ELF version gates, enable production routes or change production zkGas policy.
- Reuse the complete proposal validator and shared aggregation business logic; do not copy an execution loop or skip blob/carry checks.
- Frame bincode 1.3.3 proposal bytes in ZisK stdin; do not feed RISC0 framing or ZisK's default bincode 2 encoding to that decoder.
- Proposal output is the same 32 raw digest bytes. ZisK proof/public-slot representations need explicit checked conversion.
- Guest aggregation must authenticate child proof identity and bind public output to carry data. Missing authoritative key material must fail clearly; demo keys or a no-op verifier must not masquerade as production-ready aggregation.
- CPU execution/profiling must not load proving keys, submit proving requests or perform chain/service writes. No rollout, image publication or PR creation is in scope.
- Preserve the existing checkout. Keep downloaded tools, build outputs and benchmark artifacts outside committed portable source; use an existing Python environment if Python is needed.
- Execute downloaded tools in an isolated container, with no secrets or Git credentials, no network during execution, and only scoped source/tool mounts plus disposable output/cache directories.

## Review Focus

- Wrong stdin lengths/alignment or codec mismatches must fail rather than silently mutate input.
- Nonzero emulator exit, truncated output or reaching a step limit must not produce a successful benchmark report.
- A mismatched digest, forged public slots or proof/carry mismatch must be rejected.
- Distinguish operation-weighted profiling cost from setup-backed cells and SP1 prover gas; absent metrics are not zero measurements.
- Report the fixture's actual fork and provenance; the checked-in proposal fixture is pre-Unzen and cannot establish post-Unzen compatibility.

## Task 1: Establish the isolated compiler/emulator environment

**Responsibility:** Root agent; downloaded artifacts and disposable execution environment only.

- [ ] Fetch the two official archives and verify their published digests before extraction; inspect archive paths and executables before use.
- [ ] Check compiler version and ZisK target support inside a network-disabled container. Capture the exact compiler/SDK executable identity and environment.
- [ ] Check that the CPU emulator launches and supports cost statistics and binary public output without proving keys.
- [ ] Inspect build/dependency entry points; fetch dependencies separately from execution in a disposable Cargo cache.
- [ ] Record any compiler/SDK compatibility failure as a source or environment issue with its exact command/output, rather than downgrading shared pins.

## Task 2: Implement the guest and CPU benchmark adapter

**Files/responsibility:** One implementer owns `guests/zisk/`, the focused benchmark binary/module, its workspace registration if needed, shared ZisK proof/input types under `crates/primitives-shasta/` if needed, and focused tests/documentation. Root owns tool acquisition and running the real benchmark. Keep changes out of live prover/server routing.

**Interfaces:** Consume the existing `GuestInput`, `prove_shasta_proposal`, `aggregate_shasta_zk_with_verifier`, and bincode 1.3.3. The benchmark accepts explicit input/guest ELF/emulator/output paths. It prepares framed input, computes the native expected hash, runs the CPU emulator, checks output and writes a JSON report plus raw logs/statistics.

- [ ] Add failing focused tests for raw stdin framing/codec round-trip and public digest extraction, including truncation and malformed lengths/slots.
- [ ] Implement the standalone proposal guest wrapper, using current shared dependencies and ZisK runtime. Keep dependency resolution and crypto patches isolated to this guest workspace.
- [ ] Prepare the aggregation wrapper/proof adapter with authoritative VK handling and checked proof/public-output binding. If key material is unavailable, keep the build/enablement boundary explicit; do not insert a fake verifier or claim a genuine aggregation pass.
- [ ] Add failure tests for process failure, missing/malformed cost data and output mismatch; implement the benchmark CLI around the official CPU emulator, with an explicit configurable execution limit and no proving operation.
- [ ] Emit backend-specific metric names, raw breakdown, input/ELF identity, native/guest output comparison, elapsed time and measured peak RSS (or explicit absence). Preserve native errors and emulator errors.
- [ ] Run focused unit tests and formatting. Build both proposal and aggregation code where genuine setup/program identities are available; record any unverified aggregation boundary precisely.

## Task 3: Run complete-input execution and finish review

**Responsibility:** Root runs the real CPU benchmark; independent reviewer and tester inspect the complete diff and behavior.

- [ ] Build the guest with the verified compiler and run the checked-in complete Mainnet proposal fixture; compare its exact hash against the shared native validator.
- [ ] Locate any existing complete post-Unzen fixture without accessing secret files; if available, run it too. Otherwise state that current-fork execution remains unverified.
- [ ] Save raw output/logs/statistics and a reproducible benchmark report. Repeat a small deterministic probe where useful; do not repeatedly rerun the entire expensive fixture without a reason.
- [ ] Independently review the actual diff and verify framing, bad-exit, malformed-stats and hash-mismatch behavior. Fix confirmed findings and obtain the originator's re-check.
- [ ] Run focused tests/builds and proportionate workspace checks. Record failures caused by environment, resources or pre-existing source separately.
- [ ] Update the feasibility note and CPU workflow with measured results and exact commands. Commit locally on `feat/zisk-integration`; do not publish a production capability that has only compile/execution evidence.
