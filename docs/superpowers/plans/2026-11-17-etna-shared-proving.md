# Raiko2 Etna Integration Plan and Progress

> **For agentic workers:** Use superpowers:subagent-driven-development or
> superpowers:executing-plans for implementation. Track work by the stable P00–P08
> IDs below. Check a task only after its acceptance evidence is recorded.

**Goal:** Prove supplied Etna ranges with shared consensus primitives and Raiko2's
existing execution, witness and prover infrastructure.

**Architecture:** Raiko2 composes narrow verification calls around the existing
EVM replay and proof backends. Producer policy and BFT networking stay outside the
guest. Preserve CachedTrie, witness pooling, execution DB and Unzen behavior.

**Tech stack:** Rust, etna-core, existing Alloy/prost/Comet codecs, alethia-reth,
RISC0 3.0.5 and SP1 6.8.1 at the recorded feasibility checkpoint.

**Spec:** [Shared proving design](../specs/2026-11-17-etna-shared-proving-design.md),
including its current planning-baseline amendment.

## Current state and source authority

Updated 2026-11-20. This plan is the authoritative progress tracker for
[Raiko2 #305](https://github.com/taikoxyz/raiko2/pull/305).
The user resumed implementation on 2026-11-19. Work now includes source
alignment, the bounded statement/cut codec and the proof-specific execution projection. Review requests for other
PRs are handled separately; only verified dependency or proof-rule changes
alter this plan. Keep shared build caches and resource limits in place.

| Source | Exact revision | Meaning |
| --- | --- | --- |
| Existing Raiko2 checkpoint | `845ecbcbe9c5cb81e1c09d0ca37464e58c2dbc18` | Implemented and tested consensus-input slice under the earlier design |
| Shared dependency | `f31e7ec55345ac7faf99756504535b0cc9929ec0` | Merged taiko-mono #22398; used by that slice |
| Patched SDK diagnostic | `a29b53c462b69035ef2acfb5785d78ba20067605` | Actual CPU guest feasibility with the then-current patch tables |
| Merged design target | `93b59fe629dc8987ecb03c7d59d1f66004de068f` | taiko-mono #22412; reviewed head merged as `2e6763e2`, implementation adoption still separate |
| Shared body API candidate | `d7c6c3e30599463177152346d21dee6610f9f7f0` | [taiko-mono #22415](https://github.com/taikoxyz/taiko-mono/pull/22415), optional full-original-body helper; ready for external review; not the current Raiko2 dependency |
| Inspected client / protocol | `af1b952adb2d14263da60c5090e15e65b5d8d48c` / `4c6424bc97b226d100cc9ce9dc828c9567acef4e` | Still the earlier 9-field input, 23-word statement and 4-word checkpoint implementation |
| Inspected execution candidate | `53f33b708d869cfb3f8cccd597381fa2e92a5be5` | Anchorless execution exists; adoption also requires the OP-reth 2.7 / traits 0.8 / Alloy consensus 2.5 interoperability update |

The recovery-audit issue was fixed and the exact revised head was
[approved](https://github.com/taikoxyz/taiko-mono/pull/22412#pullrequestreview-5968429349), then merged at that same head.
That delta changes no proving rule or ABI. The contract/client snapshots still
implement the earlier relation, so generated runtime ABI agreement remains open.
The new codec follows the normative proposed layout and independent ABI vectors;
it does not establish compatibility with those old implementations. No runtime,
SDK or execution dependency pin changes in this slice. Inspect later source deltas
before adoption rather than silently following development branches.

## Milestone progress

`DONE (earlier baseline)` records reusable evidence, not conformance to #22412.
`PARTIAL` means the listed code exists but its remaining acceptance gate is open.
`TODO` means no completion claim. Dependencies are deliverable gates, not a
requirement to run independent inspections serially. No percentage is inferred
from checkbox counts.

| ID | Deliverable | Status | Depends on | Completion gate / evidence |
| --- | --- | --- | --- | --- |
| P00 | Portable shared core and patched SDK feasibility | DONE (earlier baseline) | — | Merged #22398; diagnostic README and checkpoint `a29b53c4` |
| P01 | Freeze revised spec, implementation pins and ABI vectors | PARTIAL | Compatible upstream implementations | Design approval, source matrix and codec vectors recorded; native ABI/backend framing agreement pending |
| P02 | Canonical supplied-block input and EL/Comet binding | PARTIAL | P01 for revised behavior | Earlier gate plus independently checked F31 header/envelope/EL projection; strict nested witnesses and range composition remain open; #22415 is optional |
| P03 | Authenticated origin, L1 context and committee schedule | TODO: reuse and compose | P01, P02 | Reuse existing L1/MPT readers; Etna reference, origin, storage/committee binding and far-path specification alignment remain open |
| P04 | Strict Etna execution and FI omission verification | TODO | P01, P02 | Existing execution path passes full-body and singleton-equivalence cases |
| P05 | Range consensus coverage, DA, cuts and statement | PARTIAL: codec only | P02–P04 for full range | Native encoding and digest vectors pass; context, execution, DA, QC coverage and proof composition pending |
| P06 | Provider, pipeline and backend wiring | TODO | P02–P05 | Existing task lifecycle produces and consumes the complete Etna input |
| P07 | Both real guests, aggregation and proof acceptance | TODO | P05, P06 | Real proofs, child continuity, permitted L1 verifier combination and resource bounds |
| P08 | Regression, independent final review and ready handoff | TODO | P01–P07 | Required checks, Unzen regressions and exact-head review; no unresolved material findings |


## Reuse audit and scope correction (2026-11-20)

- Removed the unused independent Etna L1 verifier and its 159-vector scaffold; reuse existing L1 linkage at its actual caller and retain the raw/future-header adapter requirement as an open integration gap.
- Removed ordinary transaction-root recomputation from the Comet/EL byte projection. Existing `stateless::validate_block` consensus/pre-execution remains authoritative for the same matched block; its existing `rejects_mismatched_transaction_root` test is the regression check.
- Retained BFT/domain/committee duties, original envelope/EL byte equality, cross-layer Comet/EL height, generation and independently authenticated expected-parent binding. Generic EL validation against an arbitrary supplied parent is not a substitute for that last relation.
- No account/MPT implementation was added. Reuse existing `SparseState` / `CachedTrie`; future work is Etna-specific root/address/slot/context composition.
- Restored the earlier near/far L1-reference decision in the paired design; its historical SSZ bridge and current normative linear-chain rules still need explicit reconciliation.

Current acceptance is the existing 25-test fork crate plus the common EL rejection
regression, scoped Clippy/format/no_std checks and independent review/verification.
The common regression exercises the existing Shasta/Unzen validator; Etna fork
wiring is still open and is not established by that check.
Earlier standalone L1 evidence is withdrawn from the completion gates. No source
or SDK pin, EVM/trie/provider/pipeline or production guest/ELF change is included.

## What #22412 changes in the proving plan

| Previous assumption | Revised target | Work affected |
| --- | --- | --- |
| Guest replays ValState and proposer priorities | Priorities are node/Comet state; the guest does not check `app_hash` or its ValState preimage | P02, P03, P05; remove priority arithmetic from the planned guest, keep committee derivation |
| Comet `app_hash` equals the parent EL hash | Node `app_hash` commits EL hash plus ValState; guest binds EL through the canonical envelope and EL parent chain | P02; the existing core body helper's bare-EL equality cannot be reused unchanged |
| Every block and embedded LastCommit needs QC verification | Verify h and the last height of each signing-committee interval in `(p,h]`; bind all other headers by ancestry | P02, P05, P07; derive coverage, do not accept a host-provided skip flag |
| Grid-aligned range/cuts, 23-word statement, 9-field input | Any terminal height; every interior cut; 21 words / 672 bytes; 8-field LandInput | P01, P05–P07; 4-word cut leaf and 3-word checkpoint record |
| Minimum proof unit is K blocks | Every valid block must be provable alone, including any required FI trial | P04, P07; singleton and worst-case proof acceptance |
| Registry displacement and old evidence limits | Lowest empty slot; source-defined supply/minStake constraint; 900 blocks / 1 hour evidence limits | P01, P03; native/fixture conformance, not guest evidence-pool replay |

The existing strict gate is evidence about the previous relation. Its primitive
codecs, signatures, body commitments, bounds and fixture oracles are reusable;
its bare-EL app-hash check and mandatory predecessor QC are not the final new
range verifier. Retaining the old tests as historical controls does not make
them the acceptance definition for the new relation.

## Global constraints and review focus

- Keep Raiko2's EVM, CachedTrie, witness and execution DB ownership. No client
  service migration, new generic Protocol trait, trie replacement or mining replay.
- Pin reviewed revisions. Regenerate ABI/bindings through their existing tools;
  do not hand-edit generated contracts, ELFs, VKs or provenance.
- Committee provenance, keys/powers, strict weighted quorum, L1 roots, EL/FI/DA
  and complete range/output binding remain proven. Removing priorities does not
  authorize an arbitrary host committee or skipped execution.
- Do not put a ValState hash back into settlement to compensate for an old API.
  App-hash/priority replay and stock evidence-pool behavior are client concerns.
- Derive mandatory QC targets from the authenticated schedule and range. Extra
  child-proof boundaries must not silently require otherwise unnecessary QCs.
- Preserve `(p,h]` when settlement advances to c: for `p <= c < h`, retain the
  original proof origin, blobs, cuts and statement; do not rewrite into `(c,h]`.
- Never turn missing witness, timeout or resource failure into a valid FI skip.
  A committed list is executed completely, including ordinary revert/OOG.
- No Unzen/Etna or recovery-generation-spanning proof. Preserve stop/settle/restart
  at the special fork and generation-qualified identities thereafter.
- Reuse the configured shared target and existing crypto patches, serialize builds,
  bound jobs/disk use and freeze source before expensive guest builds. No new SDK,
  duplicate target, cleanup or benchmark campaign without task-specific need.
- For each implemented milestone: focused RED/GREEN controls, scoped checks,
  independent review, and independent behavior verification for proof/consensus.
  Record source/command/results; fix and recheck material findings before DONE.

## P00 — Shared-core and SDK feasibility: completed evidence

- [x] Deliver portable committee/key/power/schedule, native Comet codecs and QC/body primitives upstream (#22398).
- [x] Run the stock diagnostic in actual patched RISC0 3.0.5 and SP1 6.8.1 CPU guests; compare native and independent oracle journals.
- [x] Independently review the previous Raiko2 consensus gate: seven authored tests, six independent groups, focused strict Clippy and workspace formatting.

Evidence: [diagnostic results](../../../crates/etna-guest-compat/README.md),
[gate tests](../../../crates/protocol-etna/tests/certified_envelope.rs), and
[#305](https://github.com/taikoxyz/raiko2/pull/305). These are not real proofs of
#22412. The diagnostic lock changes leave production ELF provenance stale.

## P01 — Freeze the new relation and source interfaces

**Owns:** dependency/source matrix, protocol codecs and shared-vector provenance;
root/guest manifests and generated binding entrypoints only when adopting code.

- [x] Record reviewed design `93b59fe6` and its exact-head approval; its recovery-only delta leaves the proving/ABI target unchanged. Later deltas still require inspection.
- [x] Compare actual client `af1b952a`, protocol `4c6424bc` and execution candidate `53f33b70` with the design: old 9/23/4 settlement and bare-EL app hash remain; execution adoption changes the Reth/Alloy graph.
- [ ] Pin a compatible reviewed implementation set, including execution fork and rules/SDK identities. Do not treat the preceding source audit as completed integration.
- [x] Implement pure typed ABI encoders for the proposed 8-field input, 21-word / 672-byte statement preimage and 32-byte Keccak digest, four-word cut leaf and five-word cutHash. Cross-check representative and full-width codec-only vectors against independent manual packing and offline Cast.
- [ ] Compare generated native contract/client layouts, selectors and vectors, including the three-word record and semantic uint48/range boundaries. Record the exact public-input/journal framing required by each pinned backend verifier.
- [x] Specify and independently verify the structural-only shared body API candidate in [taiko-mono #22415](https://github.com/taikoxyz/taiko-mono/pull/22415), without AppHash or certificate policy and without native caller changes.
- [ ] Adopt additional shared APIs only when a concrete caller benefits. #22415 is optional, not a global gate; the direct proof-specific path uses existing F31 header/DataHash primitives. Keep the current pin while its review proceeds.

Evidence for the encoding subtask: [source](../../../crates/protocol-etna/src/statement.rs),
[golden tests](../../../crates/protocol-etna/tests/land_statement.rs) and
[fixture provenance](../../../crates/protocol-etna/tests/fixtures/README.md).
Thirteen crate tests (seven earlier-gate plus six codec) and two independent
oracle cases passed; scoped strict Clippy and workspace formatting passed.
The codec deliberately does not enforce authentication, range/count/budget
constraints or semantic uint48 bounds. Full-width vectors test encoding only.

**Accept:** cross-language preimage and digest golden vectors agree with the
matching native contract/client encoders and backend framing; old 23-word,
missing-field, reordered-cut and wrong-generation inputs reject. The matrix
identifies implemented versus specified behavior. Historical pins alone do not close P01.

Build consistency was repaired without changing runtime pins or guest locks. The test
now distinguishes independent packages in the same repository and follows the actual
`guest-common` dependency edges to the precise locked commits. The full xtask suite
with `digests` passed 42 tests; two independent controls, strict Clippy and formatting
passed. The existing CI lanes now include `protocol-etna`; the Python CI environment
also declares its missing Keccak backend. Production guest provenance remains an
open draft gate, rather than being rewritten to match stale artifacts. Unused
private signing fields were removed from the diagnostic generator inputs; all
public values and the pinned 684-byte journal are unchanged. Native re-execution
passed; historical guest ELF/cycle measurements were not repeated.

## P02 — Supplied blocks and canonical envelope binding

**Files:** `crates/protocol-etna/src/lib.rs`, narrow envelope/input modules in that
crate; `crates/protocol-etna/tests/certified_envelope.rs` and new envelope tests.

- [x] Build the earlier canonical-wire/current-QC/body/embedded-QC gate in `845ecbcb`; keep its evidence scoped to that relation.
- [x] Implement and independently verify the narrow upstream body API candidate in #22415: 67 core tests and three independent checks, including 26 Go vectors and 2,112 differential inputs. It recomputes full BlockID and returns exact transaction bytes; it authenticates no identity, domain, version, committee or execution by itself. External review and Raiko2 adoption remain open.
- [x] Bind the exact v6 envelope shell and supplied EL projection with existing canonical RLP and Ethereum codecs. Preserve byte identity and the normative/native wire schema; no ABCI/lander service import. Full strict nested-witness decoding/authentication remains required in P03/P04.
- [ ] Bind every supplied EL header/body to its committed envelope, height, parent and generation; treat Comet app-hash bytes as opaque to the proof, as the revised spec requires.
- [ ] Compose header-hash ancestry, DataHash binding and required endpoint certificates. Recompute full original part sets only for callers that require that stronger body relation; do not add it to every guest block by default. All header identities still require authentication.

### Immediate P02 projection slice

Use the current F31 `header_hash`, `data_hash`, domain and app-version primitives.
`projection.rs` binds a caller-authenticated expected Comet header hash and the
single exact envelope to the actual supplied Ethereum Header and typed
transactions. It checks domain/version/height, fixed-origin header ancestry,
EL parent and 13-byte generation/anchor extraData; no AppHash-content check or
per-height detached QC is introduced.

The v6 parser checks only the strict outer shell and optional slot arity, retaining
borrowed execution/witness RLP spans. Compare the Ethereum Header's canonical
encode/decode fixed point and ordered EIP-2718 byte strings;
limit the complete original execution item to 124,000 bytes. Native wire vectors
and real public signed transaction fixtures guard compatibility. The returned
projection is not complete envelope or proof validation: nested witness grammar,
authenticated provenance/schedules, all-height validator hashes, strict execution,
FI, DA and endpoint QCs stay mandatory in their composing milestones.

- [x] Add behavioral RED and implement the bounded projection in the existing fork crate.
- [x] Preserve all external dependency records; inspect normal no_std features.
- [x] Run focused/independent native wire and mutation tests, strict scoped Clippy and formatting; obtain independent adversarial review.

Historical projection evidence at `39f803b5`: [implementation](https://github.com/taikoxyz/raiko2/blob/39f803b532df88f688a350ef95bcf8adce0021f5/crates/protocol-etna/src/projection.rs)
and [12 regression tests](https://github.com/taikoxyz/raiko2/blob/39f803b532df88f688a350ef95bcf8adce0021f5/crates/protocol-etna/tests/execution_projection.rs).
The semantic RED failed at the unimplemented verifier, then the final native
`cargo test --locked --offline -p raiko2-protocol-etna --all-targets -- --test-threads=2`
run passed 33 tests: 12 projection, seven earlier-gate, six statement and eight
independent scratch-only native-oracle groups. The latter used the exact
`envelope.rs` and raw-L1-header codec from client `af1b952a`, compiled separately
against its locked Alloy consensus 2.5.0, then consumed those bytes with Raiko2's
unchanged 2.4.2. Public signed legacy/EIP-1559 inputs, optional witnesses,
124000/124001-byte execution items and a larger witness-only envelope were covered.
The native encoder source SHA256 was
`4171e5e1cd72fe46853047fcc21676277c73946318d37cee49c255a3da0cc26e`.
These are wire fixtures, not valid complete Etna blocks or state-transition proofs.

Strict scoped Clippy and workspace formatting passed. A separate locked
`#![no_std]` consumer compiled, and its normal dependency tree activated no `std`,
`use_std` or `use-std` feature. All 1207 other root lock package records remain
unchanged; only three direct existing-version Alloy edges were added to the fork
crate. Independent adversarial review and independent behavioral verification
passed. Validation used host Rust 1.95; no production guest build, ELF refresh,
real proof, cycle/performance measurement or end-to-end integration is claimed.

A scratch-only patch of #22415 into the current Raiko2 Rust 1.95 graph passed
14 native tests: the new API consumer plus all seven earlier gate and six
statement cases. The consumer matches the Go header/part hashes and current
Tendermint type, including opaque AppHash bytes. The other 1,207 lock records
are unchanged. This verifies native crate compatibility, not canonical envelope
binding, guest/proof acceptance or dependency adoption. The real checkout
keeps the merged `f31e7ec5` pin.

**Accept:** valid new app hashes and alternate permitted beneficiaries pass;
substituted envelope/EL bytes, raw/projection mismatch, height/domain/parent errors,
malformed framing and changed authenticated header commitments reject. Full-BlockID
callers separately reject changed original part sets. No full-transition claim
is attached to an unauthenticated input or a structural-only result.

## P03 — Authenticated context and committees

**Files:** narrow context modules in `crates/protocol-etna`, the existing
witness-backed read APIs in `crates/stateless`, and Etna guest composition.

- [ ] Authenticate proof origin, generation, parent EL/Comet identities, L1 reference/header ancestry, fixed parameters, applicable rules and planned stop.
- [ ] On the switching boundary, verify the L2 SignalService proxy account/EIP-1967 slot under S's settled state root against the implementation built into the guest (PROOF-6.2-46a). The originWriter timestamp precheck and node-local `beaconRootsFrom`/fork-time trust check remain originWriter/client responsibilities, not additional guest witnesses.
- [ ] Adapt existing authenticated storage reads to registry snapshots, complete entry enumeration, latest-at-cutoff selection, records and power derivation; derive active/pending committee timing including H+2 and recovery-origin floors.
- [ ] At every height, compare both `validators_hash` and `next_validators_hash` with the independently authenticated active/pending schedule (PROOF-6.2-18), including interior heights without a required QC. Keep proposer-priority arithmetic and ValState preimages outside this guest path.

**Accept:** wrong root/slot/count/cutoff/key/power/epoch/generation and substituted
origin reject; initial deployment, nonzero S, ordinary handover and recovery-origin
cases pass. Mutate current and next validator hashes at a non-QC endpoint while
resealing the authentic ancestry and endpoint QCs; both mutations must reject.
Reuse shared algorithms; do not accept host success flags.


### P03 reuse and near/far reference integration

The unused independent L1 module and its tests/vectors from `1bc5bea6` were
withdrawn after the reuse audit. Its earlier native checks remain historical
evidence in that commit, not completed integration. Generic hash linkage already
exists in `guest-common`; account/storage authentication already exists in
`stateless` over `SparseState` / `CachedTrie`. Do not add another generic verifier.
Existing public helpers are parent-state-oriented; add only a thin expected-L1-root
adapter when a real Etna call site needs it. Preserve exact witness address/slot
and format obligations. Typed L1 headers cannot preserve unknown future fields:
track the raw-header adapter gap when reusing linkage rather than treating it as
already solved or restoring a parallel library in advance of its consumer.

The [paired design's recovered near/far strategy](../specs/2026-11-17-etna-shared-proving-design.md#recovered-long-distance-anchor-decision)
restores the prior N..L / Q policy and links the original design and research.
Reuse generic linkage; keep Etna reference selection separate from Shasta origin
metadata. Normal Q=L needs no submission-delay tail when its expected landing
fits the BLOCKHASH/EIP-2935 query window. Otherwise choose newer Q and use a short
tail or beacon `block_roots` / `historical_summaries` bridge for L, retaining N..L.
All other required L1 roots must still be independently covered. Account for
proving/inclusion headroom, raw future fields, fork/slot/path bindings and archive
availability. Execution-block lookup age is distinct from beacon-slot windows.

- [ ] Reconcile the retained historical bridge policy with normative PROOF-6.2-33/34 and BATCH-6.3-5..7 before enabling it; current spec still describes a linear resource/recovery limit.
- [ ] Wire the existing common L1 verification at the actual Etna consumer, adding only the necessary raw-header representation adapter and byte/count bounds. Do not require a new generic L1 framework.
- [ ] Bind selected Q to the same statement/contract reference, cross-check requested L against actual block anchors, and authenticate all origin/committee/rules roots.
- [ ] Reuse existing trie reads with the correct authenticated root and enforce Etna-specific witness/address/slot semantics; do not reimplement MPT or the execution database.

No historical SSZ guest, archive provider, reference-selection host API or full
origin/committee proof is claimed implemented by this audit.

## P04 — Existing execution and FI verification

**Files:** `crates/stateless/src/validation.rs`, `crates/guest-common/src/etna.rs`
when introduced, and focused tests in their existing crates.

- [ ] Align the Etna EVM dependency and select fork-specific header/system-call behavior, including the source-defined extraData/generation/anchor fields, required fee-share value and nonzero anchor-root context. Remove legacy anchor/first-receipt-success assumptions only on the Etna path; preserve Unzen checks.
- [ ] Wire the entire exact committed Etna body through existing strict consensus/execution/witness/trie/post-root code; ordinary transaction-root, header/body and STF checks stay there. Reject `[valid, zk-overflow, valid]` rather than filtering it into a shorter block; accept valid empty bodies and ordinary first-tx revert/OOG.
- [ ] Authenticate the FI queue entry and bytes, due-time/eligibility rules and cursor progression. Complete any required singleton trial, or verify an equivalent rejection predicate against the same parent, attributes and full budgets.
- [ ] Retain reads required before a discard, including storage/code/BLOCKHASH/EIP-7702; isolate trial effects from actual execution. Missing data and infrastructure errors remain errors.

**Accept:** differential singleton include/skip/revert/OOG/zk-discard cases,
missing-read mutations and execution-root mutations have the pinned decisions.
A host Engine result alone never authorizes omission.

## P05 — Complete range relation and settlement output

**Files:** range/statement modules in `crates/protocol-etna`, existing guest
composition, and new `consensus_range.rs` / `land_statement.rs` integration tests.

- [ ] Derive the required QC heights: terminal h plus the final height of each signing-committee interval inside `(p,h]`, with the exact origin/recovery schedule. Check each required strict quorum under its derived keys/powers. Confirm the source/vector interpretation when distinct epoch records have identical signing keys and powers; do not add certificate requirements merely because metadata changed.
- [ ] Verify all intervening header/full-BlockID links, native body commitments and envelope/EL continuity, plus P03's current/next-validator hash checks at every height. A later committee cannot replace the outgoing committee's endpoint certificate; do not require a QC for authenticated origin p merely because it is an input boundary.
- [ ] Authenticate and decode ordered blobs and references against the complete supplied range; preserve all per-block anchor, ruleset, stop, FI and execution checks.
- [x] Provide the pure 21-word preimage/digest and ordered cut-hash encoders (ENC-A-29/30/35). The statement recomputes nested commitments from the same generation, p/h, blobs and supplied cut leaves.
- [ ] Derive and authenticate p through h cuts and terminal values from the actual verified range, with every interior height in order. Bind the computed bytes32 statement digest to the final proof using the pinned backend framing; add no ValState field.
- [ ] Preserve overlapping settlement and historical origins without reproving a suffix. Reject wrong leaves, blobs, references, generations and cross-fork boundaries.

**Accept:** singleton, no-handover and multi-handover ranges; range ends immediately
before/at/after a handover; missing outgoing/terminal QC; same-set recurrence;
interior heights without detached QCs; omitted/duplicate/swapped leaves; `p=c` and
`p<c<h`; `c>=h` cancellation; and reference expiry. Validity is established for
all `(p,h]`, not merely the newly settled suffix.

## P06 — Thin host and backend integration

**Files:** existing extension points in `crates/provider` / `crates/pipeline`,
adding Etna provider code and `crates/pipeline/src/forks/etna` when needed; new
`crates/guest-common/src/etna.rs` and existing `crates/prover` codecs/backend seams.
No Etna provider/pipeline fork module exists at checkpoint `845ecbcb`.

- [ ] Define and review the concrete complete guest-input/output types from P02–P05 before wiring collection. Use the existing Preflight/Validation/PipelineSpec and GuestInputCodec seams; do not invent a generic Protocol framework.
- [ ] Collect raw consensus/execution/L1/FI/DA material, distinguish missing history from authenticated contradictions and feed the same guest relation on both backends.
- [ ] Bind task/cache/artifact identities to range origin, generation, rules/spec ABI, image and ordered content/reference; preserve retry/restart and cancellation behavior without reinterpreting existing Unzen proposal IDs.
- [ ] Reuse the existing request/provider/pipeline/queue ownership and update API/config documentation only for actual surface changes.

**Accept:** collection-to-native-verifier replay, stale-generation/image/reference
and cached old-layout rejection, partial collection/retry, task restart and Unzen
request compatibility. Do not count collection as guest authentication.

## P07 — Guests, aggregation and actual proof acceptance

**Files:** existing RISC0/SP1 entrypoints, aggregation callbacks, prover backend,
xtask guest builder and tool-generated ELF/VK/provenance outputs.

- [ ] Freeze aligned host/guest locks and current crypto patches. Use normal build entrypoints once per backend after source review; record exact compiler/SDK/image identities.
- [ ] Wire deserialize → shared verification/execution → exact backend public-output encoding with the existing guest architecture. Authenticate the statement digest in that encoding, not the 672-byte preimage as a substitute. Validate fixed-range child proof identity and continuity; arbitrary child boundaries must not introduce extra mandatory QCs or a new landing grid.
- [ ] Execute representative valid and mutated invalid cases in both actual guests; generate and verify real proofs for both supported ZK backends and a permitted two-proof L1 verifier combination in an isolated test.
- [ ] Measure a worst-case valid single block, including maximum legal committee/registry/witness data and any full-budget FI trial, plus target multi-block/blob ranges. Record SDK/hardware/resource evidence and unavailable counterpart-prover requirements explicitly.
- [ ] Regenerate production ELF/VK/provenance through the documented tooling and verify source/artifact bindings. Native execution and mocks are not proof acceptance.

**Accept:** actual valid proofs and invalid-case rejection, matching statements,
child ordering/image/generation failures, single-block completeness and supported
batch limits. No IVC, new zkVM, new pricing or broad performance campaign is part of this milestone.

## P08 — Final review and ready gate

- [ ] Run required affected fmt/lint/unit/integration lanes and Unzen regressions, with current exact sources and proof/ABI conformance.
- [ ] Obtain independent whole-stack adversarial review and behavioral verification; fix and have the raiser recheck all material in-scope findings.
- [ ] Record remaining assumptions and external gates; only mark #305 ready after actual callers, production artifacts and proof acceptance satisfy P01–P07.
- [ ] Keep deployment, image publication/registration, governance actions and merges outside this implementation plan unless separately authorized.

## Updating progress and preserving history

For each completed item, update its checkbox and milestone row together, recording
commit/PR, exact source pins, executed checks and review evidence in this plan or
a linked tracked artifact. A changed source/ABI can reopen a milestone; keep its
old evidence labeled historical. Do not mark a whole milestone done because one
fixture or host-only test passed. Update #305's current-status link/summary when
these states change; this table remains the single source of progress status.

Legacy mapping: old Task 1 is the rejected MPT prototype; Task 2 / Task 3a and
patched-SDK preflight map to P00; the `845ecbcb` slice maps to partial P02; old
Task 3b context maps to P03, Task 4 to P04, Task 5 to P05, Task 6 to P06/P07 and
Task 7 to P08. The previous plan remains available in Git at `845ecbcb`; the
design document and diagnostic README retain its source/checkpoint provenance.
