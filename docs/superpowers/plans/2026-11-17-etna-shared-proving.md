# Upstream shared Etna proving implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement task-by-task. Steps use checkbox syntax. Root coordinates the repositories and shared build target; one implementer owns a coherent task at a time.

**Goal:** Support Etna PoS proving through common upstream rule/verification crates, with Raiko2 restricted to collection, witness and backend adapters.

**Architecture:** Extract `etna-core` from the actual Etna client and make the node consume it immediately. Add `etna-proof-core` for authenticated range replay and the canonical landing statement; it calls narrow strict-execution/singleton adapters over the existing stateless execution and witness path. Raiko2 and RISC0/SP1 guests call those libraries without reimplementing Etna rules.

**Tech Stack:** Rust, alloc-capable protocol core, existing alloy/prost/Comet codecs, alethia-reth stateless execution, RISC0 3.0.5, SP1 6.8.1, Solidity ABI/shared vectors.

**Spec:** `docs/superpowers/specs/2026-11-17-etna-shared-proving-design.md`. The user selected the shared-library/guest-committee direction on 2026-11-17; builder check found no remaining user decisions. This replaces #295's L1 committee-authority proposal for this implementation.

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

The next upstream review concerns the shared consensus API. Wait for that review
to converge before substantial Raiko2 integration. Prefer an exact commit from
the merged Etna development mainline; a reviewed branch commit is a temporary
option. Keep Raiko2 in its own branch and record code revisions for later
implementation-conformance work. The separate TLA+/Lean work is untouched.

## Global constraints

- Keep the existing Raiko2 trie, witness and execution database implementation. Only fork-specific behavior required by Etna may change through narrow adapters; no generic backend replacement or broad extraction is part of this plan.
- Common deterministic Etna logic belongs upstream and has one production implementation, consumed by the node and guest. Raiko2 owns no alternative committee, QC, FI, range or statement rules.
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
2. Never turn execution errors or missing witnesses into an empty singleton: test omitted discarded-path code/storage/BLOCKHASH and ordinary reverting first transactions.
3. Preserve exact native Comet/protobuf semantics, signature slots, full BlockID and strict power threshold: shared stock vectors plus mutations.
4. Host feature unification or unwinding must not hide guest failures: check isolated portable dependency closure and execute both aborting guest targets.
5. Mutable landing frontier and public metadata cannot rewrite immutable proof origin or body: test overlaps, internal cuts, wrong blobs, images and reference/generation changes.

## File ownership and dependency direction

- mono `packages/taiko-client-rs/crates/etna-core`: canonical witnesses/types, parameters, key/domain, schedule, committee and authenticated-state interfaces, validator hashes/state, pure block/FI rules and canonical Comet validation.
- mono `packages/taiko-client-rs/crates/etna-proof-core`: proof input/context, checked replay/transition, origin/ancestry/DA, exact statement/cuts and composition. Depends on etna-core, never abci/lander/raiko2.
- mono abci/lander: adapters and compatibility exports that consume the core; transport, store, local trust and runtime policy stay here.
- existing stateless execution/witness backend plus narrow adapters: strict block execution, deterministic singleton completion and authenticated state reads. Preserve generic trie/DB implementations; only demonstrated missing Etna execution behavior belongs upstream. No FI queue/committee policy.
- raiko2 provider/pipeline/guest IO/prover/xtask: mechanical integration, fetching/witness construction, versioned request identity, backend composition and generated artifacts.

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

**Files:** etna-core modules `params`, `types`, `key`, `domain`, `schedule`, `committee`, `validator`, `valstate`; move corresponding abci tests/stock vectors and modify abci imports/re-exports, `config`, `store`, `wire`, `envelope` and callers.

**Interfaces:** Retain existing names `ChainParams`, `ConsensusKey`, `Schedule`, `CommitteeState`, `CommitteeWitness`, `VerifiedWitness`, `ValState`, `validator_updates`, registry root/record/config hash functions and native validator commitment helpers under etna-core. Pure data/config validation is canonical; builtin selection/TOML and state-store adapters remain outside or explicitly feature-gated. No mirrored parameter/state structs.

- [ ] Add cross-consumer vector tests proving node imports and core return identical canonical committee records/roots/config hashes and Go-derived ValState transitions/preimages.
- [ ] Move the consensus dependency closure, including validator wire/hash helpers; access authenticated L1 state through an existing-backend adapter instead of importing a replacement MPT engine; do not create core→abci→core cycles. Resolve foreign-type inherent configuration methods using host free functions or explicit optional configuration API.
- [ ] Cover cutoff versus proving-anchor root, S(t)/H+2 boundaries, power caps/ties, exclusions, recovery initial committee and old pending-update discard. Authenticate origin ValState and advance every height, including non-switches.
- [ ] Run core and affected node/lander tests, portable target checks and independent review/behavior verification; commit the coherent extraction.

## Task 3: Shared block rules, canonical Comet verification and transition replay

**Files:** etna-core envelope/rules/forced/validation/native-consensus modules; existing abci app validation/finalize and lander validate/native/history adapters.

**Interfaces:** Preserve canonical envelope version6 and existing eligibility/header rules. Expose pure context/check/transition results, `verify_commit` and body/BlockID validation, with a deterministic crypto interface. Shared low-s/key/order/domain checks precede backend crypto recovery. A pending transition becomes committed only after complete execution/FI verification.

- [ ] Migrate existing stock Comet vectors and negative tests for nil votes, absent slots, duplicate/substituted signers, altered sign bytes, exact2/3, wrong generation/domain/full BlockID and envelope/EL byte mismatch.
- [ ] Extract pure `ChainContext`/committee lookup and rejection errors from Engine/L1/runtime errors. Centralize update-schedule selection currently repeated in FinalizeBlock and lander replay.
- [ ] Reuse existing FI `NeedsSkip` as a pending request, never a completed authorization. Preserve ineligible/fee/revert/OOG/zk-discard distinctions.
- [ ] Run node/lander differential fixtures and core target/crypto vectors; independent review and verification, then commit.

