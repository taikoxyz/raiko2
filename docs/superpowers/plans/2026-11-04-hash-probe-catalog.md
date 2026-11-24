# Hash-probe catalog implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development. Execute continuously under existing user authorization.

**Goal:** Generate independently checkable concentrated keys and bounded intermediate-R workloads, then measure their actual pricing gap.

**Architecture:** A std-only Rust hash/search CLI produces strict data-only catalogs. The existing authoritative warm-read constructor accepts one optional intermediate R. Independent qualification gates actual execution, including verification that the new input realizes the intended hash concentration.

**Tech Stack:** Rust2024 std-only, existing Python case framework/managed venv, unchanged native/quote/SDK6.3 observer/b76/c893 artifacts.

**Spec:** `docs/superpowers/specs/2026-11-04-hash-probe-catalog.md`.

## Global constraints

- No model/price/SDK/guest/ELF/native-builder/pin/stable-helper/resource-cap changes or deployment/merge.
- Search max2^28 trials/120s, at most512 keys; default4/custom2 request+row/runtime/gas/envelope identities remain unchanged without the new option.
- New custom data is a development probe; old raw input/report/manifests stay immutable. No actual underpricing or seed-stability claim before measurement.
- Same6GiB/600s/576MiB disk and producer cadence. No full SP1 run before realized-concentration gate; no implicit retries.

## Review focus

- U64/U128 overflow and endian/limb placement must agree with independent actual hashes.
- Search exhaustion/deadline must not leave a misleading valid catalog or overwrite an existing path.
- Probe metadata must describe the executed R and retain the correct full envelope/zero baseline.
- Probe R is strictly intermediate (1 <= R < full); reject the duplicate full endpoint before creating output so zero/probe/full remain three unique sample IDs.
- New calldata/signatures can change seeds: old observed values are hypotheses, not a fixture guarantee.
- Source hash provenance may change while default request/row semantics remain byte-identical; do not force old metadata hashes.

### Task 1: Maintain search utility and probe-R construction

Own new `bin/zkgas/bench/hash_probe_catalog.rs`, focused case changes `cases/warm_read_catalogs.py`, focused tests `tests/test_warm_read_catalogs.py` plus optional `tests/test_hash_probe_catalog.py`, bench README and supplied spec/plan. Existing five helpers/native/SDK tool untouched.

- [ ] Add focused RED tests for formula/CLI/bounds/output installation and intermediate-R/default compatibility.
- [ ] Implement the std-only hash/search CLI and one-constructor probe factory/CLI.
- [ ] Run bounded rustc unit/CLI tests and focused Python byte/gas tests; no real-seed search/native/quote/SP1/guest/SDK work.
- [ ] Inspect complete scoped diff/hygiene, local Conventional Commit and freeze source/build/executable/default identities.
- [ ] Root dispatches fresh independent source/spec/quality review.

### Task 2: Independently qualify and measure

- [ ] Validate all512 real hash vectors and bounded failure cases before one frozen real search.
- [ ] Export zero/probeR512/full once, independently qualify native3/quote3/signature/root/intent/gas/quote-index identities and freeze policy/order.
- [ ] Actual zero first; verify realized concentration using the new input's own historical values. Retain a non-concentrated result rather than silently changing it.
- [ ] Actual probe next only after that gate; compare literal node/fullproposal against canonical SP1 PGU and classify >2B/<=30M severity. Launch full only when justified by measured data.
- [ ] Retain report/fixtures/usage and independent raw/source/host closure, whole-increment review, held-Draft update and working.log/all rulings/own scratch closure. Pricing work follows separately if warranted.
