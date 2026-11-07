# Few Long Root-REVERT Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Test fewer long failed transactions without the previous64-event REVERT allowance, retaining reusable tools and independently checked evidence.

**Architecture:** Add a finite producer using the existing guarded runtime/gas helper and authoritative request schema. Qualify all inputs independently before frozen same-program SP1 actuals; preserve both quote surfaces, controls and resource receipts. Ordinary no-price conclusions go to `working.log` and research docs without another user checkpoint.

**Tech Stack:** Existing managed Python, standard-library tests, unchanged native/quote/SP1 execute-only binaries.

**Spec:** `docs/superpowers/specs/2026-11-03-long-root-revert.md`.

## Global Constraints

- Keep model c893, native ad0d78dd, SDK6.3/ELFb76, host/guest/GuestInput/dependencies/pins unchanged; no Rust/guest build, refit, activation, merge, timer or automatic Atlas publication.
- Preserve literal node block ceiling sums versus whole same-proposal actual, complete-proposal estimates separately, all own-N0 controls and failures. Positive actual-minus-node >2B PGU at <=30M charged block gas remains high priority.
- Exactly16 inputs: modes update-revert/restore-revert,users2/4,state8192/16384,zero/full. Account count equals users; each is visited once.
-31M header,30M charged including Anchor167355,15M/user,16777216 EIP7825 cap,full gross+2301 envelopes and every ordered prefix/sentry; <=8192 distinct targets and initialized ranges.
- Only zero-range rootREVERT; effective refunds0, restore preterminal2800N; attempted work retained and complete contract maps/roots restored. Expected failed user receipts are valid inclusion, not invalid tx.
- No duplicate runtime/gas logic or old generator changes; strict source/helper/request/model identities. Managed Python only; no implicit measurement retries.

## Review Focus

- A2/A4 requests must not silently retain64-user routes or recipient metadata: Task1 actual-byte/all16 and schema tests.
- RootREVERT keeps attempted events but removes refunds/state changes: Task1 trajectory,wrong-refund/state/terminal regressions; Task2 independent raw/native verification.
- FullN must be constrained by candidate gas/range/targets/prefix/sentry, not N+1 out-of-gas: Task1 independent sufficient-envelope N+1 execution.
- Large declared maps or native acceptance must not conceal wrong owner/value/checksum: Task1 semantic mutants; Task2 all16 signed roots/witness gates.
- Reducing event count must not be interpreted as tariff attribution or global coverage: Task2 preserve full/literal/sensitivity metrics and limited research claims.

### Task 1: Reusable finite producer and actual-byte regressions

**Files:** create `bin/zkgas/bench/cases/storage_long_revert.py`, `bin/zkgas/bench/tests/test_storage_long_revert.py`; modify only the relevant new usage section of `bin/zkgas/bench/README.md`; include the supplied spec/plan in the scoped commit.

**Interfaces:** produce `make_case(mode='update-revert',users=2,state_size=8192,level='full')->dict`, `fits(mode,users,state_size,n)->bool`, `full_n(mode,users,state_size)->int`, `prepare(output:Path,*,model_id=MODEL)->int`; consume `storage_root_journal._runtime/gas_accounting` and existing request schema, address base and actual-byte VM. Corpus/IDs and exact maps follow the spec.

- [ ] Write failing stdlib tests covering all16 actual-byte programs, shared pair maps/envelopes, exact gas/refunds/routes/phases/attempted support, N+1 under sufficient envelope and strict invalid inputs/output/model handling.
- [ ] Run RED with `~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests -p test_storage_long_revert.py -v` before implementation.
- [ ] Implement the small producer with fresh truthful metadata and transitive source/helper identities; reuse rather than copy runtime/gas/oracle primitives.
- [ ] Add semantic counterexamples and unchanged-native opt-in specimens; document CLI/test usage and finite evidence bounds.
- [ ] Run focused GREEN and old journal/shared VM regressions, inspect complete scoped diff/path hygiene, commit Conventional Commit. No source binaries/guest rebuild.

### Task 2: Frozen qualification, actuals and continuous research record

**Files:** create `docs/research/2026-11-03-storage-long-revert.md`; update relevant rows of `docs/research/zkgas-workload-coverage.md`. Raw qualification/actuals/recovery are separate retained evidence. Append the user-owned Atlas `working.log` without committing local operational state or adding a runner there.

**Interfaces:** consume reviewed Task1 catalog, all16 independently qualified canonical inputs/original quotes and unchanged artifacts; produce frozen config/policy,16 raw actuals, analysis and independent audit identities, concise research record and current log state.

- [ ] Independently review Task1 spec/source; independently qualify all16 actual bytes/native/signatures/receipts/full roots/canonical proof support/original real quotes and counter retention before freeze.
- [ ] Freeze source/tool/input/quote/artifact/resource/policy identities. Check disk reserve, then sequential bounded same-input SP1 actuals without implicit retries; retain failures as failures.
- [ ] Independently audit all raw actuals, literal node sums/full totals, PGU normalization, controls, own-N0 increments and packing/state contrasts. Preserve per-event REVERT allowances without replacing primary denominators.
- [ ] Document all results, remaining gaps and finite-domain limits; verify local links/hashes/rows and review full diff. Commit research docs after audits.
- [ ] Final independent whole-increment review, FF push own PR296 branch, update body and exact-head readback, retaining Draft/do-not-merge. Append verified conclusion/current-next-stage to working.log; directly proceed to the next no-price interaction rather than waiting for user confirmation.
