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

## QC design hold and non-QC continuation (2026-11-22)

The user directed that QC/committee work pause pending the revised design,
which is expected to manage membership changes, penalties and queries on L2.
Reusing the node implementation is a candidate after that design settles, not
an adopted interface. Do not extend or freeze the old L1 committee-authority
proposal, registry proof adapters, committee derivation or QC policy. Keep the
existing tested helpers as partial components without wiring them into a final
proof. This amendment supersedes earlier instructions to complete that path.

- [ ] **QC-HOLD:** once the revised design is stable, inspect the authoritative
  node/L2 contract APIs and decide what to reuse or verify. P03 committee and
  P03-L01..L04 authority work, the fewer-QC discussion and final committee-related
  statement wiring wait for that review. #22423 is a historical provisional
  discussion and is not a blocker for independent execution/DA work.
- [x] **P05-D01a:** bind the complete decompressed execution-list bytes to the
  original matched item spans, with exact canonical outer RLP framing and the
  inclusive 64 MiB bound. Retain borrowed spans and existing codecs; no second
  expected payload, dependency or transaction/header decoder.
- [ ] **P05-D01b:** qualify a strict bounded inflater and compose version-6
  framing, compressed-size/dictionary/checksum/exhaustion checks with D01a and
  the existing ordered blob/KZG path. D01a alone does not establish DA.

The initially considered miniz_oxide 0.8.9 streaming backend is not adopted.
Independent source review found missing-history and incomplete-Huffman
admission differences from reference decoders, plus an advertised-window
constraint that the streaming wrapper does not enforce. Byte equality and
checksum cannot establish decoder conformance. Resolve that profile with differential malformed
stream tests before integration; do not add a custom DEFLATE parser or present
existing permissive Unzen decoding as strict Etna validation. A decoded-slice
size check does not bound allocation during inflation: D01b must enforce that
limit as output is produced.

The current non-QC payload source was inspected at taiko-mono main
`3602629758beb653423a23608fd1960810efcfac`: one version byte, one dictionary-free
complete zlib stream, no trailing bytes, canonical RLP list items equal to the
projected range, 64 MiB output and 124000 bytes per item. This remains a pinned
component baseline, not a claim the evolving overall design is frozen.

## Pending decisions and independent work (2026-11-22)

Keep design/upstream dependencies separate from engineering tasks not yet done:

| Item | Disposition | Independent work that can proceed |
| --- | --- | --- |
| FI consumption and skip trials | Phase two deferred; authenticated FI-off profile remains required for phase one | Preserve cursor/witness slots and existing execution architecture |
| QC management and committee authority | User hold pending L2-managed design | Retain tested helpers; do not extend the old L1 proposal or freeze replacement interfaces |
| Final ABI, public inputs and verifier framing | Committee-dependent fields wait for the new design; the present codec is provisional | Preserve existing byte vectors; complete interface-independent body checks |
| Etna execution source/profile | Alethia #248 remains open at `ad584325`; Raiko2's current `59e06d0` pin has no Etna fork | Audit exact strict-execution, system-call and legacy-anchor adapter boundaries; adopt a reviewed compatible revision before final execution integration |
| Distant-anchor SSZ bridge | Historical strategy still needs normative reconciliation with the linear header-chain rule | Ordinary per-range anchor structure and the near-reference path do not require enabling the bridge |
| Strict inflater and complete DA | Engineering/conformance gate, not a requested design hold | Keep D01a exact span binding; qualify decoder behavior and reuse existing blob/KZG verification |
| R06/R07 and production proof acceptance | Final delivery gates, not additional protocol design decisions | Reuse shared target; rebuild final ELF/VK/provenance and run real proofs after the complete source/interface is settled |

Completed bounded component **P03-A05a** in the existing range verifier: generation-first
anchor witness presence; interior anchor-number monotonicity, witness presence
iff advancing, and present/nonzero identical root on inheritance. This adds no
new context schema, storage reader, EVM or detached verifier. The incoming edge
of a mid-generation range is now checked by P03-A05b using the required,
hash-bound excluded-parent header. Root/time authority, witness contents, age
and full anchor verification remain separate acceptance requirements.

