# Storage/hash composition implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development with independent source
> review and behavioral verification. Execute the already-authorized experiment without new
> approval pauses; no PR merge, guest build or deployment.

**Goal:** Retain and measure45 new finite composition inputs to test scalable node underpricing
from cheap dirty writes, hashes, frame rollback and equivalent-work packing.

**Architecture:** A Python ProgramRequest producer uses the existing assembler and native
constructor. A shared guarded kernel supports direct execution or one self-DELEGATECALL with
RETURN/caught REVERT. Existing quote/sampler/guards collect both surfaces and identical-proposal
actual workload; there is no new runner, input schema or native implementation.

**Tech Stack:** Python standard library, EVM assembly, existing frozen SDK6.3/ELFb76 tooling.

**Spec:** `docs/superpowers/specs/2026-11-01-storage-hash-composition-design.md`.

## Global constraints

- Freeze modelc893/64PGU, ELF/native/quote/sampler identities and source/request/catalog hashes
  before opening actuals. No coefficient, discount, limit, SDK or guest change.
- Preserve all earlier producer/request bytes; use fresh output roots. Keep node per-block,
  literal sum and aggregate-ceil; full estimate and proposal actual with both deficit flags.
- Existing15M per-user/31M block EVM limits remain; explicit evm-gas-only profile excludes the
  old Unzen admission cap. No resource-guard failure receives a PGU value.
- Sequential sampling:6GiB PSS,600seconds,192MiB output,16MiB log, min free512MiB. Reuse frozen
  artifacts and shared Cargo target only if an unforeseen source change needs a justified build.
- Existing Python environment only; no installs or broad new build graph. Portable artifacts
  contain no operator paths, credentials, human labels, tool implementation or binaries in Atlas.

## Review focus

- Reverting child work must really execute and roll back; status/marker/count/gas gates agree.
- All bulk SSTOREs use slot2 and nonoriginal7/11, rather than gas-expensive original restores.
- KECCAK uses32 zero bytes; its memory is disjoint from counter/input/output, including direct.
- Aggregate counts multiply all users/blocks; their original storage remains0 at every user entry.
- Wrong operands with unchanged receipt/counts need an independent concrete oracle; short/OOG
  runs must fail native completion and never be assigned successful measurements.

### Task1: producer, regression coverage and usage

**Owner/files:** implementer owns `bin/zkgas/bench/cases/storage_hash_composition.py`,
`bin/zkgas/bench/tests/test_storage_hash_composition.py` and its README section only.
**Interface:** `make_case(kernel='dirty', layout='direct', rounds=2048, transactions=1, blocks=1)`
returns the existing catalog row+ProgramRequest; `prepare(output, model_id=MODEL)` emits45 requests.

- [ ] Write tests for matrix, domains, own baselines, source bindings, no-overwrite and semantic
  stack/memory/storage/return/rollback behavior; observe the intended missing-producer failure.
- [ ] Implement the smallest assembler producer, fixed body/widths and explicit count scopes.
- [ ] Run focused Python/native positives and negatives with the frozen constructor. Include a
  wrong operand that can pass receipt/counts but fails the independent oracle.
- [ ] Preserve previous inputs; run Python framework checks, inspect complete diff, independent
  source review, and commit only owned coherent changes after confirmed findings are resolved.

### Task2: native qualification and actual sampling

**Owner:** independent tester qualifies all45 requests, canonical inputs, outputs, counts,
transaction nonces, storage state and code/memory semantics with its own oracle.

- [ ] Freeze and independently verify inputs/source/artifacts before actuals.
- [ ] Root samples the45 qualified inputs once using existing workload.measure sequentially.
- [ ] Independent verifier rejoins raw input/native/quote/SP1 reports, normalization, both
  rounding conventions, actual underpricing, matched increments, resource guards and cleanup.

### Task3: research checkpoint and immutable retention

**Owner:** root writes evidence-based research and PR update; Atlas data preparation is separate.

- [ ] Explain scalable gaps versus absolute residuals and record all controls/failures.
- [ ] Obtain independent source/report/archive review. Preserve raw report bytes and exact
  declared auxiliary projections; verify compressed/decoded/member checksum closure.
- [ ] Publish new immutable data and a reviewed PR with actual remote-download verification.
  Keep all three source PRs Draft/do-not-merge and read back every completed GitHub update.
