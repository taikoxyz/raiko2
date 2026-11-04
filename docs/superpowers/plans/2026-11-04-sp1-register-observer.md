# Historical SP1 register observer implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development. Steps are executed without interim user approval, under the existing continuous-experiment authorization.

**Goal:** Recover bounded selected-PC historical register observations while retaining the unchanged canonical gas computation.

**Architecture:** One optional observer consumes each already-held canonical raw chunk through public SDK6.3 splicing steps. The existing runner/options/GasVM/report path remains authoritative. No arbitrary RAM, private dispatch or full tracing records.

**Tech Stack:** Rust2024, existing standalone SDK6.3 estimator, existing cached dependencies, existing Python process/resource guards.

**Spec:** `docs/superpowers/specs/2026-11-04-sp1-register-observer.md`.

## Global constraints

- No price/model/guest/SDK/ELF/native-builder/pin/lock/cap/input change, guest build, download, private-dispatch copy, process suspension, native mmap read or key search.
- Preserve canonical134217728-memory-entry/two-slot cadence, input framing, aggregate gas and public values; original actuals remain immutable.
- Plans64KiB, watches64, registers0..31, per-watch samples1024/global2048, instruction limit1..1B, observation4MiB.
- Same6GiB/600s/16MiB log/192MiB stage/576MiB disk guard; no unsafe SDK rebuild or unrelated cleanup.
- Nontrivial integration: independent source review and independent behavior verification; own Draft296 stays do-not-merge.

## Review focus

- Reaching a trace end must not consume another memory oracle entry or reinitialize guessed state.
- A sample cap or instruction cap must not produce a false complete report.
- Invalid/oversized/duplicate plans must fail before creating a native runner or resource receipt.
- Borrowed trace guards must remain alive for both consumers and be released without retaining full histories.
- Default CLI/output and canonical gas/options must remain compatible; diagnostic timing is separate.

### Task 1: Implement bounded optional observations

Files: add `bin/zkgas/sp1-estimator/src/observation.rs`; modify `src/main.rs`, `src/estimator.rs`, estimator README and bench README; promote this spec/plan. Own focused tests in the touched modules. Existing IO/resources only if required for one authoritative implementation.

- [ ] Read actual SDK6.3 public step/trace lifetime APIs and the two source-only observer preflight reports.
- [ ] Resolve cached extern/native-link/compiler identities and enforce small private metadata/type checks before link; no Cargo dependency builds.
- [ ] Add failing focused tests for strict plan/domain/PC checks, sampling/counts/limits and no-observation compatibility.
- [ ] Implement plan validation, bounded observation state and typed public splicer steps; retain one canonical options/execution loop.
- [ ] Run focused tests and formatting/diff/hygiene checks with bounded host-only compilation. Retain all attempts and exact provenance.
- [ ] Self-review complete scoped diff, local Conventional Commit, freeze source and report; no diagnostic executions/push by implementer.
- [ ] Root dispatches fresh source/spec/quality reviewer before Task2.

### Task 2: Independently qualify diagnostic behavior and retain results

- [ ] Root derives an explicit ELF-bound selected-PC plan from actual b76 disassembly; no generic tool hardcoded PCs.
- [ ] Independent tester runs exactly three guarded diagnostics on retained inputs (zero default, zero observed, full observed), using source-reviewed executable and immutable original reports.
- [ ] Require exact original raw gas/PGU/public values/options and input/ELF identities, complete trace continuity and source-expected PC counts; test a bounded incomplete observation without inventing coverage.
- [ ] Preserve failures, limits, source/build/tool/inputs/outputs/resource receipts and a concise usage/result record. Do not relabel diagnostic runs as new pricing actuals or modify baseline evidence.
- [ ] Fresh whole-increment review, own held-Draft update/readback if ready, working.log/all rulings/own SDD closure, then continue hasher validation as a separate task.
