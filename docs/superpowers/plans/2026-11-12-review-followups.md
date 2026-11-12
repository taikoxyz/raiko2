# zkGas review follow-ups implementation plan

> **For agentic workers:** Use the independent engineering workflow in AGENTS.md. Complete source review and behavioral verification before updating the maintained Draft PRs.

**Goal:** Resolve the concrete review findings and compare the maintained model with both original Unzen and a transparent, incrementally patched Unzen baseline.

**Architecture:** Keep the current research guest unchanged. Reth owns runtime observation and prices; Raiko owns native GuestInput replay and composable evaluation tools; Atlas retains immutable data. Preserve all predecessor evidence and distinguish numerical-model, extractor, executable and program identities.

**Tech stack:** Rust host code, Python standard-library analysis, existing SP1 execute-only observations.

**Spec:** User-approved review triage in the session: cover exceptional execution, reconcile PR/artifact scope, restore current-profile reproducibility, investigate common tariffs and completion, then freeze and validate. The user additionally approved a patched-Unzen offline baseline without assuming which model will win.

## Constraints

- No container/hasher replacement, guest build, production activation, fee change or deployment.
- Keep #260/#284/#296 Draft and do-not-merge. Do not merge GitHub PRs.
- Preserve dirty primary checkout and unrelated untracked worktree files.
- Do not treat raw Unzen as PGU or split multi-block actuals into block truth.
- Do not refit on validation observations or erase opened/failed/control rows.
- Reuse targets, one build job, bounded processes; avoid large new dependency graphs with limited disk.
- Public reproducibility-material policy remains undecided; do not change repository visibility or remove history.
- The new Reth-main OP 2.6/Rust 1.99 upgrade is a separate scope decision. Raiko main remains Rust 1.95 and pins the compatible Reth lineage.

## Review focus

1. Actual OpcodeNotFound must be priced without swallowing unsupported successful instructions or collector failures.
2. Failed/discarded candidates must not leak into committed node selectors; executed work must remain attributable on the proving surface.
3. Restored main artifacts and provenance must be the same exact group; preserved bytes do not imply current-source freshness.
4. Research profiles must bind real program/version/model identities and must not inherit an unrelated historical normalization.
5. Patched Unzen must use observable features and declared units, never actual-based per-row corrections or silently favorable subsets.

## Task 1: Integrate existing main behavior and separate artifacts

**Files:** Raiko manifests/locks, `.github/workflows/ci.yml`, `bin/guest-launcher`, `crates/guests/elf`; Reth integration conflicts.

- [x] Record original remote heads and local backup refs.
- [ ] Incorporate reviewed compatible upstream behavior, retaining tx-selection overflow semantics.
- [x] Merge launcher optional quote and ordinary/SDK-smoke behavior; preserve main CI fixes and shadow lanes.
- [x] Restore the entire generated ELF/VK/provenance group from the exact Raiko base without rebuilding or editing its contents.
- [ ] Generate minimal compatible lock updates, verify dependency deltas and artifact blob identity.
- [ ] Run focused launcher/trace/tool checks and document artifact freshness as an unclosed activation gate.
- [ ] Independent review and verification, then update existing Draft PRs with exact remote-head guards.

## Task 2: Exceptional-execution coverage

**Files:** Reth `crates/evm/src/zk_gas/runtime.rs`, focused tests and pricing documentation.

- [x] Reproduce top-level and caught-child OpcodeNotFound diagnostics in focused tests.
- [x] Route only confirmed undefined-instruction exceptional execution to an explicit sealed error price.
- [x] Keep unsupported successful routes diagnostic and preserve actual EVM opcode/Unzen/gas/status/state behavior.
- [x] Verify committed/attempted ownership and plain/inspected parity.
- [x] Record extractor semantic revision separately from unchanged numerical JSON prices.
- [ ] Independent review/test and integrate the reviewed Reth revision into Raiko.

## Task 3: Current profile and reproducible evaluation

**Files:** `bin/zkgas/profiles`, `bin/zkgas/src/compare.rs`, focused compare/report tests and README.