The latest [Alethia #248 candidate](https://github.com/taikoxyz/alethia-reth/tree/ad584325c6b1e15f3c163fb25e71c04a78b194e0)
now selects the 4-second Etna fee target, including its first block. This removes
the old candidate's observed 2-second discrepancy from the current worklist;
it is source evidence, not a successful Raiko2 adoption or guest execution.
The candidate also uses OP-reth `op-v2.7.0-1` and Alloy consensus `2.5.0`, whereas
this Raiko2 baseline uses its earlier OP-reth v2.6-aligned revision. Dependency
and API/SDK compatibility still need a bounded adoption check; no pin was changed.
Raiko2 #305 still has no newer human review than `5333d6d6` at this refresh.

## L1 authority boundary amendment (2026-11-20)

Protocol discussion: [taiko-mono #22423](https://github.com/taikoxyz/taiko-mono/pull/22423)
(draft; provisional representation, protocol-owner review pending).

The user directed that facts L1 can establish at proof verification belong in
L1-constructed public inputs. The host/provider reads them; the guest binds
the values it actually uses to the same statement L1 reconstructs. Host reads
alone are not authority. This amendment supersedes blanket requirements below
to re-prove L1-readable facts inside the guest, only once the equivalent L1
predicate and statement binding are established. It changes no QC policy.

At inspected protocol head `7cd9606af6a8c9290874bd1525d58008e24ebadc`, existing
statement fields already bind L1's domain, generation, retained origin,
configuration digest, reference hash and blob commitment. The contract does
not independently authenticate historical signing sets: its terminal snapshot
comes from calldata. The proposed signing-set/schedule hash, or smaller
checkpoint-authority variant, needs coordinated protocol review before guest
provenance checks can be removed. Checkpoint identity and activation timing
must cover every current/next set used by the full range, including recovery
and overlap; configuration hash is not a signing-set hash.

- [ ] **P03-L01 — Authority inventory:** map each context value to its L1
  source, guest use, public field and historical selection predicate. Do not
  add guest storage proofs for fields already constructed authoritatively by
  L1. Retain reference-header/hash and execution/data binding obligations.
- [ ] **P03-L02 — Protocol review:** propose the smallest authority commitment,
  its L1 derivation/history and exact statement schema. Review selected-set
  authority versus registry-checkpoint authority, costs, H+2, A-to-B-to-A,
  recovery, full-width domain and `p < c`. Representation is not frozen.
- [ ] **P03-L03 — Coordinated acceptance:** after owner review, update the
  normative relation, native client, L1 statement/verifier and both guest
  journals/aggregation consistently; settle redundant embedded witness rules
  before omitting their validation. Include mismatch/replay/cutoff vectors.
- [ ] **P03-L04 — Narrow Raiko2 integration:** fetch in provider/host, bind in
  guest, reuse native signature/quorum and shared selection where still
  required. Remove the unneeded guest registry proof adapter only after its
  authority replacement is enforced, without changing existing EL/trie code.

Local registry witness work at `d535534f` and its later canonical-path
refinement are component experiments, not an adopted production path. They
remain unpushed and unwired. Pause further guest registry-proof integration
while P03-L01/L02 define its replacement. Do not present their native tests
as proof of an L1-authorized committee or complete Etna proving.

## Current state and source authority

Updated 2026-11-20. This plan is the authoritative progress tracker for
[Raiko2 #305](https://github.com/taikoxyz/raiko2/pull/305).
The user resumed implementation on 2026-11-19. Work now includes source
alignment, the bounded statement/cut codec, proof-specific execution projection
and per-height validator binding / required-QC verification through the shared
native checker. Review requests for other PRs are handled separately; only verified dependency or proof-rule changes
alter this plan. Keep shared build caches and resource limits in place.

| Source | Exact revision | Meaning |
| --- | --- | --- |
| Existing Raiko2 checkpoint | `845ecbcbe9c5cb81e1c09d0ca37464e58c2dbc18` | Implemented and tested consensus-input slice under the earlier design |
| Shared dependency | `f31e7ec55345ac7faf99756504535b0cc9929ec0` | Squash-merged #22398 into feat/taiko-client-rs-abci; reviewed development pin, not main |
| Patched SDK diagnostic | `a29b53c462b69035ef2acfb5785d78ba20067605` | Historical CPU guest feasibility with the then-current patch tables; diagnostics are no longer active workspace members |
| Merged design target | `93b59fe629dc8987ecb03c7d59d1f66004de068f` | taiko-mono #22412; reviewed head merged as `2e6763e2`, implementation adoption still separate |
| Shared body API candidate | `d7c6c3e30599463177152346d21dee6610f9f7f0` | [taiko-mono #22415](https://github.com/taikoxyz/taiko-mono/pull/22415), optional full-original-body helper; ready for external review; not the current Raiko2 dependency |
| Reviewed design candidate | `3fe7350119f2efe38cf588c145a9913bf9b3b398` | [taiko-mono #22419](https://github.com/taikoxyz/taiko-mono/pull/22419); approved after the import/gossip specification fix; proving clarifications recorded separately, no blanket source/layout adoption |
| Earlier client / protocol audit | `af1b952adb2d14263da60c5090e15e65b5d8d48c` / `4c6424bc97b226d100cc9ce9dc828c9567acef4e` | Historical 9-field input, 23-word statement and 4-word checkpoint snapshots |
| Later protocol / client witness audit | `7cd9606af6a8c9290874bd1525d58008e24ebadc` / `69090335f64df4ac509859a465f55f93d2f4c5d9` | Protocol source has 8-field input, 21-word statement and 3-word checkpoint; client account-witness schema inspected, generated settlement/backend agreement still pending |
| Inspected execution candidate | `53f33b708d869cfb3f8cccd597381fa2e92a5be5` | Anchorless execution exists; adoption also requires the OP-reth 2.7 / traits 0.8 / Alloy consensus 2.5 interoperability update |

The recovery-audit issue was fixed and the exact revised head was
[approved](https://github.com/taikoxyz/taiko-mono/pull/22412#pullrequestreview-5968429349), then merged at that same head.
That delta changes no proving rule or ABI. The earlier contract/client audit
used the superseded relation; the later protocol source above now has the
8/21/3 layout. Generated native-client and backend-journal agreement remains
open. The codec follows the normative proposed layout and independent ABI
vectors; this alone does not establish end-to-end implementation compatibility. No runtime,
SDK or execution dependency pin changes in this slice. Inspect later source deltas
before adoption rather than silently following development branches.

The #22419 review initially found an import/evidence-gossip dependency at
`d32f4568`: an empty block evidence budget does not disable historical lookups.
The revised `3fe73501` retains history while either stock age limit is met,
authenticates the excluded boundary and requires stock header/commit/validator
lookups, including the terminal block-commit key.
It was [re-reviewed and approved](https://github.com/taikoxyz/taiko-mono/pull/22419#pullrequestreview-5986783866).
Actual importer-to-peer acceptance remains an implementation gate. This is
node/import work, not a reason to duplicate EL/MPT logic in Raiko2. Its proof
changes clarify signing-set identity (keys and assigned powers) and permit one
narrowly scoped settled-switch root reuse; the latest fix changes no proving
rule. Candidate interface/storage changes and all anchor tasks remain gated.

## Devnet delivery phases (2026-11-20)

The user explicitly authorized FI to be disabled for the first devnet phase.
Phase one targets the complete Etna execution/Comet/committee/DA/statement path
in that configured environment. Phase two adds FI consumption, eligibility and
singleton-trial verification. This is a delivery split, not a claim that the
unrestricted normative proof relation is already implemented.

- Keep the same envelope witness positions, origin/cut/statement FI cursor
  fields and existing execution/prover ownership in both phases. Carry the
  authenticated cursor unchanged in phase one; do not remove it from the ABI.
- The phase-one deployment must actually disable FI with no unprocessed entries;
  the eventual guest composition must be restricted to that deployment/profile
  and reject FI witnesses or consumption. A host-controlled skip flag is not
  part of the proof interface. The exact profile binding is still P01/P06 work,
  not an implemented flag or a claim about existing devnet configuration.
- Phase two fills the existing FI verification seam with authenticated queue
  entries, due checks, consumed-cursor updates and the necessary trial. It must
  retain the phase-one range/statement/input architecture and existing EVM/MPT.
- P04's FI items and FI-specific P07 worst-case trials move to phase two. P03,
  non-FI execution, DA/range/output binding, both real guests and the applicable
  proof/ABI/provenance gates remain mandatory for phase one.
- The fewer-QC design challenge is open in [#305](https://github.com/taikoxyz/raiko2/pull/305#issuecomment-7030686273).
  Do not silently alter QC policy or claim protocol consensus while proceeding
  with independent byte/identity composition.

### Immediate range projection slice

Reuse the existing per-block projection to derive every intermediate Comet/EL
parent from the preceding matched input over a complete bounded `(p,h]`, then
bind both terminal hashes to the expected statement endpoints. This supplies
the missing Etna Comet/envelope binding around existing EL range replay; it does
not reimplement the EL validator, aggregation framework or BFT signing process.
Comet links compare the predecessor header hash under the current target; no
stock full-BlockID or embedded-LastCommit rule is added. Origin/schedule
authentication, nested witnesses, execution, anchor/time rules, DA and final
statement composition remain separate gates.

## Parallel review findings and merge gates

The first external review at 0c218cbd/aeda3262 is tracked alongside implementation.
The source inspection confirmed #22398 merged into feat/taiko-client-rs-abci,
not taiko-mono main. The immutable F31 development pin is intentional for this
draft. Before merging into Raiko2 main, use an appropriate reviewed upstream
main/tag revision or explicitly resolve the source-lifetime/build-reproducibility
gate with reviewers; do not call feature-branch integration mainline adoption.
The obsolete e6 diagnostic pin is removed with its executable diagnostic targets.

- [x] R01: deprecate the superseded public certified-envelope gate; retain seven clearly historical controls.
- [x] R02: expose original committed transaction payload spans; cover both generation-parent guards and uint48 overflow; do not trust cached hashes or recovered signers.
- [x] R03: add checked statement range/interior-cut count validation before hashing while preserving raw ABI vectors.
- [x] R04: cover all manifest dependency groups and every taiko-mono package in pin guards.
- [x] R05: remove diagnostic production-workspace coupling and preserve commit-pinned historical evidence.
- [ ] R06: settle upstream dependency provenance before main merge.
- [ ] R07: run `just build-guest all --force` at the final source, commit generated provenance and pass the unskipped artifact/schedule-export CI gates before main merge.
- [x] R08: lint Etna integration tests explicitly in CI with all targets.

The scoped review fixes passed 68 protocol tests (7 historical controls), 44
xtask library tests, strict all-target Clippy for both touched crates, workspace
formatting and an isolated no_std consumer/normal-feature audit. Three projection
guard mutations each compiled and failed its exact new regression; the unmutated
source was compiled again and all 68 tests passed. One initial mutation run reused
a stale artifact at the shared virtual path; that result was discarded. Subsequent
variants forced source freshness without another target directory. Independent
source review and independent behavioral verification passed. Workflow YAML was
parsed with the Ruby standard library and its scoped Etna all-target lane checked.
These checks do not replace final-head production rebuilds or upstream mainline
source availability. R06/R07 remain unchecked and #305 remains draft.

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
| P02 | Canonical supplied-block input and EL/Comet binding | PARTIAL: complete range/parent projection | P01 for revised behavior | Bounded original-byte projection and Comet/EL chains bind the expected endpoints; strict nested witnesses and authenticated execution composition remain open; #22415 is optional |
| P03 | Authenticated origin, L1 context and committee schedule | PARTIAL: field binding | P01, P02 | Every current/next validator hash and neighboring schedule edge checked; authenticated origin/L1/storage/committee derivation and far-path alignment remain open |
| P04 | Strict Etna execution; FI in phase two | TODO | P01, P02 | Phase one: existing strict execution on the matched full body; phase two: authenticated FI include/skip and singleton-equivalence cases |
| P05 | Range consensus coverage, DA, cuts and statement | PARTIAL: codec, QCs, range and cut/statement projection | P02–P04 for full range | Matched Comet/EL identities and supplied cursor shape bind every interior cut and statement endpoint; actual FI/context authority, execution, DA and final composition remain pending |
| P06 | Provider, pipeline and backend wiring | TODO | P02–P05 | Existing task lifecycle produces and consumes the complete Etna input |
| P07 | Both real guests, aggregation and proof acceptance | TODO | P05, P06 | Real proofs, child continuity, permitted L1 verifier combination and resource bounds |
| P08 | Regression, independent final review and ready handoff | TODO | P01–P07 | Required checks, Unzen regressions and exact-head review; no unresolved material findings |


## Reuse audit and scope correction (2026-11-20)

- Removed the unused independent Etna L1 verifier and its 159-vector scaffold; reuse existing L1 linkage at its actual caller and retain the raw/future-header adapter requirement as an open integration gap.
- Removed ordinary transaction-root recomputation from the Comet/EL byte projection. Existing `stateless::validate_block` consensus/pre-execution remains authoritative for the same matched block; its existing `rejects_mismatched_transaction_root` test is the regression check.
- Retained BFT/domain/committee duties, original envelope/EL byte equality, cross-layer Comet/EL height, generation and independently authenticated expected-parent binding. Generic EL validation against an arbitrary supplied parent is not a substitute for that last relation.
- No account/MPT implementation was added. Reuse existing `SparseState` / `CachedTrie`; future work is Etna-specific root/address/slot/context composition.
- Restored the earlier near/far L1-reference decision in the paired design; its historical SSZ bridge and current normative linear-chain rules still need explicit reconciliation.

The reuse correction passed the then-current 25-test fork crate plus the common
EL rejection regression, scoped Clippy/format/no_std checks and independent
review/verification. The BFT and review-fix slices below expand the fork crate to 68 tests.
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

- Review #305 in parallel with implementation. Before each bounded step and
  before submitting its checkpoint, refresh the exact head, review replies,
  inline comments and CI. Handle material interface/proof-boundary feedback before
  building dependent changes; preserve reviewed commits and keep deltas small.
  At the `0c218cbd` checkpoint there were no submitted reviews or comments.

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

Evidence: [historical diagnostic results](../../research/2026-11-18-etna-shared-core-guest-compatibility.md),
[gate tests](../../../crates/protocol-etna/tests/certified_envelope.rs), and
[#305](https://github.com/taikoxyz/raiko2/pull/305). These are not real proofs of
#22412. The diagnostic workspace/lock additions were later removed. Root manifest and build-tool changes still require production provenance refresh.

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

### Complete range projection checkpoint (2026-11-20)

`range::verify_range_projection` is a thin caller of the existing per-block
projection. It checks `origin <= p < h < 2^48`, representable/budgeted exact
input count before processing, derives each Comet/EL parent from the preceding
matched block and binds both terminal hashes to the expected endpoints. A
generation's first Comet header has no parent ID; later ranges bind the first
header to p. The result retains original execution/transaction/anchor/committee/
FI spans and inherits the current APP_VERSION policy. It is byte/identity
projection, not a certified range or state-transition proof.

The valid three-block case compiled and failed at the RED stub. Frozen GREEN
passed all **86 protocol tests** (68 prior, including 7 historical controls,
plus 18 range cases), strict package all-target Clippy and workspace formatting.
Five separately authored scratch tests compiled and passed against the same
production source. Forged first/interior Comet and EL parents remain rejected
after resealing descendants and terminal identities; singleton/later-origin,
uint48, count/byte bounds, generation, original spans and each endpoint have
direct controls. The isolated no_std consumer compiles and its 221 normal
feature rows enable no std/use_std/use-std. Independent adversarial source
review and behavioral verification passed.

One nonblocking cost observation remains: the current public-helper composition
hashes each Comet header twice. It does not change acceptance; account for it
when measuring/wiring the real guest rather than widening this slice into an
upstream API refactor. No guest cycle measurement or proof build was run.
Source/runtime pins, manifests/locks, EL/trie, provider/pipeline, production
guests and ELF/provenance are unchanged. Origin/storage/committee provenance,
strict nested witnesses, execution/time/anchor checks, DA/cuts and final backend
composition still gate P02/P03/P05–P08. Phase-one FI restrictions are not yet a
guest implementation. No QC policy conclusion is implied by these tests.

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

The anchor work is an explicit tracked backlog; adding these items does not
mark their implementation or upstream specification alignment complete.

- [ ] **P03-A01 — Range/root inventory:** derive N and L from the actual covered blocks, cross-check requested latest anchor L, include every required origin/committee/rules root and bind selected Q's number/hash to the same job and landing statement.
- [ ] **P03-A02 — Near reference/headroom:** prefer Q=L when it covers the inventory and remains queryable through proving, queueing, retries and inclusion. Cover BLOCKHASH ages 1/256, EIP-2935 ages 257/8191 and expiry at 8192 execution blocks; prefer finalized Q where the query window allows.
- [ ] **P03-A03 — Header/link reuse:** wire the existing generic L1 linkage at the real Etna caller with only the raw future-field adapter, count/byte bounds and expected-root binding it needs. Do not restore the unused independent L1 verifier or a new trie/framework.
- [ ] **P03-A04 — Distant bridge:** retain N..L, then use a short tail or one authenticated Ethereum beacon `block_roots` / `historical_summaries` bridge from L to newer Q. Bind Q's fork-specific beacon-root field from the same raw header, fork/slot/index/list length and original hash of L. Do not confuse 8191 execution blocks with 8192 beacon slots or L1 beacon roots with L2 anchor state roots. Test missed slots, period boundaries, expired reference and missing archive data.
- [ ] **P03-A05 — Anchor/root semantics:** reuse existing MPT reads at the authenticated L1 root, check Etna witness presence/inheritance, generation/age/progression, expected addresses/slots and EL anchor-number/parentBeaconBlockRoot equality. Every new or inherited anchor and recovery snapshot keeps its required authentication.
- [x] **P03-A05a — Structural range transitions:** enforce the generation-first and covered-interior witness/number/inherited-root relation at the existing range caller; authenticate actual L1 root/time, origin floor and the incoming mid-range edge separately.
- [ ] **P03-A06 — Specification gate:** reconcile the retained historical bridge policy with normative PROOF-6.2-33/34 and BATCH-6.3-5..7 before enabling it; current target still describes a linear resource/recovery limit. The candidate's settled-switch exception is not a general historical-anchor bypass.
- [ ] **P03-A07 — Settled-switch candidate:** after source/spec adoption, authenticate an ordinary S(t) with generationStartHeight < S(t) <= p through consecutive same-generation EL hashes/heights back from p's authenticated slot-285 EL hash. Use S(t)'s root only for that rotation's registry witness, still derive D(t)/origin-floor selection and H+2 activation, and reject old-generation/discarded headers. This #22419 candidate does not exempt covered block anchors or recovery R from L1 ancestry.

No historical SSZ guest, archive provider, reference-selection host API or full
origin/committee proof is claimed implemented by this audit.

### P03-A05c — Shared raw-header prerequisite (2026-11-22)

[taiko-mono #22426](https://github.com/taikoxyz/taiko-mono/pull/22426), draft at
`8268eb63d71a6a1602880722f44550eb43bea05b`, extracts the existing ABCI raw L1
header parser into `etna_core::l1_header::L1HeaderView`. Its source base is the
client branch `fbcec295`, not mono main. ABCI delegates to the same parser while
retaining its owned header/wire API. The only manifest/lock change is an existing
local ABCI-to-core edge; no third-party source or version changes. Raiko2 remains
pinned to F31: this is a prerequisite, not dependency adoption or proving support.

- [x] **P03-A05c1 — Portable shared codec candidate:** exact borrowed raw-header
  identity and four positional fields, original 4096-byte limit, error ordering
  and opaque direct-item behavior. The new API has an actual ABCI caller; it
  does not restore the unused standalone L1 verifier.
- [ ] **P03-A05c2 — Review/source gate:** complete upstream review and full native
  client CI before merging the draft; select a compatible reviewed pin before
  downstream use. Recheck the F31-to-selected-source delta rather than upgrading
  unrelated QC/committee semantics during their hold. R06 remains open.
- [ ] **P03-A05c3 — Anchor/reference/time composition:** open the exact carried
  header bytes against the selected authenticated near-reference ancestry,
  compare anchor number/root to the same matched EL header and roll inherited
  facts through the range. Only when p+1 inherits does it need the excluded
  parent's raw anchor opening (PROOF-6.2-47). Then check the exact timestamp
  maximum and authenticated age/origin limits, rejecting parent-time overflow.
  Reuse the existing proof engine for required storage predicates; do not add
  another trie or treat a host read as authenticated state.

Reuse refinement: Shasta's L1 linkage lives inside a private event-bound helper
with its own stalled-anchor policy; it is not a directly callable Etna verifier.
Keep that policy scoped to Shasta and factor only useful low-level link checks
when the Etna caller is present. No generic framework or new storage reader.

Candidate evidence: 58 core unit tests and 27 exact-source adapter/oracle tests
(11 delegated, 11 pinned original and five independent manual-fixture cases)
passed offline against upstream-locked dependency identities. Strict core doc
lint and all-target core/adapter Clippy passed. A separate no_std consumer passed,
and its 138 normal runtime feature rows excluding host proc-macros have no std.
The pinned formatting recipe and real native workspace metadata passed. Full
`just verify`/native-client/Docker checks remain open on the upstream draft;
this harness is not their substitute. No Raiko2 guest/ELF, dependency or runtime
code change is included in this checkpoint.

### P03-A05b — Excluded-parent binding (2026-11-22)

Continue A05a at the existing range and statement functions with the actual
excluded-parent EL header as a required native argument. Bind its number and
canonical hash to the same existing range context. Reuse the covered-header
round-trip check to keep the typed parent fields faithful to those same encoded
bytes; a hash over an ambiguous optional-field representation is insufficient
as a generic field-binding contract. This is header identity/codec verification,
not re-execution or a new consensus engine.

For a mid-generation origin, initialize the existing anchor-transition checks
from the parent's same-generation Etna extraData and anchor root. For the fixed
generation origin, bind identity but ignore its old anchor/extraData: genesis,
Unzen and prior-generation recovery parents must remain supported, with a carried
first-block witness. Do not add an optional parent or a skip-compatible path.
The parent is already needed for execution; this freezes no final guest-input,
ABI, committee or public-input schema and introduces no storage reads.

- [x] **P03-A05b:** verify the required parent's height/hash/field fidelity and
  the incoming mid-generation anchor edge; propagate failures through the same
  statement wrapper and retain genesis/recovery exceptions. Add alias/parent
  mutation and inherited/advancing/regressing boundary controls.

This closes A05a's intentionally open incoming edge at the native range boundary.
L1 witness/root/time authority, origin L1 floor, checkpoint/rules/age checks,
strict DA and the complete execution/proof path remain separate requirements.

A05b acceptance evidence:

- Compiled semantic RED: 37 existing/positive range controls passed and nine
  negative controls failed because the old loop ignored the supplied parent.
- `cargo test --locked --offline -p raiko2-protocol-etna --all-targets -- --test-threads=2`:
  120 maintained tests plus ten independently authored scratch tests passed,
  including 46 maintained range tests. Parent mutations preserve or rebind the
  intended hash as appropriate; descendant chains and endpoints are resealed.
- `cargo clippy --locked --offline -p raiko2-protocol-etna --all-targets -- -D warnings`,
  `cargo fmt --all -- --check` and the locked standalone no_std consumer passed.
  The normal runtime feature audit has 221 rows and no std/use_std/use-std.
- Independent adversarial source review and independent behavioral verification
  passed at the same final production/test bytes. Same-RLP/hash optional-field
  aliases, same-hash wrong height, root/extraData mutation, full generation and
  fee/length checks, incoming witness/root transitions and uint48 boundaries
  reject correctly; canonical shorter origin headers remain supported.
- Two test-only lint fixes changed no production bytes or assertions. Final
  checks used immutable offline snapshots and the existing shared target with
  a 15 GiB reserve. No dependency, guest artifact or backend rebuild was run.

The required native parameter is updated at every actual caller. No optional
compatibility route or host bypass is retained. This is not a finalized guest
transport schema or complete L1/anchor/proof authentication.

### P03-A05a acceptance evidence (2026-11-22)

The existing `verify_range_projection` loop now checks generation-first witness
presence and covered adjacent anchor transitions. Its statement wrapper receives
the same rejection behavior without another API or parser. The per-block checker
still returns original borrowed witness bytes; this is not witness authentication.
For `p > origin`, bind p's actual EL header before claiming the incoming transition
is verified. Future composition must also authenticate anchor roots/time, origin
floor, checkpoints, rules, age and recovery state.

- Semantic RED: the 27 original range tests and two new positive controls passed;
  six negative tests failed because the old loop accepted the invalid transitions.
- `cargo test --locked --offline -p raiko2-protocol-etna --all-targets -- --test-threads=2`:
  109 maintained tests plus five independently authored scratch tests passed,
  including 35 maintained range tests. Mutations reseal DataHash, both ancestry
  chains and endpoints, so rejection is attributable to the new transition check.
- `cargo clippy --locked --offline -p raiko2-protocol-etna --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, and the locked standalone no_std consumer passed;
  its normal runtime feature audit has 221 rows with no std/use_std/use-std.
- Independent adversarial source review and independent behavioral verification
  passed. Generation-origin singleton, recovery-origin, mid-generation first-edge
  boundary, witness omission/redundancy, uint48 anchor values and changed/missing/
  zero inherited roots have behavioral controls. Tests use immutable offline
  snapshots and the existing shared target with a 15 GiB reserve.

No source pin, manifest, lock, EVM/trie, guest entrypoint, production ELF/VK or
final ABI changed. R06/R07 and the complete proving path remain open. The current
head's xtask failure was read back as the existing RISC0 source-fingerprint gate,
not a failing native protocol test.

### Immediate BFT schedule binding / QC requirements

Implement the P03/P05 field relation using existing `Validators.hash()` and
caller-authenticated active/next sets at each height, with complete `(p,h]`
coverage and no host skip flags. Check both hashes even at non-QC heights and
check next/active coherence across neighbors. Derive h plus the outgoing endpoint
of every change in signing keys or assigned powers; metadata-only rotation adds
no target and aggregation child boundaries do not create protocol QCs.

- [x] Add compiled semantic RED, the minimal field-binding/target derivation and meaningful full-range/mutation boundaries.
- [x] Run focused and independent exhaustive-model tests, strict scoped Clippy, formatting and standalone no_std checks; obtain independent source review.
- [x] Verify an exact ordered commit list at all derived targets through the shared native QC checker, using the corresponding checked header/active set and the canonical deployment domain.
- [ ] Compose these helpers with independently derived schedules, authenticated ancestry and the final adopted QC policy. Matching supplied schedules and native certificates alone do not establish schedule authority or full proof validity.

Implemented in `crates/protocol-etna/src/coverage.rs`. For independently
supplied active/next sets and authenticated full-landing endpoints, the helper
checks exact counts, consecutive uint48 heights, both commitments at every height
and next/active coherence. It reuses the shared canonical cached keys/powers
hash. The result is a requirement list, not a certified range or verified QC.

Acceptance evidence on host Rust 1.95:

- The valid A,A,B,B,A range compiled and failed against the stub before GREEN;
  the implemented relation returns the outgoing endpoints plus h.
- `cargo test --locked --offline -p raiko2-protocol-etna --all-targets -- --test-threads=2`: 68 passed, including 7 explicitly historical gate controls, 11 schedule cases, 5 model cases, 17 projection cases, 12 statement cases, 13 signed native-QC cases and 3 independent certificate cases.
- The independent model checks 1,360 short schedules using explicit sorted keys/powers, 10,016 current/next hash substitutions and 3,648 disconnected adjacent schedule mutations. It is finite regression coverage, not an unbounded theorem or composer test.
- `cargo clippy --locked --offline -p raiko2-protocol-etna --all-targets -- -D warnings` and `cargo fmt --all -- --check` passed. A separate locked no-default-features `#![no_std]` consumer compiled; its 221-record normal-feature closure enabled no `std`, `use_std` or `use-std` feature.
- Independent adversarial source review and independent behavioral verification passed. Immutable offline source, the existing shared target/lock, two jobs and a 15 GiB reserve were used. The native-QC and review-fix slices keep the existing runtime pins, EL/MPT, provider/pipeline and production ELF bytes; the review fixes restore both guest manifest/lock pairs to base.

These checks do not authenticate the supplied schedule, derive origin/H+2 or
verify header ancestry, DA, execution, FI or a final proof. Native commit
signatures are checked only by the explicitly pinned native helper below.
Aggregation must pass the whole landing endpoints; a shorter child cut cannot
stand in for them. P03/P05 remain partial until the real composition is tested.

`verify_required_native_commits` first reuses the complete schedule binding,
requires exactly one commit at each derived height in ascending order and checks
all target heights before any native signature verification. It derives the
chain ID from the full `DeploymentDomain` and uses checked offsets to select the
same height's header and active set for `verify_native_commit`. It does not add
origin, embedded-LastCommit, aggregation-child or non-target certificates.
Callers must bound and canonically decode raw commit witnesses before this API.

The signed tests cover A-to-B-to-A, a missing outgoing commit, extra/duplicate/
reordered targets, a later malformed target before an invalid first QC, wrong
signed header or outgoing keys, full-width chain IDs/Inbox/generation, exact
2/3 rejection, nil/absent votes and malformed present slots. The independently
signed vote-domain mutation preserves the header/BlockID to isolate signature
domain binding. A re-signed APP_VERSION mutation proves the inherited restriction
is the rejecting predicate. The native positive RED first passed the shared QC
checker, then failed at the new stub before implementation. Two test-only lint
issues were fixed; the final frozen source passed all checks and independent
source/behavioral review.

P01 must also reconcile F31 `verify_native_commit`'s APP_VERSION restriction with
the candidate's explicit guest field table before using it as the complete new
range certificate relation. This coverage component changes no version policy
or legacy gate and does not silently solve that adoption question.

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


### Reuse inventory (2026-11-20)

This is integration work, not a new EVM or blob-verification implementation.
The existing stateless path already checks ancestors, header/body/receipts and
post-state root using the witness DB and native executor. The current Alethia
pin `59e06d0` has no Etna fork, so a later timestamp alone cannot select it.
An inspected upstream candidate `efc3c162` supplies timestamp-gated strict
committed-list/anchorless execution and a devnet override; it is not an adopted
runtime pin. Raiko2 still needs the matching trusted fork profile and narrow
scoping of legacy zero-root, first-successful-receipt and anchor assumptions.
The old `efc3c162` candidate's 2 s fee target was an audit finding, not the
latest source: #248 at `ad584325` now selects 4 s for Etna, including the first
Etna block. This resolves that source difference only. The PR remains unmerged,
and its runtime/API/SDK compatibility has not been tested in Raiko2. Network
timestamps are configuration, not a reason to duplicate the upstream executor
or trust a host fork override.

The provider's Beacon fetch/order path and both guests' shared KZG/versioned-
hash checks are reusable. Etna's remaining DA seam is strict version-6 payload
framing/decompression and exact ordered item equality with the matched range,
using the same verified hash list in the landing statement. Reuse the blob
unpacker, with Etna padding/full-chunk checks; do not route Etna through the
Unzen manifest's offset/default-on-error semantics. Pending #22422 makes the
first field header's high-bit check explicit; adoption and client/guest parity
remain P01/P05 tasks, not verified production support.

## P05 — Complete range relation and settlement output

**Files:** range/statement modules in `crates/protocol-etna`, existing guest
composition, and new `consensus_range.rs` / `land_statement.rs` integration tests.

- [x] Derive terminal h plus each interior signing-set endpoint from supplied sets and verify each required native quorum under its corresponding keys/powers and full deployment domain. Identical keys/powers across metadata changes add no target.
- [ ] Authenticate the actual origin/recovery schedule and adopted QC field policy before composing those native checks into the final range proof; supplied sets alone are not committee authority.
- [x] Compose the bounded complete Comet/EL header-hash chains and original envelope/EL bytes against the caller-authenticated origin/terminal expectations; preserve the generation-origin exception.
- [ ] Join that byte projection with P03's authenticated context and schedule, current/next-validator hashes and the required native QCs over the same headers. Do not add stock full-part-set/embedded-LastCommit checks to the revised hash-link relation. A later committee cannot replace the outgoing committee's endpoint certificate; origin p does not require a QC merely because it is an input boundary.
- [ ] Authenticate and decode ordered blobs and references against the complete supplied range; preserve all per-block anchor, ruleset, stop, FI and execution checks.
- [x] Provide the pure 21-word preimage/digest and ordered cut-hash encoders (ENC-A-29/30/35). The statement recomputes nested commitments from the same generation, p/h, blobs and supplied cut leaves.
- [x] **P05-C01 — Range/statement projection:** bind all shared full-width domain/origin/terminal fields, the matched terminal header state root and each interior cut to the existing range projection and supplied uint48 cursor sequence; return the existing statement digest. This checks cursor shape, not actual FI consumption or L1 authority.
- [ ] Derive and authenticate p through h cuts and terminal values from the actual verified range, with every interior height in order. Bind the computed bytes32 statement digest to the final proof using the pinned backend framing; add no ValState field.
- [ ] Preserve overlapping settlement and historical origins without reproving a suffix. Reject wrong leaves, blobs, references, generations and cross-fork boundaries.

**Accept:** singleton, no-handover and multi-handover ranges; range ends immediately
before/at/after a handover; missing outgoing/terminal QC; same-set recurrence;
interior heights without detached QCs; omitted/duplicate/swapped leaves; `p=c` and
`p<c<h`; `c>=h` cancellation; and reference expiry. Validity is established for
all `(p,h]`, not merely the newly settled suffix.

### Range/statement projection evidence (2026-11-20)

`range::verify_range_statement_projection` composes existing range projection,
`cut_leaf` and `LandStatement::try_digest`. The existing per-block verifier now
retains its already-checked Comet hash for that actual cut caller. It introduces
no hash pass, EVM/trie/KZG implementation, dependency pin, manifest/lock or
production guest/ELF change. Shape validation remains shared with the original
encoder, and errors preserve their source chain.

Compiled semantic RED preserved all 18 original range tests while the nine
new tests failed at the explicit composition seam. Final native evidence is
95 protocol tests (nine new), five independently authored signed-fixture
checks, strict all-target Clippy, workspace fmt and a locked standalone no_std
consumer. Its 221 normal runtime dependency rows, excluding host proc-macros,
enable no std/use_std/use-std. Independent source review and behavioral
verification passed. Two test-only Clippy fixes changed no production bytes.

Equal-count substituted cuts that the old shape-only encoder accepts now
reject, including changed height, EL/Comet identity, cursor and order. Full-width
domains, historical proof origins, singleton and uint48 boundaries, wrong
terminal state root and resealed broken ancestors have direct controls.
Supplied cursor validity, execution, DA, L1/committee authority, phase-one FI-off
profile and backend proof framing are still composing-verifier obligations.
The result is not a full certified range or proof. No guest cycles, SDK build
or real proof was run. R06/R07 and #22423 authority review remain open.

### P05-D01a — Decompressed execution-list equality (2026-11-22)

[`verify_decompressed_execution_payload`](../../../crates/protocol-etna/src/payload.rs)
checks the canonical outer RLP list prefix followed by each original projected
execution-item span in order, with no second full payload allocation or
transaction/header decoder. It rejects empty ranges, mismatched bytes and
lengths, and actual or expected encodings above 64 MiB including the prefix.
The caller still authenticates the complete range, bounds inflation during
execution and establishes strict compressed-stream and ordered blob provenance.
D01b, execution, anchors, FI-profile authorization and final guest/proof
composition remain open; this is not complete DA verification.

Acceptance evidence for this bounded component:

- Compiled semantic RED: all seven initial payload tests failed at the explicit
  unimplemented seam before implementation.
- `cargo test --locked --offline -p raiko2-protocol-etna --all-targets -- --test-threads=2`:
  101 maintained tests and three independent scratch tests passed, including
  seven historical earlier-relation controls. The six new maintained tests
  cover exact original empty/signed spans, count/order/substitution, canonical
  framing/truncation/extra roots and the exact inclusive 64 MiB boundary plus
  one-over actual and expected sizes.
- `cargo clippy --locked --offline -p raiko2-protocol-etna --all-targets -- -D warnings`
  and `cargo fmt --all -- --check` passed.
- A locked standalone `#![no_std]` consumer exporting the new API passed
  `cargo check --locked --offline --lib`; the normal runtime feature graph
  excluding host proc-macros contained 221 rows and no std/use_std/use-std.
- Independent adversarial source review and independent behavioral verification
  passed. Checks used immutable offline snapshots and the existing shared target
  with a 15 GiB disk reserve. No manifest, lock, backend, guest entrypoint,
  production ELF or proof-output change is included.

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

### Production ELF source provenance gate

CI's RISC0 source fingerprint mismatch is separate from native test results.
The fingerprint covers the production guest's local source closure plus the
whole root manifest, backend guest manifest/lock, toolchain inputs and all build-tool source,
including its tests. At `aeda3262`, against comparison base `3f4ec387`, four hashed
inputs changed:
`Cargo.toml`, `guests/risc0/Cargo.toml`, `guests/risc0/Cargo.lock` and
`xtask/build-guest/src/lib.rs`. Replacing just those bytes with base versions
reproduced the recorded fingerprint. Both checked-in RISC0 ELFs still match their
provenance digests and are unchanged; no current-source rebuilt bytes or image ID
are established. The Etna crate is not yet in that production guest's closure.

The later diagnostic cleanup restored both production guest manifests and locks
to comparison base `3f4ec387` byte for byte. Root manifest and build-tool source
still differ, so both production provenance records remain a merge gate.
P07 must run `just build-guest all --force` on the final source, review generated
ELF/VK/provenance changes, then pass `just build-guest all --check`. Do not hand-edit the recorded source fingerprint
or weaken the check. A legitimate rebuild may leave ELF bytes identical while
refreshing provenance. The stale source check runs before artifact verification
and RISC0 precedes SP1, so its first failure does not establish that SP1 artifacts
are current.

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
