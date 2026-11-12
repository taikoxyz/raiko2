# Dense BN254 Pairing Implementation Plan

Use superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-11-03-bn254-pairing-dense.md`. Base published `cd2e2fc99428f8be48f94e73bde71a4cb7b44c63`. Existing isolated worktree; preserve dirty main and all prior datasets. This follows completed caller-warm32 and transient8 no-repricing panels.

### Task 1: Add reusable dense-pairing producer and oracle

**Files:** create `bin/zkgas/bench/cases/bn254_pairing_dense.py`, `bin/zkgas/bench/tests/bn254_pairing_vm.py`, `bin/zkgas/bench/tests/test_bn254_pairing_dense.py`; append `bin/zkgas/bench/README.md`; include supplied spec/plan. Exactly six tracked files. Existing helper/oracle sources unchanged.

- [ ] RED first for kind/pair/level domains, independent point/encoding/product intent, actual copy/call wrapper and gas/envelope/full+1 constraints.
- [ ] Implement minimal18-case emitter, separate bounded byte oracle, source/helper/request provenance and CLI. No duplicate authoritative G1 generator; tests independently validate group/point intent.
- [ ] All18/two-user trajectories, zero/single/full and boundary/negative checks; frozen-native three specimens with complete retained evidence, including accepted reorder mutant. No build, guest, compiler, quote or SP1.
- [ ] Full six-file diff/added-line hygiene/links/source checks; Conventional Commit, no push. Fresh reviewer and independent verifier complete exact-head source gate before canonical sampling.

### Task 2: Qualify, freeze, sample and record

**Files:** create `docs/research/2026-11-03-bn254-pairing-dense.md`, update `docs/research/zkgas-workload-coverage.md`, append working log. New private Q/R/archive only; previous cohorts remain read-only and separately counted.

- [ ] After final source PASS, qualify18 canonical inputs/36 signatures/valid dense operands/wrapper gas/code/account/root/witness/receipts/counters and18 original real quotes; retain positive native smoke through explicit source bridges.
- [ ] Freeze all18 inputs/tool/model/source/resource identities; actuals once sequentially with zero/single phases first and resource/failure receipts preserved.
- [ ] Independent raw18/diagnostic27/host72 closure, absolute/incremental gaps and real gas/occupancy; investigate >2B/30M deficits before selecting any tariff. No extrapolated global guarantee from single-call evidence.
- [ ] Verified recovery archive, concise research/coverage/usage record, fresh broad final review, own Draft296 push/body/readback retaining do-not-merge; no-price conclusions working log only, no human checkpoint.