## Task 4: Narrow Etna execution and FI singleton adapters

**Files:** existing Raiko2 stateless/witness interfaces and thin fork adapters; alethia-reth changes only for a verified missing Etna behavior, based on #248 in a dedicated isolated branch. Preserve CachedTrie, node pooling, the execution database and post-state-root support. Etna policy remains in the shared protocol library.

**Interfaces:** `ExecutionBackend` has distinct `verify_committed_block` and `build_singleton_on_parent` requests/results, plus authenticated state read capability. Strict execution checks the entire committed list. Singleton returns Included or a completed deterministic Empty; missing witness/config/infrastructure remains Err. Neither operation decides FI queue policy.

- [ ] Add RED fixtures for strict `[valid,zk-overflow,valid]` rejection, included ordinary revert/OOG at index0, empty Etna blocks and singleton rollback without trial effects leaking into actual execution.
- [ ] Reuse the existing witness-backed database/state-root support without replacing or broadly moving it. Add only the necessary Etna execution adapter and authenticate parent/ancestors/grandparent and signer recovery. Execute system calls on both actual and separate trial branches; an empty trial's state root need not equal its parent.
- [ ] Capture/read witness footprint from the complete aborted trial, including EIP7702/code/storage/BLOCKHASH; removing a required item must fail, not yield Empty. Test actual/trial execution ordering gives identical outputs.
- [ ] Align all exposed reth Git identities and preserve backend crypto package instances. Run native/differential and guest checks, independently review/verify, commit and record the exact development pin.

## Task 5: Complete common range verifier and canonical output

**Files:** mono etna-proof-core input/context/replay/ancestry/DA/statement/cuts/composition modules, generated portable ABI definitions, node/lander provider input adapter and shared fixtures.

**Interfaces:** `verify_range(input, execution_backend, consensus_crypto) -> VerifiedRange` authenticates its complete context and returns the canonical statement/preimage/hash plus typed boundaries/cuts. `RangeInput<W>` carries raw Comet/envelopes, origin/ValState/registry/L1 witnesses, exact block and FI trial witnesses, DA and reference data. `PreparedBatch::verify_restored` remains host journal validation, not this verifier.

- [ ] Compare current protocol ABI/statement/layout/vectors with the client's generated pin; reuse existing generation entrypoint to refresh only when necessary. No independent L1 authority interface is added.
- [ ] Authenticate fixed origin, parent hash/state, registry/committee/ValState, full range, own/inherited anchors and every required L1 root; include switching-origin Anchor implementation witness. Implement the currently specified ancestry policy first; historical Merkle optimization is separate.
- [ ] Enforce all per-block common checks and strict execution; resolve FI pending decisions only by authenticated singleton replay. Compute every required grid cut and the complete flat blob/KZG/statement binding.
- [ ] Test wrong origin/cutoff/header/committee/ValState/blob/cut/reference and generation changes, handovers/recovery, complete overlap `(p,h]` with live frontierc and terminal values. Failed transitions leave replay state unchanged.
- [ ] Run Rust/Solidity/client vector comparisons and portable/native replay, independently review and verify, then commit. No native fixture is described as a real proof.

## Task 6: Raiko2 wrapper integration and both real guests

**Files:** raiko2 provider/pipeline fork wiring, shared-type re-exports, protocol request/backend mapping, guest entrypoints, prover aggregation, xtask/provenance/API docs and generated ELF artifacts.

**Interfaces:** Raiko2 uses immutable upstream revs and the shared RangeInput/output. Provider fetches source consensus history and execution/trial witnesses. Guest entrypoints initialize existing crypto, deserialize, invoke common verify_range, and commit its exact output. No locally duplicated Etna rule engine.

- [ ] Add versioned Etna range request and immutable task/cache/artifact identities without reinterpreting existing Unzen v4 proposal IDs. Test missing history/witness versus contradictory inputs, persisted retry/restart and stale generation/image/reference handling.
- [ ] Align the full upstream dependency graph, fix removed devnet construction APIs and update host/both guest locks together. Record actual resolved hashes and preserve existing crypto patches.
- [ ] Implement the two thin range guests and fixed-range aggregation/wrapper with exact child continuity, image/rule/generation and ordered content/cut commitments. Each final backend covers execution+consensus+DA+complete statement.
- [ ] Build each backend once after source freeze using inspected xtask/cache configuration. Run a real valid proof and mutated invalid cases, verify each output/proof and their combined L1 verifier result in an isolated test environment. Preserve Unzen fixtures and API/queue regressions.
- [ ] Independently review and behavior-test both backend and failure-path results; commit source/locks separately from tool-generated artifact refreshes as appropriate.

## Task 7: Full validation, upstream review and interruption handoff

- [ ] Run the affected repositories' required final fmt/lint/unit/integration gates, plus actual two-backend proof/ABI conformance and Unzen regression. Attribute any unavailable resource/tool/integration check explicitly; never label it passed.
- [ ] Independent final whole-stack review of exact recorded bases/heads and cross-repo pins, including negative controls, discarded witness paths, source/image identities, and untested assumptions.
- [ ] Fix material findings and have their raiser recheck; no automatic merge/deploy/activation. Publish reviewable upstream/adapter PRs when ready, with exact validation and remaining limits. If a diagnostic draft is useful earlier, keep its incomplete status explicit.
- [ ] Record all commits, branches, source/target identities, tests, pending failures and next task in the local ledger and tracked handoff so interruptions resume without repeating completed work or compiling another target.
