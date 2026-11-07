# BLOCKHASH Density Plan

**Goal:** Measure twelve dense BLOCKHASH inputs against frozen model2fb.

**Architecture:** One Python producer/test module, existing assembler and fixture
interfaces; reuse all frozen host/program artifacts and resource guards.

**Spec:** [finite experiment](../../research/2026-10-31-blockhash-density.md).

- [x] Test first; implement `blockhash_density.py`, focused tests and bench usage.
  Preserve N+1 total events, semantic warmup and fixed 64-query body at N0.
- [x] Independent source review and native/quote qualification, including
  ancestor/target checks and native-only boundary/rejection controls.
- [x] Commit/freeze producer and canonical inputs, run actual SP1 once per
  input, independently verify source/model/output/PGU joins and owned cleanup.
- [x] Document numerical results and retain data in Atlas with precise original
  hashes/projections. Update guarded source PR records; no merge or deployment.

RISC0 CI is explicitly outside this task per user instruction. No deletion,
fingerprint bypass, guest build or production dependency change is requested.


Completed experiment records: Raiko tools PR #296, maintained Reth PR #260 and
Atlas fixture PR #21 / `fixture-unzen-blockhash-density-v1`. Original fitting
and prospective cohorts, native quotes and independent resource/data audits
are retained. The later provenance/semantic correction is recorded separately
in [the semantic audit](../../research/2026-10-31-calibration-semantics.md);
it does not rewrite the published fixture or change the frozen candidate.
