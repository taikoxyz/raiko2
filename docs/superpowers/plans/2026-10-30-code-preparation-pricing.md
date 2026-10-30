# Code preparation pricing implementation plan

> **For agentic workers:** use implementation ownership plus independent source review and behavioral verification.

**Goal:** Correct existing-code and per-occurrence deployment preparation pricing from the completed SP1 observations.

**Architecture:** Existing unique-code bytes get a data-derived unit64; common real frame-return collection adds nonempty code-preparation events/bytes priced8192+32/B. Reuse the current committed/attempted and node/prove boundaries.

**Tech Stack:** Maintained Reth pricing branch, existing generator/seal workflow, Raiko native quote, fixed SDK6.3 estimator.

**Spec:** `docs/research/2026-10-30-code-preparation-pricing.md`.

## Global constraints

- Policy64 unique/B and8192/nonempty-event+32/processed-B fixed before16-row prospective execution.
- All other coefficients, global discount, DefaultDisabled, Unzen behavior and GuestInput stay unchanged.
- One authoritative processing hook/helper; no trait-bound break or duplicate collection.
- No guest build, model hand-editing, automatic merge or deployment.

## Review focus

- Successful processing occurs per create, including repeated hash and ancestor-reverted work.
- Early failure/empty code/nonce overflow must not invent processing bytes.
- Plain/inspected parity and collector movement must charge exactly once; semantics-changing inspectors are outside quantitative contract.
- Checked selector/prefix updates must fail diagnostically and atomically without changing execution.
- Historical/native evidence and old-program identities must remain separate from new pricing reports.

## Tasks

- [x] Reth: add failing tests for raw event/byte accounting, both execution paths, duplicate hashes, nested/outer revert, failed/empty creation, collector reset/discard and bounds. Implement the shared hook/helper and deterministic successor generator; run focused EVM/block/generator/no-std/Clippy checks and independent review/verification before commit/push.
- [ ] Raiko: pin the reviewed Reth commit without guest rebuild; add required node/prove selectors and contribution once, proposal-once zero; validate consistency and missing-field/forgery rejection. Run package/workspace checks and independent actual native replays.
- [ ] Generate/qualify the frozen16-row prospective set, execute the unchanged program, audit independently, and reprice all92 original rows plus50 historical inputs. Report actual results and limits before publishing the corrected-model conclusion.
- [ ] Preserve all original defects and actual successor quotes in a reviewed data-only Atlas fixture; keep reusable tools/usage in Raiko and leave production artifact alignment as a separate Draft merge gate.


## Verified upstream checkpoint

Reth correction `8da28fd02e44d81d2c84725fbd082f73cd48428e` is pushed to the maintained
Draft #260 branch. Model identity is
`a2988920125e304121acb629b9b96fd1ad85ee707c89978240408c81e2a42f3e`.
The complete 18-file source diff passed independent adversarial review. Independent
behavioral verification covered 95 real EVM transaction cases across six probes,
plus real block/receipt/state/bundle and signed-payload lanes. Final-source
no-default compilation and deterministic generator/174-route checks passed.

This checkpoint does not claim Raiko actual successor workload closure. Original
SP1 measurements, prospective separation and historical program identities remain
unchanged; downstream checks and actual repricing are the next gates.
