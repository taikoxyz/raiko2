# Persistent Storage Mutation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure committed many-leaf storage work in valid gas-bounded multi-user blocks.

**Architecture:** Permit the existing storage fixture with bounded explicit CALL users; reuse its constructor and proof builder. A separate Python producer partitions raw slots using one persistent phase slot and derives exact gas/refunds. Independently qualify inputs, freeze artifacts, measure SP1 and report both estimate surfaces.

**Tech Stack:** Existing Rust fixture and Python standard-library producers/tests; retained SP1 tools.

**Spec:** `docs/superpowers/specs/2026-11-02-storage-mutation.md`

## Global Constraints

- No guest build, pricing/SDK/ELF/dependency/production-pin/GuestInput ABI change.
- Preserve30M charged gas,31M header and existing15M per-user tool cap; ordinary Unzen/Osaka EVM rules apply.
- Node literal block sum, full proposal estimate and identical-proposal actual PGU stay separate; Unzen score is not PGU.
- Retain failures/N0 and honest mutation-aware multiproofs. Overquotes require ordinary-transaction compatibility assessment, not automatic dismissal.
- Python uses the existing user-managed virtual environment; no dependencies installed. Portable artifacts contain no personal paths.

## Review Focus

- Multi-user storage must share the intended recipient and seed exactly once; created/authorization/access-list combinations must remain rejected.
- Gross gas and charged gas differ under refunds; each prefix must have enough remaining header gas for the next signed envelope.
- Phase slot is shared but every body range is disjoint; equal values/counts can hide wrong target order without an independent oracle.
- Mutation hints and actual final root must agree, including delete support nodes and absent insert keys.
- Legacy default/native profiles and frozen GuestInput stay unchanged; new artifacts must be source/hash bound before actuals.

### Task 1: Bounded storage CALL envelopes

**Files:**
- Modify: `bin/zkgas/src/fixture/transactions.rs`, narrow optional diagnostics in `storage.rs`/`constructor.rs`, and focused existing fixture tests as needed.

**Interfaces:**
- Consumes: existing `ProgramTransactions::validate`, `ProgramStorageState`, shared-recipient constructor.
- Produces: storage_state + CALL +one block +1..8 explicit users accepted with existing gas bounds; all other combinations unchanged.

- [ ] Write failing regression cases for accepted1/2/8 CALL users and rejected9/create/multiblock/access-list/authorization combinations.
- [ ] Implement the narrow validation change, preserving absence/default serialization and existing nonstorage behavior.
- [ ] Report optional final root/count from actual committed recipient storage only in the new explicit-storage mode; test values and legacy omission for independent full-trie verification.
- [ ] Run focused fixture Rust tests and package Clippy/fmt with existing shared target, bounded jobs2. Do not build guests or install dependencies.
- [ ] Inspect complete diff/path hygiene and commit source plus the already-written spec/plan.
- [ ] Independent review and behavioral verification compare representative old/native inputs byte-exact and validate multi-user phase/state effects. Freeze one reviewed native executable with source/compiler/hash metadata.

### Task 2: Reusable24-case producer and semantic tests

**Files:**
- Create: `bin/zkgas/bench/cases/storage_mutation.py`
- Create: `bin/zkgas/bench/tests/test_storage_mutation.py`
- Modify: `bin/zkgas/bench/README.md`

**Interfaces:**
- Consumes: existing assembler and fixture schema with explicit8 CALL users.
- Produces: `make_case(mode='update', state_size=8192, level='full') -> dict`; `prepare(output: Path, *, model_id=MODEL) -> int`; CLI `--output-dir`, `--model-id`; exact24 request/catalog identities.

- [ ] Test strict modes/sizes/levels,24 distinct identities, own-N0 envelopes/background, typed bounds and no overwrites before implementation.
- [ ] Implement fixed-width loop, phase0 range partition, before-value guards, post-write checksum/counter and final phase update. Derive gross, refund and charged gas and maximal full N under all spec constraints.
- [ ] Independently execute actual bytes across8 users with EIP2200/2929/3529 cold/warm/original/current/refund semantics, verify ordered keys/values, final state, opcode routes and exact gas. Reject wrong ranges, values, hints, final phase and checksum mutants.
- [ ] Add opt-in frozen-native smoke controls and usage. Preserve existing producers; avoid duplicate private runtime implementations in product code.
- [ ] Run focused Python tests plus relevant existing regressions, inspect full diff and commit. Independent source review and all24 native/quote/root/signed-receipt qualifications precede actuals.

### Task 3: Frozen actuals and findings

**Files:**
- Create: `docs/research/2026-11-02-storage-mutation.md`

**Interfaces:**
- Consumes: reviewed producer/new native and qualified canonical inputs; unchanged c893/quote/SDK6.3/ELF.
- Produces: immutable freeze,24 guarded same-input actuals, metric/paired reports and retained recovery evidence.

- [ ] Preflight actual disk reserve; freeze source/input/tools/model and controls before sequential actuals, with no implicit retries.
- [ ] Independently join raw actual gas10//191, canonical hashes, public outputs, node literal sums, proposal totals, charged gas/refunds and resource receipts. Retain all failures and own-N0 differences.
- [ ] Report whole-block shortfalls, >2B priority and overestimate/compatibility limits; do not fit on holdout data or call finite coverage a global bound.
- [ ] Independently review the full increment, commit research and update Draft/do-not-merge PR296 with exact-head readback. No merge/deploy; separate Atlas fixture publication may follow.
