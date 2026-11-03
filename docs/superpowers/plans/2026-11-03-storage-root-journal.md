# Storage Root Journal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure many-key restore/root rollback over heterogeneous storage roots and make remaining coverage explicit.

**Architecture:** A new Python producer composes existing disjoint prestate/recipient construction with one guarded restore/rollback runtime. A bounded test VM gains root REVERT; independent36 native/realquote gates precede frozen SP1 actuals and a coverage inventory.

**Tech Stack:** Existing Python standard-library tools and retained native/quote/SP1 binaries; no host/guest build.

**Spec:** `docs/superpowers/specs/2026-11-03-storage-root-journal.md`

## Global Constraints

- Keep pricing/model/native/quote/SDK6.3/ELF/pins/GuestInput/host/guest/old generators unchanged; no Rust/guest build or fit.
- Retain literal node block ceilings/sum, full proposal estimate and same-input actual separately; no raw Unzen-as-PGU.
- Keep64 users,31M header,<=30M charged includingAnchor,15M/user,2301 full-gross envelope reserve and all ordered prefix/sentry constraints.
- Root REVERT is a valid included failed transaction; discard effective refund and restore storage while retaining attempted work/nonce/gas.

## Review Focus

- Root REVERT clears refunds and caller-owned maps; false success expectations or private map rebinding can make the oracle lie.
- Reverted phase never advances between users; distinct target pairs differ from the committed restore mode.
- N0/small/full storage maps match; full signed envelope and proof/range bounds must match actual gas and terminal.
- Initial/final contract roots are not whole-world equality; gas fees,nonce andAnchor persist.
- Coverage inventory separates sampled families/references/promotion lanes and does not manufacture a completion percentage.

### Task 1: Producer, bounded rollback oracle and usage

**Files:**
- Create `bin/zkgas/bench/cases/storage_root_journal.py` and `bin/zkgas/bench/tests/test_storage_root_journal.py`.
- Modify `bin/zkgas/bench/tests/test_storage_mutation.py` only for bounded root REVERT, and `bin/zkgas/bench/README.md`.
- Commit supplied spec/plan with source implementation.

**Interfaces:**
- Consumes existing disjoint request/map builder,assembler/guards and bounded execute(code,state,envelope) test helper.
- Produces make_case(mode='restore-stop',accounts=8,state_size=8192,level='full'),gas_accounting(mode,n),full_n(mode,accounts,state_size),prepare(output,model_id);36-case catalog,truthful phase/refund/terminal metadata.

- [ ] TDD root rollback restores dictionary identity/contents,clears effective refund and retains preterminal trace; reject unsupported nonzero REVERT ranges without pretending full-EVM support.
- [ ] TDD exact36 grid,typed bounds,unique IDs/hashes,exclusive output/model/source identity and ownN0 storage/envelope pairing.
- [ ] Implement guarded runtime and exact gas/fullN solver from spec; derive count/phase/refund/prefix metadata from source of truth without old aliases.
- [ ] Independently execute all actual bytes for64 users,exact raw reads/writes/originals/warmth,refunds/sentry/terminals,maps/routes,phase repeats,distinct targets and N+1 maximality; reject meaningful wrong-semantics mutants.
- [ ] Add bounded native smoke for both successful restore and expected root REVERT,including failed-user receipts and original/final roots. Run new+relevant existing stdlib regressions,inspect full diff/hygiene and scoped Conventional Commit. No Rust/guest build.
- [ ] Independent source/spec review and all36 native/signature/state-root/witness/realquote gates precede actuals.

### Task 2: Frozen actuals, findings and coverage inventory

**Files:**
- Create `docs/research/2026-11-03-storage-root-journal.md` and `docs/research/zkgas-workload-coverage.md`.

**Interfaces:**
- Consumes reviewed36 inputs/native/realquotes,unchanged tools/model,and existing campaign evidence for inventory.
- Produces36 guarded actuals,literal metrics and ownN0/small-full/mode/state/account contrasts,coverage status and local recovery evidence.

- [ ] Root freezes source/artifacts/inputs/realquotes before36 sequential6GiB/600s actuals; minfree256MiB+320MiB retention reserve,no implicit retry.
- [ ] Independent raw audit validates gas10//191/input/output identities,literalnode/full totals,root-REVERT receipt/refund/phase semantics,all state roots and resource guards; separate exact144-name host SHM check.
- [ ] Report every deficit/control,>2B/30M priority,remaining topology/journal/ranking/usability limits; no pricefit,linear N0 projection or isolated actual attribution.
- [ ] Write linked coverage inventory with finite sampled versus still-open composition/production lanes; prior80 new inputs/36full are a separate prepanel snapshot,not IID confidence or percentages.
- [ ] Fresh independent whole-increment review,scoped docs commit,update current Draft/do-not-merge PR296 with exacthead/body readback. Preserve local data for separate Atlas publication; no merge/deploy.
