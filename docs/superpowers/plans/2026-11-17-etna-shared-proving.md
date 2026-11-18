# Raiko2-facing Etna library and integration plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement task-by-task. Steps use checkbox syntax. Root coordinates the repositories and shared build target; one implementer owns a coherent task at a time.

**Goal:** Verify already known Etna blocks using shared checks where useful and Raiko2's existing EVM, witness and proving infrastructure, with minimal fork-specific changes.

**Architecture:** Keep the reviewed `etna-core` checkpoint for shared consensus computations and verification. Check supplied blocks through existing Raiko2 execution paths, adding only narrow missing Etna checks/adapters. Reuse affordable mining computations; verify their outputs when reuse is unsuitable or zk-expensive. Proof orchestration may remain in existing Raiko2 modules; an upstream `etna-proof-core` crate is optional, justified by actual reusable checks.

**Tech Stack:** Rust, alloc-capable protocol core, existing alloy/prost/Comet codecs, alethia-reth stateless execution, RISC0 3.0.5, SP1 6.8.1, Solidity ABI/shared vectors.

**Spec:** `docs/superpowers/specs/2026-11-17-etna-shared-proving-design.md`. The user selected the shared-library/guest-committee direction on 2026-11-17; builder check found no remaining user decisions. This replaces #295's L1 committee-authority proposal for this implementation.

## Historical partial scope correction (superseded below)

The upstream milestone delivers the library Raiko2 needs to reference. Further
taiko-client-rs changes require a concrete Raiko2 consumer, necessary extraction
wiring, or a correctness/regression fix in the extracted surface. Re-exports and
small adapters may preserve the existing client interfaces; restructuring
ABCI/lander/mining/Engine/state or unifying their orchestration is out of scope.

Keep consensus-check composition in existing Raiko2 guest modules unless an
identified caller needs a small shared helper. A complete shared transition
framework is not an upstream prerequisite. Checkpoint-start inputs may be narrowed
without moving the lander's replay workflow. RegistryProof transport adaptation is
conditional on actually using it; the RPC proof-count comment is non-blocking
until that consumer requirement is demonstrated. Snapshot authentication and all
protocol checks remain mandatory whichever read path Raiko2 uses.

The #22398 review revision is `61d27093f8970a24eca8d0dc3d711a034c9cea95`.
It changes four existing files for the exported generation-start guard, preserved
host fault classification, removal of redundant decoding, tests and API docs.
Independent source and behavior checks passed: core 60, lander 327 (one resource
case ignored), four independent groups, strict core lint/docs and both actual
zkVM target compiles. Full `just verify` stopped at unavailable Go in the isolated
environment; broader host consumer/lint and Docker gates remain outstanding.
The draft was updated and external review re-requested; it is not merge-ready.
Actual-lockfile guest handover/cut fixtures are a downstream integration gate,
not permission to start integration while upstream API review is open. The later
full-prover tasks below do not enlarge this upstream milestone.

## Entire-PR boundary at the current review head

Review head `1d6d1666afec8795cf5203668317a534b339367f` replaces the earlier
72-file extraction with 31 new etna-core files plus only workspace Cargo.toml and
Cargo.lock changes. All existing client/CLI/config/native tests/fixtures/generators
match base `ea74662169262dc8ab6ada7028d2ef54137088d2`; no host adoption,
re-export, type/config migration, adapter or constructor change remains. Every
future upstream addition in this milestone must preserve that entire-PR boundary.

This is a temporary independently callable port with pinned provenance and copied
stock data, not a library already consumed by the node. Client adoption is separate.
Do not reintroduce the reverted migrations to satisfy historical task text below.
Final independent checks: core61, extra witness-shape controls, unchanged native
metadata/lock/fixture identities, strict core lint/rustdoc and both zkVM library
targets pass. Whole workspace/Docker/proof integration remain outside this evidence.

## Mining/proving boundary (user clarification)

The user confirmed a hybrid on 2026-11-17. The guest knows the committed blocks;
its obligation is protocol validity, not reproducing a proposer's choices. For
each proposed extraction, record whether it is unnecessary build policy, a useful
low-cost computation to reuse, or a relation to verify using supplied witnesses.
Choose using relevant RISC0/SP1 cycles, witness size, memory and dependency cost;
no optimization benefit is established by source shape or host timings alone.

- Pool selection, candidate ordering/retry policy, Engine payload IDs, compression
  generation, blob packing and submission scheduling stay outside the guest.
