# Storage Root Diversity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Test heterogeneous storage witnesses at unchanged block gas against frozen node/full pricing.

**Architecture:** A separate producer composes existing fanout requests and runtime, replacing only configured storage maps and truthful profile metadata. Independent native/quote/root/signature qualification precedes a new frozen32-point SP1 panel and immutable old-reference comparisons.

**Tech Stack:** Python standard library and existing native/quote/SP1 binaries; no Rust or guest change.

**Spec:** `docs/superpowers/specs/2026-11-03-storage-root-diversity.md`

## Global Constraints

- Keep c893/native/quote/SDK6.3/ELF/model/pins/GuestInput/guest and old producer/data bytes unchanged; no guest build or fit.
- Preserve64 users,31M header,<=30M charged inclAnchor,<=15M/user and every prefix envelope reservation.
- Literal node sum/full proposal/actual remain separate; no raw Unzen-as-PGU or per-operation actual attribution.
- New32 inputs are unfitted holdout; previous homogeneous16 unique references remain old observations with explicit provenance.

## Review Focus

- Own-N0/full must salt by fullN, not currentN, so initialized storage remains matched.
- Different roots do not imply no hash reuse; quantify actual serialized/proof sharing.
- Shifted phase values and keys must be truthful in all metadata and semantic/root oracles.
- Same EVM gas is independently observed, including originals/refunds/prefixes; native success alone cannot establish intended semantics.
- Reference actuals must use exact same program/tool identities and remain labelled reused, never fresh executions.

### Task 1: Reusable diversity producer, semantic tests and usage

**Files:**
- Create `bin/zkgas/bench/cases/storage_root_diversity.py` and `bin/zkgas/bench/tests/test_storage_root_diversity.py`.
- Modify `bin/zkgas/bench/README.md`; commit supplied spec/plan with implementation.

**Interfaces:**
- Consumes storage_fanout.make_case/full_n and storage_mutation runtime/gas accounting.
- Produces make_case(profile='background-values',accounts=8,state_size=8192,mode='update',level='full')->dict; prepare(output:Path,*,model_id=MODEL)->int;32-case catalog and canonical requests.

- [ ] TDD exact2x2x2x2x2 grid, literal ID/reference/baseline/source identities, strict typed inputs, model-ID CLI and exclusive output creation.
- [ ] Implement maps exactly from spec, preserve old producers/runtime/gas and derive per-account phase/range metadata without stale inherited claims.
- [ ] Independently execute actual bytes for all32: exact declared owner/order, raw reads/writes/originals/warmth, refunds/caps, counter/checksum/phase, full final maps/counts, routes and envelopes.
- [ ] Check N0/full initial maps identical, unchanged common runtime/body values/gas, phase shifts explicit; reject collapsed/wrong profile/owner/range/phase/tail salts even for successful native/interpreter execution.
- [ ] Add optional bounded native smoke and portable usage. Run focused plus relevant existing generator/harness regressions, inspect complete diff/hygiene and commit source/spec/plan. No Rust/guest builds.
- [ ] Independent source review plus all32 native/signature/root/witness/realquote gates precede actuals.

### Task 2: Frozen actuals, reference comparisons and findings

**Files:**
- Create `docs/research/2026-11-03-storage-root-diversity.md`.

**Interfaces:**
- Consumes reviewed producer,32 independently qualified canonical inputs/originalquotes, frozen tools and16 unique homogeneous raw-reference observations.
- Produces32 guarded actual observations, literal metrics, own-N0/profile/state/account/reference contrasts, private recovery archive and precise finite research.

- [ ] Freeze source/inputs/realquotes/tools and old reference index before actuals; preserve minfree256MiB plus320MiB retention reserve. Root orchestrates32 sequential6GiB/600s actuals, no implicit retry.
- [ ] Independent raw audit joins SDKgas10//191/input/publicoutputs/retainedquote provenance/receipts/roots/literalnode/full totals/resources and reference identities; verify all owned SHM absent separately.
- [ ] Report all literal deficits and >2B priority at valid gas, root/hash sharing and quoted selectors, ranking/overquote/usability limits and explicit phase-value differences. No refit or universal-bound claim.
- [ ] Independent final whole-increment review, scoped research commit and update existing Draft/do-not-merge PR296 with exacthead/body readback. Preserve immutable local evidence for separate data-only Atlas publication; no merge/deploy.
