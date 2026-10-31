# BLOCKHASH Density Plan

**Goal:** Measure twelve dense BLOCKHASH inputs against frozen model2fb.

**Architecture:** One Python producer/test module, existing assembler and fixture
interfaces; reuse all frozen host/program artifacts and resource guards.

**Spec:** [finite experiment](../../research/2026-10-31-blockhash-density.md).

- [ ] Test first; implement `blockhash_density.py`, focused tests and bench usage.
  Preserve N+1 total events, semantic warmup and fixed 64-query body at N0.
- [ ] Independent source review and native/quote qualification, including
  ancestor/target checks and native-only boundary/rejection controls.
- [ ] Commit/freeze producer and canonical inputs, run actual SP1 once per
  input, independently verify source/model/output/PGU joins and owned cleanup.
- [ ] Document numerical results and retain data in Atlas with precise original
  hashes/projections. Update guarded source PR records; no merge or deployment.

RISC0 CI is explicitly outside this task per user instruction. No deletion,
fingerprint bypass, guest build or production dependency change is requested.