- Share protocol constants, canonical encodings and verifiers. Equivalent producer
  and verifier algorithms are allowed; validate them against the same rules and
  vectors rather than imposing one code path everywhere.
- Reuse the existing EVM replay, CachedTrie/witness/database, crypto and post-state
  root pipeline. No large EVM architecture change, backend relocation or mandatory
  new execution framework is part of this plan.
- Keep #22398 at its reviewed scope while waiting for external review. Its existing
  committee/ValState computations are reusable; no alternate algorithm is required
  merely to rename them verification.

## Shared consensus review gate

This revised plan and its linked design are the active execution baseline.
Start new upstream consensus work from the Etna client source baseline recorded
in the design, without the #22390 MPT prototype commits. The prototype branch
and evidence are historical records, not code to carry into the new base.

Preserve Raiko2's existing CachedTrie, witness pooling/materialization and
post-state-root implementation. The user rejected a low-level backend switch;
the common library should focus on BFT/QC, committee derivation, scheduling and
ValState. Generic state authentication uses narrow existing-backend adapters.

Task 1 is an archived MPT feasibility prototype, not a dependency of Tasks 2–7.
The closed prototype [taiko-mono #22390](https://github.com/taikoxyz/taiko-mono/pull/22390)
at `8ed9c271da7fff80e6698754e9da21a7466cf6ea` remains separate from Raiko2's
integration base. Do not repeat/expand it or adopt its MPT backend for this plan.

The next upstream review concerns the shared consensus API: Task 2 plus the native
Comet/QC/body checkpoint of Task 3. Envelope, FI and full transition verification
remain downstream obligations; extract additional helpers only for demonstrated
Raiko2 callers. QC/body alone is not a complete Etna block verifier.
Wait for that review to converge before substantial Raiko2 integration. Prefer
an exact commit from the merged Etna development mainline; a reviewed branch commit is a temporary
option. Keep Raiko2 in its own branch and record code revisions for later
implementation-conformance work. The separate TLA+/Lean work is untouched.

## Global constraints

- Keep the existing Raiko2 trie, witness and execution database implementation. Only fork-specific behavior required by Etna may change through narrow adapters; no generic backend replacement or broad extraction is part of this plan.
- The node and guest implement the same Etna validity rules. Share useful primitives/verifiers; allow equivalent specialized verification algorithms. Mining logic does not automatically become an upstream API, and proving orchestration may stay in Raiko2.
- Begin at the exact four source pins in the spec. Exact development-branch commit pins are authorized; no moving branch dependency in published manifests/locks.
- Preserve existing nine-field LandInput and 23-word statement unless a concrete omission requires a separately reviewed protocol change. Prover input/transport is a distinct interface.
- Every final backend proves the complete same statement. Initial pair: RISC0 + SP1. No SGX implementation, new zkVM, IVC, zkGas repricing, production activation or deployment is implied.
- No cross-Unzen/Etna batch: stop/settle/restart at H+1. Normal Etna committee changes, recovery, overlapping settlement and FI are included.
- Every production Rust symbol is documented. Never edit generated ABI/ELF/model artifacts manually. Never introduce machine paths or personal labels into tracked artifacts.
- Tests/builds execute only after entrypoint/dependency inspection, with isolated source, clean environment, no network/credentials/original Git access and one configured writable shared host target. Dependency fetch is a separate bounded step. Guest targets remain SDK-specific, using existing cache mounts/configuration rather than new per-worktree target trees.
- Root coordinates cache locks, build jobs and disk checks before/after costly steps. No blanket cleanup, deleting other tasks, repeated whole builds or changes in other agents' worktrees. Checkpoint coherent commits and ledger after each stage.
- Independent adversarial review and behavioral verification are mandatory for consensus/proof changes; at most two review/fix waves before escalating unresolved issues.

## Review focus

1. A hash-consistent host value is not authority: authenticate every origin, committee, anchor and total power; test substituted roots/generation/cutoff/ValState.
2. Never turn execution errors or missing witnesses into a verified FI skip: test omitted necessary trial reads and ordinary reverting first transactions; lightweight skip predicates require singleton-equivalence coverage.
3. Preserve exact native Comet/protobuf semantics, signature slots, full BlockID and strict power threshold: shared stock vectors plus mutations.
4. Host feature unification or unwinding must not hide guest failures: check isolated portable dependency closure and execute both aborting guest targets.
5. Mutable landing frontier and public metadata cannot rewrite immutable proof origin or body: test overlaps, internal cuts, wrong blobs, images and reference/generation changes.

## File ownership and dependency direction

- mono `packages/taiko-client-rs/crates/etna-core`: existing canonical consensus types, useful pure computations and native verifiers; add only narrowly justified shared checks.
- mono abci/lander/CLI/config/tests: unchanged from the pinned base in this milestone; use them as source and vector oracles, with client adoption separately scoped.
- raiko2 existing `crates/protocol*`, `crates/pipeline`, stateless/witness and guest modules: fork-specific proof inputs/context, known-block/range verification, witness collection and proof/backend composition. Preserve existing execution and DB ownership.
- alethia-reth: add only a demonstrated missing fork behavior or small reusable execution/admission primitive; retain strict execution and necessary transaction-rejection semantics.
- A portable upstream range/statement library may be introduced if concrete consumers justify it. It must not depend on host services, and neither Raiko2 nor the node is restructured merely to accommodate it.

## Task 1: MPT feasibility prototype (historical; not an integration prerequisite)

The prototype is preserved in taiko-mono #22390 at `8ed9c271`. Focused core
regressions, independent review and actual RISC0/SP1 diagnostic guest execution
passed; full repository merge gates remain pending. It is not complete Etna
proving support and will not replace Raiko2's CachedTrie backend.

The subsequent SP1 comparison used one ELF, 15 synthetic cases and 57 executions
including controls. Cold three-/four-slot total cycles increased 9.9%/18.8%;
shared-node and overlapping requests also regressed, while a retained
VerifiedStorage helped repeated single-slot reads. The user chose to preserve
the current low-level implementation. No further work on this prototype is
required by this plan. Resume planning at the shared consensus boundary below.

## Task 2: Shared committee, parameter, schedule and validator-state implementation

**Files:** etna-core modules `params`, `types`, `key`, `domain`, `schedule`, `committee`, `validator`, `valstate` and required standalone test/data copies. Existing abci/lander/CLI/config and original fixture paths stay unchanged; the former migration is historical and reverted.

**Interfaces:** The independent core exposes its portable `ChainParams`, `ConsensusKey`, `Schedule`, `CommitteeState`, `ValState`, `validator_updates` and committee/validator commitments. Its optional `RegistryProof` contract authenticates exact root/account/ordered slots; supplied proof count, returned-word count, snapshot/cutoff/entries/selection and record checks remain complete. Existing native types, caches, MPT errors and configuration methods are preserved as-is. Document temporary source coexistence and pin parity to original fixture/source identities; no native facade is introduced in this PR.

- [x] Check core outputs against unchanged native/Go committee, wire and ValState vectors, with pinned source provenance and standalone copies; do not claim node imports the crate.
- [x] Port the pure dependency closure without a host service dependency. Keep native config methods and all existing client APIs unchanged; no new native adapter, free-function migration or MPT engine is part of the final diff.
- [x] Cover cutoff versus proving-anchor root, S(t)/H+2 boundaries, power caps/ties, exclusions, recovery initial committee and old pending-update discard. Authenticate origin ValState and advance every height, including non-switches.
- [x] Run core and affected node/lander tests, portable target checks and independent review/behavior verification; commit the coherent extraction.

Historical extraction checkpoint: `4fb1de48c5870bf36c8bd6c7c756df0bf5435550` on the shared upstream
branch. Independent review and 80 focused native test executions passed. A
pointer-width count defect found during review was fixed and reproduced with
actual RISC0 RED/GREEN execution. The affected lander library passed 325 tests,
with one resource acceptance test intentionally ignored. Full `just verify`,
Docker, complete Raiko2 guest integration and generated proofs remain unrun.

## Task 3: Callable consensus checks and Raiko2 validity composition

Execution checkpoints

- [x] Task 3a: native Comet codecs, checked validators and QC/body/ancestry with
  standalone test/data copies; native compatibility migration was reverted.
- [x] Task 3b preflight: run the stock Etna vectors before merging #22398 at
  `e6e186fe6a2e3912739db9b9cae1740e31064824`. Locked diagnostic builds used the
  existing Raiko2 RISC0 3.0.5 and SP1 6.8.1 patch tables. Both actual CPU guests
  emitted the same 684-byte journal as the isolated native runner and the
  independent stock oracle. See the [validation results and entry points](../../../crates/etna-guest-compat/README.md).
- [ ] Task 3b: map the existing Raiko2 guest's missing Etna checks to callable
  primitives; keep envelope/FI/transition composition in existing Raiko2 modules.
  Extract a helper only when that caller needs it. For `S > 0` proving authenticate
  the switching origin and derive the correct initial committee/context. The
  current lander's nonzero-genesis guard is a separate client limitation, not a
  requirement to restructure or extend the lander before implementing the guest.

The early shared-API milestone was published as a draft in
[taiko-mono #22398](https://github.com/taikoxyz/taiko-mono/pull/22398), head
`ccc9221f7d5d3c33bb3d8a62387c8f2606698bfe`. Independent task and integrated
source reviews passed; core 59 and lander 326 tests passed, with one resource
acceptance case ignored. Seven independent native QC/body groups and identical
representative challenges executed successfully in actual RISC0 RV32 and SP1
RV64 guests. These diagnostic executions do not constitute complete range proofs.
Full `just verify`/Docker/resource gates remain outstanding. At draft publication,
Rust/Docker CI checks were skipped and link checking failed on unchanged vendored
MIT license links returning HTTP 403. Wait for external API review before substantial
Raiko2 integration.

**Files:** Existing Raiko2 fork-verification modules and etna-core exports needed by identified callers. Use current abci/lander rules as source evidence; change client files only for necessary compatibility wiring or an extracted-surface fix.

**Interfaces:** Preserve canonical envelope version6 and existing eligibility/header rules. Reuse `verify_commit`, body/BlockID checks and other demonstrated pure APIs; no mandatory common transition result or client adoption of guest orchestration. The Raiko2 caller authenticates and consistently binds context and accepts a transition only after all consensus, execution and FI checks complete.

- [ ] Migrate existing stock Comet vectors and negative tests for nil votes, absent slots, duplicate/substituted signers, altered sign bytes, exact2/3, wrong generation/domain/full BlockID and envelope/EL byte mismatch.
- [ ] Define the authenticated verification context separately from node-local build settings. Reuse existing validation predicates and cheap header/schedule math; keep payload_attributes/Engine orchestration outside the guest. Accept arbitrary protocol-permitted beneficiary and alternative valid transaction selections. Compose transition/update checks in the existing Raiko2 guest; introduce a shared checker only for a demonstrated missing call boundary, without changing client orchestration.
- [ ] Reuse existing FI `NeedsSkip` as a pending request, never a completed authorization. Preserve ineligible/fee/revert/OOG/zk-discard distinctions.
- [ ] Run node/lander differential fixtures and core target/crypto vectors; independent review and verification, then commit.

## Task 4: Etna execution and FI omission verification over existing paths

**Files:** existing Raiko2 stateless/witness interfaces and thin fork adapters; alethia-reth changes only for a verified missing Etna behavior, based on #248 in a dedicated isolated branch. Preserve CachedTrie, node pooling, the execution database and post-state-root support. Etna validity uses existing guest composition and shared primitives where useful.

**Interfaces:** Start with the existing strict witness-backed block validator and add narrow Etna fork/context support. If a shared caller needs an adapter, keep committed-block verification and `verify_fi_omission` as distinct contracts; a new executor framework is not mandatory. A verified FI omission establishes the same completed decision as the pinned singleton rules, without requiring a fully assembled empty block. Included revert/OOG cannot justify omission; missing witness/config/infrastructure remains an error.

- [ ] Identify the smallest existing API delta and add RED fixtures for strict `[valid,zk-overflow,valid]` rejection, included ordinary revert/OOG at index0 and empty Etna blocks. Proving a committed list must never filter a shorter prefix.
- [ ] Reuse existing witness-backed DB, signer recovery, ancestor checks and post-root calculation. Add only necessary Etna fork checks; preserve system calls and Unzen behavior. No generic backend replacement or relocation.
- [ ] For each FI omission path, establish equivalence with the completed singleton decision on the same authenticated parent, checked attributes and full budgets. A lightweight rejection predicate is allowed only with that evidence; zk/state-dependent cases retain necessary trial execution. Keep trial effects separate from actual execution and check required system-call context.
- [ ] Preserve all reads needed by the chosen verification path, including code/storage/BLOCKHASH/EIP7702 before a zk discard. Missing a necessary read must fail; never convert a prover-local resource failure into a skip. Test actual/trial ordering and rollback isolation.
- [ ] Compare reuse versus specialized verification using representative, identity-pinned guest measurements where a cost-based decision is needed. Reuse valid existing measurements; no duplicate target trees or unsolicited broad benchmark campaign.
- [ ] Align exposed dependency identities/crypto hooks, run focused native/guest/differential checks, independently review/verify and commit the narrow change.

## Task 5: Complete common range verifier and canonical output

**Files:** Existing Raiko2 fork protocol/pipeline/guest-common modules for input/context/range orchestration; shared canonical ABI/codec/statement checks and small upstream verifiers where justified. No mandatory client-owned proof framework or wholesale module migration.

**Interfaces:** A fork range verifier authenticates the supplied blocks/context and returns the canonical statement/preimage/hash plus boundaries/cuts. Compose existing execution with the required shared checks; use local proof-input types rather than forcing client request types into the guest. Inputs include raw consensus/EL data, authenticated origin/ValState/registry/L1 witnesses, execution and necessary FI evidence, DA and reference data. PreparedBatch::verify_restored remains host journal validation, not the verifier.

- [ ] Compare current protocol ABI/statement/layout/vectors with the client's generated pin; reuse existing generation entrypoint to refresh only when necessary. No independent L1 authority interface is added.
- [ ] Authenticate fixed origin, parent hash/state, registry/committee/ValState, full range, own/inherited anchors and every required L1 root; include switching-origin Anchor implementation witness. Implement the currently specified ancestry policy first; historical Merkle optimization is separate.
- [ ] Verify each supplied block and execute its complete committed body strictly. Resolve FI only with authenticated evidence for the completed singleton decision. Decode/bind the supplied blobs and ordered blocks, then derive required cuts/statement; do not rerun compression generation, batch selection or blob packing.
- [ ] Test wrong origin/cutoff/header/committee/ValState/blob/cut/reference and generation changes, handovers/recovery, complete overlap `(p,h]` with live frontierc and terminal values. Failed transitions leave replay state unchanged.
- [ ] Run Rust/Solidity/client vector comparisons and portable/native replay, independently review and verify, then commit. No native fixture is described as a real proof.

## Task 6: Raiko2 wrapper integration and both real guests

**Files:** raiko2 provider/pipeline fork wiring, shared-type re-exports, protocol request/backend mapping, guest entrypoints, prover aggregation, xtask/provenance/API docs and generated ELF artifacts.

**Interfaces:** Preserve existing Raiko2 execution/guest/prover architecture and pin reviewed dependencies exactly. Providers collect needed consensus/execution/FI evidence; existing guest entrypoints initialize current crypto, deserialize the fork input, compose verification with existing execution and commit the complete output. Share validity predicates without requiring identical mining algorithms or a new cross-repo request framework.

- [ ] Add versioned Etna range request and immutable task/cache/artifact identities without reinterpreting existing Unzen v4 proposal IDs. Test missing history/witness versus contradictory inputs, persisted retry/restart and stale generation/image/reference handling.
- [ ] Align the full upstream dependency graph, fix removed devnet construction APIs and update host/both guest locks together. Record actual resolved hashes and preserve existing crypto patches.
- [ ] Extend existing RISC0/SP1 guest and aggregation paths for Etna with exact child continuity, image/rule/generation and ordered content/cut commitments. Avoid new parallel execution/provider/queue architectures. Each final backend covers execution+consensus+DA+the complete statement.
- [ ] Build each backend once after source freeze using inspected xtask/cache configuration. Run a real valid proof and mutated invalid cases, verify each output/proof and their combined L1 verifier result in an isolated test environment. Preserve Unzen fixtures and API/queue regressions.
- [ ] Independently review and behavior-test both backend and failure-path results; commit source/locks separately from tool-generated artifact refreshes as appropriate.

## Task 7: Full validation, upstream review and interruption handoff

- [ ] Run the affected repositories' required final fmt/lint/unit/integration gates, plus actual two-backend proof/ABI conformance and Unzen regression. Attribute any unavailable resource/tool/integration check explicitly; never label it passed.
- [ ] Independent final whole-stack review of exact recorded bases/heads and cross-repo pins, including negative controls, discarded witness paths, source/image identities, and untested assumptions.
- [ ] Fix material findings and have their raiser recheck; no automatic merge/deploy/activation. Publish reviewable upstream/adapter PRs when ready, with exact validation and remaining limits. If a diagnostic draft is useful earlier, keep its incomplete status explicit.
- [ ] Record all commits, branches, source/target identities, tests, pending failures and next task in the local ledger and tracked handoff so interruptions resume without repeating completed work or compiling another target.