- [x] Add a current research profile binding the actual b76 program, SDK 6.3.0, execute stage and current numerical model.
- [x] Preserve explicit historical profile access; remove the stale historical profile from the current default path.
- [x] Test matching research input and mismatching model/program/backend/stage; never fabricate VK identities.
- [ ] Provide reusable train/evaluation metric calculation and commands for the retained cohort.
- [ ] Reproduce headline results through checked-in tools, with exact row selection and fixed scales.

## Task 4: Patched-Unzen baseline

**Files:** New `bin/zkgas/bench/targeted_unzen.py`, focused tests; minimal host semantic-selector export if required.

- [ ] Freeze an explicit rational conversion/profile and calibration/validation identities before evaluation.
- [ ] Retain original raw Unzen and its original budget separately.
- [ ] Implement declared additive stages: scalar-sensitive ECMUL; raw access-list work; optional container reserves as a separate complexity increment.
- [ ] Obtain semantic counts from replay, including actual precompile dispatch/success/failure and committed scope; never infer history from case names or quote prices.
- [ ] Treat missing features as unavailable with explicit coverage; compare models on named identical intersections.
- [ ] Report finite capacity/packing, workload-shape and complexity deltas without claiming hard admission or universal bounds.
- [ ] Test units, exact arithmetic/rounding, missing data, duplicate identities and invalid training/evaluation overlap.
- [ ] Independently replay retained data and evaluate against original Unzen and current node estimate.

## Task 5: Targeted pricing and completion investigation

**Files:** Existing controlled producers, Reth price generators, research reports.

- [ ] Retain existing prices while isolating ORIGIN/normal REVERT/CALL/ECRECOVER placeholder effects.
- [ ] Quantify KZG/BLS availability under explicit illustrative limits; distinguish constructed extremes from historical/product requirements.
- [ ] Attribute block-MPT completion costs and identify missing phase evidence; do not delete completion solely because the node aggregate is near actual.
- [ ] Propose only supported local price changes, preserving heavy/error/mixed regressions and precursor results.

## Task 6: Freeze, validate and retain evidence

- [ ] Freeze the final candidate and all baseline profiles before new validation truth.
- [ ] Use application-bearing, untouched inputs; reuse opened cohorts only as development regressions.
- [ ] Report node/proposal/actual separately, literal shortfalls, historical packing, model-complexity cost and limitations.
- [ ] Retain fixtures in Atlas and tools in Raiko; update working log, PR summaries and knowledge references with actual completed results.

## Progress ledger

- Initial scope approved by the user; numerical model801 and b76/SDK6.3 evidence retained as the predecessor.
- Independent artifact audit confirmed #284 introduced 14 artifact/provenance changes and the default comparison profile is stale.
- Exact Raiko main82d9a7f remains Rust1.95/SP1 6.8.1/compatible OP-Reth revision. Its merge is distinct from the newer Reth-main Rust1.99 upgrade.
- Reth `eff4ed4a` independently reviewed and pushed to #260: 293 EVM tests and 77 pricing tests passed; numerical JSON unchanged. Compatible upstream checkpoint is c874, not newest OP/toolchain main.
- Raiko #284 `475adcc2` independently reviewed and pushed: entire artifact directory equals base82d9a7f; metadata and 23 RISC0/44 opcode experiment tests passed (one optional opcode skip). Launcher compilation hit its bounded 180-second limit before assertions; not a passed suite.
- Raiko #296 integration resolves only the reviewed Reth revision on main-based host locks; all three locked metadata graphs pass. Current research profile's complete comparison suite passed 71/71 after a bounded 4m14s host build.
- Independent review found and fixed the SDK-smoke/quote bincode gate and preservation of existing RISC0 experiment CI coverage. Patched-Unzen aggregate memory bounds are being fixed before real-panel use.
- Reclaimed one verified duplicate uncompressed archive, retaining compressed Release bytes and extracted data. No fixture, actual, guest or shared Cargo cache was deleted.
