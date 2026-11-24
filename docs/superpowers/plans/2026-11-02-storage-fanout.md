# Initialized Storage Fanout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure independent storage-root fanout under valid block gas without interpreter CALL reserve masking.

**Architecture:** Extend only existing host prestate/transaction configuration, reusing actual read discovery and canonical proofs. A separate Python producer composes the existing guarded storage runtime across64 ordered root transactions. Independently qualify native state/quotes, freeze artifacts and sample retained SP1.

**Tech Stack:** Existing Rust host fixture and Python standard-library tools; retained quote/SP1/model/ELF.

**Spec:** `docs/superpowers/specs/2026-11-02-storage-fanout.md`

## Global Constraints

- No guest/SDK/ELF/GuestInput/model/dependency/production-pin change or price refit.
- Keep31M header, <=30M charged block gas, existing15M/user cap and valid ordered admission reservations.
- Literal node sum, full proposal estimate and same-input actual PGU remain distinct; raw Unzen score is not PGU.
- Existing omitted/default/code-only outputs remain byte-exact. Authenticated canonical conservative proofs are not claimed mathematically minimal.
- Use existing user-managed Python environment and shared Cargo target with jobs2 and incremental disabled. No unrelated cache/worktree cleanup.

## Review Focus

- Optional recipients must be enforced at constructor entry as well as parsing; signing and committed to/poststate checks must use the same index.
- Each storage map belongs to its declared account, with real raw-slot hashing and correct initial/final roots; do not copy the first map across accounts.
- Final diagnostics must follow witness root refresh; report fields omitted in old modes.
- Hold64 users and shared code constant; reject recipient mutants even if symmetric final maps/counts appear correct.
- Refunds, per-account phase transitions and actual prefix envelope reservations must all agree with native receipts.

### Task 1: Host initialized contracts and ordered recipients

**Files:**
- Modify: `bin/zkgas/src/fixture/prestate.rs`, `transactions.rs`, `constructor.rs`, `mod.rs` as narrowly needed.
- Test: `bin/zkgas/tests/fixture.rs` and focused internal fixture tests if needed.

**Interfaces:**
- Consumes: existing Program/prestate/transaction structs, ControlledTrieState, native read recorder and minimized_witness.
- Produces: optional `initial_storage` maps and `transactions.recipients`, optional initialized-contract root/count reports; defaults unchanged.

- [ ] Write failing tests for alternating recipients with distinct initialized values, repeated visits, correct signed to/nonces and final roots/counts.
- [ ] Test wrong length/undeclared recipient/create/multiblock/unsupported combinations, zero initial values and aggregate entry overflow; constructor validation must enforce the same rules.
- [ ] Implement central recipient lookup/cross-field validation, proper per-account storage seeding and post-witness diagnostics. Preserve default paths.
- [ ] Run focused/full zkgas package tests, scoped Clippy and fmt using existing shared target, jobs2, CARGO_INCREMENTAL=0; retain logs. No guest build.
- [ ] Self-review complete diff/hygiene, commit source and supplied spec/plan, copy a separate standalone native with full source/toolchain/hash provenance.
- [ ] Independent source and behavioral review: old-default byte equivalence, distinct/repeated recipient roots, invalid combinations and real quote public-output join. Freeze only after gates.

### Task 2: Reusable24-case fanout producer and semantic tests

**Files:**
- Create: `bin/zkgas/bench/cases/storage_fanout.py`
- Create: `bin/zkgas/bench/tests/test_storage_fanout.py`
- Modify: `bin/zkgas/bench/README.md`

**Interfaces:**
- Consumes: storage_mutation runtime/gas_accounting, host recipient/prestate schema.
- Produces: `make_case(accounts=1, state_size=8192, mode='update', level='full') -> dict`, `prepare(output: Path, *, model_id=MODEL) -> int`; CLI `--output-dir`/`--model-id`.

- [ ] TDD exact3x2x2x2 grid, typed bounds, unique IDs/request hashes, source provenance and no-overwrite behavior.
- [ ] Compose64 round-robin recipients, S/A raw initialized slots and one shared runtime, with full N derived from all bounds and uniform own-N0 envelopes. Reuse rather than duplicate runtime/gas logic.
- [ ] Independently execute actual bytes across isolated rolling account states; assert exact recipient schedule, raw reads/writes, originals/warmth/refunds, final maps/counts/phases, opcode routes and maximal N+1 failure.
- [ ] Reject rotated/collapsed recipients, wrong storage owner/phase/value and malformed declarations; distinguish native success from intended semantics. Add opt-in bounded native smoke and usage.
- [ ] Run focused and appropriate existing Python regressions, inspect full diff, commit. Independent source and all24 native/real-quote/root/signed-order gates precede actuals.

### Task 3: Frozen SP1 measurements and finite findings

**Files:**
- Create: `docs/research/2026-11-02-storage-fanout.md`

**Interfaces:**
- Consumes: reviewed producer/new native,24 qualified canonical inputs and real quotes; unchanged c893/SDK6.3/retained ELF.
- Produces: immutable freeze,24 guarded execute-only actual observations, literal metrics and account/state contrasts, preserved evidence.

- [ ] Preflight disk including existing320MiB retention reserve; freeze source/input/tool/model/quote identities before actuals. Sequential6GiB/600s SP1, no implicit retries.
- [ ] Independently join raw actual gas10//191, input/public-output identities, original real quote provenance, node literal sums, full totals, gas/refunds and shared-memory receipts. Retain all controls/failures.
- [ ] Report high-priority >2B gaps, all literal node/full deficits, ordinary-transaction usability and ranking/overquote limits. Do not assign proposal actuals to transactions or call this a global bound.
- [ ] Independently review full source/results increment, commit research, update existing Draft/do-not-merge PR296 with exact-head readback. Preserve data locally for separate immutable Atlas publication; no merge/deploy.
