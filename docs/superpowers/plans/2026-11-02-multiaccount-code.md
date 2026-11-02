# Multi-account code loading implementation plan

> **For agentic workers:** Use subagent-driven development with independent task
> review and behavioral verification. User approval covers implementation/sampling
> and the existing source/data PR workflow. Do not ask to continue between tasks.

**Goal:** Qualify and measure36 multi-account shared/unique-code inputs under
unchanged pricing using honest minimal GuestInputs.

**Architecture:** One optional host fixture prestate mode, followed by a stdlib
Python case generator. Existing quote/report/SP1 measurement interfaces remain.

**Tech stack:** Rust host fixture, Python stdlib/unittest, current GuestInput,
retained SDK6.3 ELF and quote modelc893.

**Spec:** `docs/superpowers/specs/2026-11-02-multiaccount-code-design.md`.

## Global Constraints

- Preserve default old request behavior/4MiB bound and exact legacy output bytes.
- New optional prestate_contracts is host-only; no guest/shared input/SDK change.
- At most512 extra contracts,24576 bytes each;32MiB new-mode request cap.
- One block, explicit CALL users; exclude conflicting storage/auth/access/Create.
- Derive final minimal proofs/code from actual native access; no padded witness.
- Same code hash deduplicates bytes/analysis, not account state or CALL execution.
-36 inputs,18 controls; full99%..100% of30M charged gas with31M native header.
- Freeze modelc893/ELFb76 and the reviewed new native before sequential actuals.
- Keep literal node/full/actual separately; strict>2B missing node PGU is high.
- No price/limit/deployment/merge; retain all failures and resource receipts.

## Review Focus

- Background/unused account code accidentally included in N0 witness.
- Shared hash dedup concealing distinct account proofs or full executed calls.
- Missing account/storage targets after pruning, including mandatory system work.
- Declared-address collision, request/code limits and partial output on failure.
- Full-gas arithmetic versus cache/warmth reset, returned-address guards and salt.

### Task 1: Extend the host fixture prestate mode

**Files:** `bin/zkgas/src/fixture/mod.rs`, `constructor.rs`, `observer.rs` and a
new `prestate.rs` module as needed; `bin/zkgas/tests/fixture.rs` or dedicated
feature-gated integration test; `bin/zkgas/bench/README.md` usage fragment only.

**Consumes:** Existing ProgramRequest/schema1, ControlledTrieState, native
executor/cache/observer and minimized MPT proof facilities.
**Produces:** Optional `program.prestate_contracts: [{address,code_hex}]`, strict
validation, authentic minimal GuestInput export and optional report diagnostics.

- [ ] Write focused failing tests for shared/unique/unused contracts, invalid
  addresses/code/size and missing target/code; then implement the one optional mode.
- [ ] Reuse actual execution cache/read sets to select proof/code; rerun minimized
  input and check roots/outputs/counts. Preserve old-mode bytes without filtering it.
- [ ] Run targeted package fixture tests/Clippy/format using primary shared target.
  Build only the host native fixture and compare representative legacy inputs
  with frozen b049. Record source/binary identities and legacy compatibility.
- [ ] Independent reviewer inspects the complete task diff; fix/recheck material
  findings. Root commits coherent source; no worker push or guest build.

### Task 2: Add the multi-account loading case producer

**Files:** Create `bin/zkgas/bench/cases/multiaccount_code.py` and
`bin/zkgas/bench/tests/test_multiaccount_code.py`; root integrates usage.

**Consumes:** Task1 prestate list/native binary, existing Python assembler and
ProgramRequest envelope helpers.
**Produces:**36-case catalog with exact own-N0/full pairs, fixed code/operand
guards, independent bounded gas/count/target oracle and native-only negatives.

- [ ] Write expected-failing tests; implement account-count/code-length/shared-
  unique matrix. Keep executed prefix identical and code salts unreachable.
- [ ] Solve exact full charged gas and uniform envelopes; test all prefixes,
  cache first/repeat visits and four-transaction warmth resets.
- [ ] Run focused stdlib and small native/quote/retained-ELF compatibility probes.
  Independent source review fixes confirmed failures before source freeze.

### Task 3: Qualify and freeze all inputs

- [ ] Independent tester validates all36 native/quote inputs, raw source hashes,
  minimal witnessed addresses/hashes/bytes, body/return/counter semantics, signed
  users and all receipt/gas/envelope values; preserve negative and cache controls.
- [ ] Root runs full stdlib suite and relevant host regression checks, commits
  final coherent source, and freezes configuration/new native/36 canonical inputs.

### Task 4: Measure actual workload and independently audit

- [ ] Root sequentially runs existing bounded SP1 measurement without guest build
  or price changes. Failed/censored samples retain null actuals and all attempts.
- [ ] Independently audit raw normalization/joins/public outputs/literal and full
  totals/18 matched increments/severity/resources. Source/inputs stay immutable.

### Task 5: Retain tools and results

Adaptive scope: retain the completed36 CALL observations and add36 separately
frozen DELEGATECALL observations after detecting confirmed-CALL reserve masking.
The spec's additional-comparison section is authoritative. The follow-up source
uses the same generator, host input format and frozen native/model/ELF. Independent
source review, all36 native/quote qualifications and actual auditing apply to
both panels. The final fixture retains72 observations with per-panel source and
configuration identities; no existing result is overwritten or repriced.

- [ ] Write factual research/usage with overquotes, controls and finite-scope
  limits. Update #296 while keeping Draft/do-not-merge.
- [ ] Prepare new data-only Atlas fixture, independently review preseal/metadata/
  archive, verify fresh GitHub download with existing workflow, publish immutable
  asset/data PR and read back exact submitted heads/bodies. No automatic merge.
