# Many-key Warm Dirty Storage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Test repeated cheap dirty writes over several state footprints, with reusable tools and literal node/full/actual evidence.

**Architecture:** One new finite producer composes guarded seed/warm/cleanup loops using existing assembler/key/guard primitives and the unchanged bytecode interpreter. Independent qualification and frozen same-program sampling precede data/docs/log/PR publication.

**Tech Stack:** Existing managed Python/std-library tests, frozen native/quote/SDK6.3/ELFb76 execute-only tools.

**Spec:** `docs/superpowers/specs/2026-11-03-storage-dirty-keys.md`.

## Global Constraints

- Keep c893/nativead0d78dd/quote/SDK6.3/ELFb76/host/guest/GuestInput/dependencies/pins/old generators and VM behavior unchanged. No prices/refit/Rust or guest build/merge/deploy/timer/automatic fixture publication.
- Exactly16 inputs: stop/revert xK1/16/256/1024 xzero/full;4users=4accounts visitedonce. Raw0 read-only phase, K main keys1->5->(7->5)^R->1,allK seeded before warm work.
- Storage/user2200+K*(5600+200R),intrinsic21000,memory6words18;STOPrefund2800K cappedgross//5;rootREVERTeffective0. Access4(K+1),write4K;no per-pair SLOAD.
-31Mheader,charged<=30M includingAnchor167355,<=15M/user,fullgross+2301 envelope,prefix/sentry,<=8192targets. Full means maximal uniformR, occupancy truthful; R+1 independently executed under sufficient gas.
- Primary literal node ceil sum/actual proposal PGU with separate full proposal and controls;>2B positive gap at<=30M highpriority. No substituted denominator/opcode actual allocation/globalbound/productioncertification.
- All16 independent gates precede frozen bounded actuals;6GiB/600s,256MiB floor+actual320MiB reserve,original quotes,max2parallelquotes,no implicitretry. Userlog updated and normal no-price rounds continue directly.

## Review Focus

- Large K/R can overflow immediate widths or interpreter steps: Task1 tests K*R four-byte boundary and actual full trajectories within existing bounds.
- Refund/prefix constraints can bind before30M: Task1 native/oracle envelope and sufficient-gas R+1 checks; Task2 truthful occupancy.
- Phase0 is read-only and seed/cleanup still execute atR0: Task1 literal streams/metadata negatives; Task2 separate access/write selector audit.
- Unrolled batch/tail or marker bugs can pass final-root checks: Task1 batch0/1/63/64/65 and native-accepted wrong-value controls; Task2 independent native/value/original/count/witness checks.
- Whole-input contrasts change work/gas/support: Task2 retain28 diagnostic flags and every literal deficit, no isolated price attribution.

### Task 1: Finite producer, semantic/gas tests and usage

**Files:** create `bin/zkgas/bench/cases/storage_dirty_keys.py`,`bin/zkgas/bench/tests/test_storage_dirty_keys.py`; append the new usage section to `bin/zkgas/bench/README.md`; include supplied spec/plan in scoped commit.

**Interfaces:** produce make_case(mode='stop',keys=1,level='full'),gas_accounting(mode,keys,rounds),fits(mode,keys,rounds),full_rounds(mode,keys),prepare(output:Path,*,model_id=MODEL); consume existing _Factory/key/guard/schema/BASE_ADDRESS/MODEL and unchanged execute/decode VM. IDs,counters,exact values and strict bounds follow spec.

- [ ] Write failing stdlib tests for16 actual-byte cases, exact storage/gas/refund/counter/marker/prefix/sentry invariants,64-boundary cases and strict APIs/catalog.
- [ ] Run RED with `~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests -p test_storage_dirty_keys.py -v` before producer implementation.
- [ ] Implement the small shared-runtime producer with honest metadata/identities and independently executable R+1; preserve all old helpers/producers.
- [ ] Add semantic negative/matching-native mutants and opt-in frozen-native specimens; document CLI/native tests and maximal-uniform occupancy limits.
- [ ] Run focused GREEN and old helper/journal/long-revert regressions; inspect full scoped diff/hygiene and commit ConventionalCommit, no broad builds.

### Task 2: Qualification, actuals and continuous record

**Files:** create `docs/research/2026-11-03-storage-dirty-keys.md`; update relevant `docs/research/zkgas-workload-coverage.md` rows; append user-owned Atlas working.log. Raw evidence/private helpers stay in separate retained campaign roots, never silently rewrite older data.

**Interfaces:** reviewed Task1 catalog -> independently qualified16 canonical inputs/original quotes -> frozen config/policy/source/artifacts ->16 actuals/28 diagnostics -> independent audits/recovery/docs/log/PR296 exact-head update.

- [ ] Independently review Task1 source/spec and qualify16 native/signatures/receipts/roots/attempted streams/canonical witnesses/real quotes/collection counters before freeze.
- [ ] Freeze inputs/source/tools/quotes/resources, verify actual576MiB minimum disk reserve, then sequential same-input SP1; retain failures without implicitretry.
- [ ] Independently verify16 raw PGU conversions/quote joins/deficits and28 scaled diagnostics, read/write-set semantics and host resources. Controls are nonempty seed/cleanup work; changed R/refunds/gas/support prevent isolated-opcode attribution.
- [ ] Write concise research/coverage, preserve all gaps and bounded-domain limits; verify rows/links/hashes/hygiene, recoverable archive, commit docs after final-data gate.
- [ ] Final independent whole-increment review; update own Draft/do-not-merge PR296 branch/body/readback, log verified no-price conclusion/currentnextstage and continue selected followup without user checkpoint.
