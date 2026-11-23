# Shared upstream Etna proving implementation

## QC scope hold (2026-11-22)

The user paused QC and committee implementation while the new L2-managed
membership/penalty design is unsettled. Revisit node-code reuse only after
its authoritative interfaces are known. This supersedes the old committee
provenance and L1 public-input proposal as an immediate implementation target;
#22423 remains provisional. Existing native QC components are not a completed
or frozen future proof relation. Do not remove them or invent replacement
interfaces during this pause.

Continue independently with execution fork compatibility and exact DA payload
binding, reusing existing executor/trie/KZG. The first DA component checks only
that the complete decompressed bytes equal the canonical list of the projected
original execution items, in order and within 64 MiB; the caller authenticates
context and complete range membership.

The user's DA clarification selects a fail-fast derivation relation: the guest
uses the existing blob codec and a mature zlib decoder over the proposal-bound
bytes, requires successful complete derivation with no remaining input, and
asserts exact equality with the entire proven block range. Execution and the
state-root relation remain proved by the existing executor. Any failure stops
verification; no recovery, default payload, shortened execution or host success
flag can produce an accepted proof.

Do not implement a separate strict DEFLATE decoder or generic malformed-input
hardening project. Reuse library checks and retain protocol framing/size limits.
The chosen decoder still has to agree with public derivation. The earlier
miniz-only streaming proposal's acceptance differences explain why it was not
adopted, not a requirement to patch miniz. The actual pinned client dependency
already enables flate2's C-zlib backend in the existing guest closure; inspect
that selection before adding another dependency. Keep std codec adapters in
existing guest-common and the pure protocol-etna byte-binding layer portable.
B02 in the existing plan tracks packing, complete derivation and composition.
Actual blob/KZG provenance, FI-off profile and final guest/proof wiring stay open.

## Independent native closeout (2026-11-23)

The phase-one native authenticated range/statement composer rejects every FI
witness and cursor consumption, while retaining unchanged nonzero historical
cursors and the generic FI-capable projection. The existing range verifier now
requires present/nonzero roots on first/advancing carried anchors as well as
its existing inheritance checks. The excluded origin parent remains opaque.
Neither check authenticates the disabled/empty-queue deployment profile, L1
anchor root, execution result or final proof. The plan records exact native
regression and independent review evidence and the remaining source gates.

## Native authenticated DA / statement checkpoint (2026-11-23)

B03 composes the existing primitive proof-of-equivalence and versioned-hash
helpers with the same InputDataSource blob vector used for complete derivation.
The outer native range/statement function takes its expected hashes directly
from `LandStatement::blob_hashes`, invokes the existing range checker with the
actual execution transaction type, and returns the projections/digest only after
real KZG and exact payload checks succeed. It adds no cryptography, transport,
strategy selector, duplicate execution path or host-success bypass.

Forty maintained plus four independent native tests with real KZG proofs and
focused lint passed; original crypto sources are unchanged. Real production
formatting and default/bench locked metadata for both guests passed. The two
standalone locks now contain the existing Etna dependency closure and preserve
SDK/crypto pins; their prior metadata-resolution failures are repaired without
building ELFs. RISC0's flate2 lock is aligned to the existing root 1.1.8 constraint.

L1 authority, EVM execution/state root, anchors/time/rules, QC/FI and final
production guest/output/proof integration remain separate. The existing ABI
codec is provisional. These native checks do not establish ELF/VK freshness,
actual guest library linkage, target performance or accepted ZK proofs.

## Native DA composition checkpoint (2026-11-22)

`raiko2_guest_common::etna::verify_blob_execution_payload` now composes the existing
pinned BlobCoder, complete v6 C-zlib decode and the existing portable
payload-to-projection matcher. It borrows raw blob views, preserves chunk/item
order and fails directly on packing, derivation/exhaustion or exact-byte mismatch.
No EVM, trie, KZG implementation, default-block fallback or custom DEFLATE
parser was added. The root lock adds only existing dependency edges.

The selected adapter matches the current native lander, including nonempty final
chunks. Bare BlobCoder accepts an empty chunk and ENC-A-23c states only its upper
bound; this implementation does not relax the public decoder or redefine that
specification. Host/native success is not final guest acceptance.

The 20 maintained plus four independent source-harness tests and focused lint
passed. Actual production-workspace formatting and locked metadata passed;
existing Etna regressions passed. Complete guest-common/workspace validation,
standalone guest lock updates and actual target-library/ELF/proof evidence remain
open. The native harness used system zlib; final profiles must confirm their
selected backend/library. Existing ordered KZG/hash/statement bindings and full
range execution must still be composed before any accepted proof output.

## Native parent/anchor boundary checkpoint (2026-11-22)

The existing native range and statement functions now require the excluded
execution parent header. Its height, canonical hash and typed field fidelity
bind to the existing range context. The same anchor-transition checks cover
p to p+1 and all later edges inside a generation; at generation origin, the
parent's old anchor fields remain opaque and the first block carries a witness.
This reuses the execution parent and covered-header codec checks, without new
storage reads, execution logic, dependencies or final public-input fields.

The context still requires independent authentication. Carried L1 witness
contents, root/time authority, origin floor, checkpoint/rules and age checks,
strict DA, execution and final proof acceptance remain open. The native helper
argument does not freeze the final guest transport schema.

## L1 authority boundary amendment (2026-11-20)

Protocol discussion: [taiko-mono #22423](https://github.com/taikoxyz/taiko-mono/pull/22423)
(draft; provisional representation, protocol-owner review pending).

User direction: L1-verifiable state facts should be supplied through the proof
statement by L1 itself. Raiko2 host/provider reads the values, the guest binds
what it uses to public inputs, and the landing contract obtains or checks the
authoritative counterpart. Preserve native signature/quorum, execution and
data bindings in the guest. Reuse shared derivation only where it remains a
guest obligation; do not introduce another storage/trie implementation.

This supersedes the earlier blanket guest-side committee provenance direction
where a coordinated protocol change supplies equivalent L1 authority. It does
not mean the current protocol already does so. At protocol source `7cd9606a`,
the 21-word statement covers L1 settlement context but not authoritative
historical keys/powers or their schedule. `activeCommitteeSnapshot` is currently
a proven output supplied in calldata. Adding a field requires an actual L1
authority source and consistent rules/codecs, not merely hashing a host value.

The provisional candidate is a canonical signing-authority hash covering the
full range's current/next sets and activation context. A smaller variant binds
L1-authenticated registry checkpoints and retains shared selection in the
guest. Exact representation, historical cutoff binding, recovery/overlap
semantics and L1 costs remain under protocol review. Existing original-witness
validity rules must be reconciled before deleting their guest checks.

Track this under P03-L01..L04 in the existing plan. Local registry-proof
components are tested but unpushed/unwired candidates; their guest integration
is paused while this authority seam is reviewed. No contract ABI, guest
behavior, dependency pin or production artifact is changed by this amendment.

## Current planning baseline (2026-11-19)

The forward implementation target is the merged design in
[taiko-mono #22412 at `93b59fe6`](https://github.com/taikoxyz/taiko-mono/tree/93b59fe629dc8987ecb03c7d59d1f66004de068f/packages/protocol/docs/etna).
The exact updated design was [approved](https://github.com/taikoxyz/taiko-mono/pull/22412#pullrequestreview-5968429349)
after the unconditional landing-level recovery audit was added, then merged as
`2e6763e21bc236cfcda6011d164b32040f9e0c8d` at that same reviewed head. That fix changes
no proving/ABI rule. Approval is not implementation compatibility: the inspected
client `af1b952a` and protocol `4c6424bc` still implement the earlier settlement
and app-hash interfaces, while execution candidate `53f33b70` also changes the
Reth/Alloy dependency graph. P01 keeps those adoption gates open. The byte codec
in `crates/protocol-etna/src/statement.rs` implements the specified layout with
independent ABI vectors; it is not a complete verifier or generated-runtime ABI
conformance. No runtime/SDK dependency pin changes are made by that slice.

For planned guest work, this amendment supersedes earlier priority, app-hash,
grid and settlement-interface assumptions:

- The guest does not replay proposer priorities or prove a ValState preimage,
  and does not check Comet `app_hash`. The node commits EL hash plus ValState in
  `app_hash`; that does not remove guest envelope/EL parent-chain checks.
- Keys, powers, committee provenance, schedule and strict weighted quorum remain
  proved. Both current and next validator hashes must match the authenticated
  schedule at every height, including non-QC endpoints (PROOF-6.2-18).
  Required QCs are h and each signing committee's final height in the
  range; all other block identities are bound by the authenticated ancestry.
- A range can end at any height. Commit every interior cut; use the specified
  21-word statement preimage and its 32-byte Keccak digest, eight-field
  LandInput, four-word cut leaf and three-word
  slot 285 record, with no ValState settlement fields.
- Every valid block must be provable alone, including any required FI singleton
  trial. A fixed aggregation boundary must not reintroduce K or mandatory
  intermediate certificates absent from the protocol relation.
- Registry selection follows the final no-displacement implementation; shortened
  evidence retention and app-import priority handling remain node responsibilities.

Existing `845ecbcb` verifies the earlier per-block relation. Its body helper
compares `app_hash` to the bare EL parent and it verifies the actual embedded
predecessor QC. Those policies need adaptation for this target, not a new EVM
or trie backend. The old seven tests, six independent groups and patched-SDK
CPU diagnostic remain historical feasibility evidence, not new-design proofs.

Use the [P00–P08 progress tracker](../plans/2026-11-17-etna-shared-proving.md#milestone-progress)
for current deliverables and completion gates. The user resumed implementation
on 2026-11-19; retain the existing shared-cache and resource policy. Separate PR
reviews change this plan only when their dependency or proof-rule impact is verified.

## Devnet phase-one scope (2026-11-20)

The user selected a first devnet phase with FI disabled, followed by FI support
without a separate proving architecture. Preserve the v6 witness slots, FI
cursor fields in origins/cuts/statements and the final input/output shape.
The phase-one composer must bind the restricted deployment/profile, carry the
authenticated cursor unchanged and reject FI consumption/witnesses; no host
skip flag may turn off a required proof check. Actual deployment/profile
binding remains an implementation gate. The later phase adds authenticated
queue/eligibility/trial checks to that same path, reusing existing execution.
The unrestricted normative FI relation and its worst-case trial acceptance
are phase-two completion gates, not phase-one claims. All non-FI execution,
Comet/committee, anchors, DA and output/proof checks remain required.

The fewer-QC proof-goal discussion in #305 remains open. Continue byte/identity
composition independently without changing the current certificate policy.

## Range projection checkpoint (2026-11-20)

The narrow `range::verify_range_projection` caller now derives intermediate
Comet/EL identities from the same matched blocks and checks both expected
endpoints over a complete bounded `(p,h]`. It reuses the existing per-block
projection, keeps original witness/transaction spans and the generation-origin
exception, and adds no EVM, trie, recursive proof or certificate policy.
Its context remains caller-authenticated; the eventual composer must establish
origin/committee authority, execution, anchor/time rules, DA and final cuts/output
on those same inputs. Structural success, including preservation of an opaque
FI witness, does not imply phase-one FI admissibility or full proof validity.
Current native evidence is 86 protocol tests, five independent scratch cases,
strict all-target Clippy, formatting and a separate no_std consumer. The current
helper boundary computes each Comet header hash twice; optimize or measure that
at real guest composition without changing the proof relation. No cycle or
production guest/proof acceptance claim is made.

## Range/statement binding checkpoint (2026-11-20)

The new pure composing caller binds the same projected range to its landing
statement, checking every interior four-word cut, full-width shared context,
terminal header state root and supplied cursor sequence. It reuses existing
range and statement algorithms and retains the already-verified Comet hash
for the cut caller, without another hash pass. Cursor shape is not FI validity;
the caller must still prove actual cursor effects or the authorized FI-off
profile. L1 context/committee authority, DA, execution and backend proof-output
binding also remain separate gates. This component does not freeze #22423's
proposed authority field or change the fewer-QC policy.

Native validation: 95 protocol tests, five independent signed-fixture checks,
strict all-target Clippy, fmt and a standalone no_std consumer passed. Existing
EVM, trie, KZG, guest entrypoints, dependencies/locks and ELF are unchanged.
P04 is upstream fork compatibility plus narrow Raiko2 legacy-assumption
adaptation; P05 DA reuses KZG/unpacking and needs strict Etna payload-to-range
binding. Neither task calls for a new execution or blob-proof implementation.

## Per-height committee binding checkpoint (2026-11-20)

`protocol-etna::coverage::bind_committee_schedule` binds every supplied header's
current and next validator commitment to independently authenticated expected sets, checks their neighboring
schedule edges and derives required commit heights for the complete `(p,h]`.
Signing-set identity uses the existing canonical keys/powers commitment; record
or stake metadata alone creates no new QC requirement. The terminal h is always
required, along with every interior signing-set endpoint, including A-to-B-to-A.
This interpretation is explicit in reviewed candidate
[taiko-mono #22419 at `d32f4568`](https://github.com/taikoxyz/taiko-mono/blob/d32f4568bde1b3a2824af40d8ee3985063b1c09e/packages/protocol/docs/etna/SPEC.md).
That candidate is not adopted wholesale, and its node/import review is separate.

The schedule-only helper returns requirements, not verified certificates.
`verify_required_native_commits` additionally checks the exact ordered targets
before verifying native certificates under the corresponding header/active set
and canonical full deployment domain. It directly reuses the shared checker;
there is no parallel signature or threshold algorithm.

The future composer must derive/authenticate the schedule and landing endpoints,
bind every header's domain/ancestry and use the adopted QC relation. Child-proof
boundaries cannot redefine landing endpoints. The pinned native helper still
requires APP_VERSION=1 at targets, which needs P01 alignment with the revised
field table. Its tested native checks are not a complete revised range proof.
The P00–P08 tracker records these open gates and the explicit P03-A01–A07 anchor backlog. Existing EL/MPT paths stay authoritative.

## Reuse audit and L1 reference strategy (2026-11-20)

The user reaffirmed that new implementation should principally cover Etna
BFT/PoS and its proof bindings. Account/storage MPT verification, ordinary EL
header/body/transaction-root validation, execution, witness pooling and generic
L1 header linkage already exist in Raiko2. Reuse those authoritative paths and
add only a demonstrated Etna adapter; do not create parallel validators.

The audit removed the unused `protocol-etna::l1` implementation and its dedicated
test/vector scaffold from checkpoint `1bc5bea6`, and removed the projection's
duplicate transaction-root computation. The existing stateless consensus and
pre-execution path remains responsible for that predicate on the exact matched
block. Its existing `rejects_mismatched_transaction_root` regression remains the
behavioral authority. The projection still binds original envelope bytes, Comet
identity, EL number to Comet height, generation, and the independently expected
EL parent. The latter is not equivalent to checking a host-supplied parent;
remove it only when the composed execution path binds that same authenticated
range parent itself. A projection result is never execution or proof validity.

| Responsibility | Reuse / remaining Etna-specific work |
| --- | --- |
| EL header/body/STF | Existing `raiko2-stateless` strict validator and execution path; adapt the fork rules and exact committed-body input, without reproducing their ordinary checks in projection |
| Account/storage | Existing `SparseState` / `CachedTrie` with the expected authenticated root; thin root/context adapter and Etna address/slot/witness-format checks only |
| Generic L1 linkage | Reuse/factor the existing `guest-common` L1-header linkage when its actual Etna caller is wired; keep Shasta source/checkpoint policy separate |
| Raw L1 representation | Existing Raiko2 typed headers cannot preserve unknown future fields; retain the raw-hash/4096-byte/four-field decoding requirement as an integration gap, not a new unused verifier library |
| BFT/PoS and settlement | Shared core signatures, quorum, committee derivation/schedule plus Etna-specific Comet/EL/DA bindings and revised landing codec |

### Recovered long-distance-anchor decision

This direction was already selected in the
[earlier design, sections 5.1.1–5.1.2](https://github.com/taikoxyz/raiko2/blob/6aed2ef9b33c0ef8bef0d627752c90c3b9188261/docs/superpowers/specs/2026-10-29-etna-pos-proving-design.md#511-l1-ancestry-link-nearby-references-use-a-historical-proof-for-long-delays).
Let N..L be the range's distinct L1 anchor span, Q the execution reference bound
by L1, and P the actual landing height. Distinguish the required anchor span L−N
from the waiting tail Q−L.

| Situation | Reference and guest witness |
| --- | --- |
| Q=L covers every required root and remains queryable through expected landing | Preserve N..L linkage; L1 reads Q's execution hash with BLOCKHASH at age 1..256 or EIP-2935 at 257..8191 execution blocks; no guest proof of the L1 history contract is needed |
| Q=L has insufficient proving/queue/retry/inclusion headroom, or a newer required L1 root exists | Choose a newer queryable Q; retain N..L and bridge L to Q with a short header tail or Ethereum beacon SSZ membership proof |
| Recent beacon history / completed historical period | Use `block_roots` in its strict slot window / `historical_summaries` with the authenticated period, list length and index respectively |

Prefer a Q reported finalized by the local L1 node when the query window permits,
as BATCH-6.3-4 specifies; selecting an unnecessarily unfinalized head creates
reorg/reproving risk. Pin Q's number and hash in the proving job/final wrapper; an
expired reference is not repaired by editing an already proven public input.

The intended cost is the anchor span plus one bridge for L, not a proof for every
anchor or every header in the waiting tail. Other L1 roots needed for committee,
origin or rules witnesses still require authentication under Q; the bridge does
not authorize unrelated roots. Q=L is a policy for Etna's selectable reference,
not a change to Shasta's proposal-metadata-bound origin.

8191 execution blocks is lookup headroom, not a measured proving-cost crossover,
and it differs from 8192 beacon slots when slots are missed. A short tail remains
valid beyond the direct-reference window; do not make SSZ exclusive at a numeric
age threshold. Bind Q's original execution header to the public reference hash;
the bridge uses Ethereum L1 Q's `parentBeaconBlockRoot`, not Taiko L2's field of
the same name, which carries an anchor execution state root. Bind chain/fork,
slot, SSZ path, index and list length, and compare the authenticated execution
hash with the exact raw header L. Missing archive data is an error, not validity.

The existing [historical-proof research and public fixtures](https://github.com/taikoxyz/raiko2/blob/170e0fd7d965b0ee20ac8f9d33295f8fb5d217c6/docs/research/2026-10-29-ethereum-historical-header-proof.md)
are witness-verification feasibility evidence, not an integrated production
guest or a throughput claim. The source branch/commits remain the historical
reference; this amendment restores only its ancestry strategy, not its old
committee authority or superseded settlement ABI.

**Upstream reconciliation gate:** the current normative target at `93b59fe6`
still requires raw parentHash links in PROOF-6.2-33/34 and describes a linear
ancestry resource/recovery limit in BATCH-6.3-5..7. Reconcile that specification
with the retained bridge strategy before implementing or enabling the far path.
The far path also needs fork-aware extraction of Q's `parentBeaconBlockRoot` from
that same authenticated raw execution header: the current four-field rule at
CONS-3.3-6/7 reads only positions 0/3/8/11 and does not supply this field. Include
that extraction in reconciliation rather than trusting a separate host root.
Removing unused scaffolding does not relax those obligations or establish this
gate as complete. Do not silently force all references through Q..N or claim
that the historical bridge is already integrated.

## Direction and authorization

The deliverable is an upstream dependency crate that Raiko2 can call for useful
Etna computations and verification. Committee derivation belongs in that shared
implementation. Raiko2 retains proof composition as well as collection, witness,
scheduling and backend integration. Development dependencies use explicit commit
pins; prefer a reviewed merged Etna development commit for eventual integration.
Commit coherent stages and retain provenance across resource interruptions.

This replaces the L1-authenticated committee-authority proposal in Raiko2 #295
for this implementation. It does not require an additional L1 authority contract.
No implementation, deployment or merge readiness follows from this document.

## Historical partial scope correction (superseded below)

On 2026-11-18 the user explicitly narrowed the upstream work to a callable crate
for Raiko2, not an architectural adjustment of taiko-client-rs. An export, helper
or adapter needs an identified Raiko2 call site or a necessary dependency of that
call site. Existing extraction/re-export changes may supply shared primitives;
keep further client changes to necessary compatibility wiring and fixes in the
extracted surface. Do not reorganize ABCI/lander services, mining, runtime state,
Engine integration or transition orchestration to make the client resemble a
prover framework.

Raiko2 composes required QCs, ancestry and applicable committee/validator hashes
inside its existing guest modules. Under the current planning target, proposer
priorities and ValState replay stay outside those guest obligations. A monolithic
shared authenticated-transition API, or adoption of that orchestration by the client, is not a prerequisite.
If a checkpoint-start call needs a smaller parent-hash/BlockID input, add only
that verification input; preserve the full proving obligations at composition.

The pooled-trie/RPC proof-count comment is conditional integration feedback, not
a mandatory redesign: change RegistryProof only if Raiko2 actually consumes that
interface and an adapter is insufficient. If another authenticated read path is
used, keep snapshot length/cutoff/entries/selection checks complete. The inherited
generation-start helper guard and redundant typed-block decoding were corrected
in upstream review head `61d27093f8970a24eca8d0dc3d711a034c9cea95`. The guard
predates the extraction; the correction also preserves native commitment-error
priority and the host's authenticated Safety classification.
Actual Raiko2 handover/cut fixtures belong to downstream integration validation;
they do not require a client architecture change or immediate downstream work.

## Entire-PR scope correction

The 2026-11-18 correction applies to every commit in #22398, not only recent
fixes. At review head `1d6d1666afec8795cf5203668317a534b339367f`, the complete
diff against `ea74662169262dc8ab6ada7028d2ef54137088d2` adds 31 etna-core files
and changes only the workspace manifest/member and lock package record. All 47
previously migrated native paths, including configuration APIs, CLI callers,
ABCI/lander code, original tests and generators, are restored to their base Git
blobs and modes. There is no existing native consumer of etna-core in this PR.
This supersedes earlier completed-step descriptions of client reexports/adoption.

Deliver the separately callable portable port first. Client adoption is a separate
migration, so temporary source coexistence is explicit and pinned to the native
baseline, with unchanged Go/native fixture copies and conformance checks rather
than a claim of shared runtime implementation. Keep the original generators as
the source for data refresh; do not duplicate their implementation. All 790 old
lock package blocks and the existing native dependency/feature graph are intact.
Independent source and behavior checks cover the entire boundary, core61, strict
Clippy/rustdoc, both zkVM library targets and witness-shape controls. Full workspace,
Docker and production Raiko2 guest/proof integration remain unverified.

## Existing execution backend decision

The user confirmed that Etna work must preserve Raiko2's working low-level
trie/witness implementation. Keep the current `risc0_ethereum_trie::CachedTrie`
backend, witness-node pooling, retained storage tries and post-state-root path.
Do not replace them with the read-only verifier from taiko-mono #22390 or
reimplement the execution database as a prerequisite for Etna.

The upstream shared library provides native BFT/QC verification, committee
selection, scheduling and legacy ValState utilities. The revised guest uses only
the computations its proof relation requires; retaining a node utility does not
make priority arithmetic a guest obligation. Generic account
and storage authentication uses adapters over the existing backends. These
adapters must authenticate values against the expected root; unverified host
values or a success boolean are never sufficient. Sharing protocol rules does
not require replacing the underlying trie implementation.

## Verification-first reuse decision

The user clarified the mining/proving boundary on 2026-11-17: Raiko2 already has
the committed blocks. Reuse client computations when they are useful and have
acceptable zk cost; otherwise verify the claimed result against the same protocol
rules. Shared rules do not require identical producer and verifier algorithms,
and client logic does not automatically become a public upstream API.

Apply three choices to each operation:

1. Proposer policy outside consensus validity (pool selection, candidate ordering,
   retry budgets and payload publication) stays outside the guest; it needs no
   replacement proving implementation.
2. Small deterministic computations may reuse existing pure code, including
   hashes/codecs, schedules and committee derivation. Keep additional upstream
   extraction scoped to a demonstrated Raiko2 call site.
3. When reuse is unsuitable or zk-expensive, expose a narrow verification
   relation over the supplied block/output and authenticated context. Share only
   the primitives needed to keep that relation equivalent to protocol rules.

Assess cycles, witness size, memory and dependency/setup cost on the relevant
RISC0/SP1 paths before claiming an improvement. A supplied result or rejection
reason is an untrusted witness until checked. Known-valid producer results must
verify, valid alternatives permitted by the protocol must also verify, and
invalid mutations must reject. For example, a block's beneficiary is freely
chosen under CONS-3.2-4; the proposer's local fee_recipient must not become a guest
constraint (the current validator preserves this distinction).

Keep Raiko2's existing EVM replay, witness, trie, database, crypto and post-state
root path. Ordinary committed-block validation executes the entire supplied body
strictly and checks its commitments; it does not rerun mining selection or
candidate filtering. Add only demonstrated missing Etna fork behavior. There is
no planned wholesale EVM architecture change or mandatory migration of Raiko2's
execution infrastructure into a client-owned crate.

For FI omission, prove the protocol's completed singleton decision, without
requiring an entire payload builder or a reconstructed empty block as the API
result. An equivalent checked rejection predicate may establish that decision
when justified by the pinned rules; state/zk-dependent cases retain necessary
trial execution and read witnesses. No fast path is accepted without the same
accept/reject semantics as the complete singleton decision.

The user closed the MPT prototype at taiko-mono #22390. This revised design and
its implementation plan are the new baseline; start upstream consensus work
without the prototype commits. Preserve the prototype only as a historical
reference, independent of Raiko2 integration.
Diagnostic guests established that it can link and execute on both backends,
but a controlled SP1 6.8.1 comparison of 15 synthetic cases found material cycle
differences: cold three-/four-slot requests used +9.9%/+18.8% total diagnostic
cycles relative to the pooled CachedTrie path. Shared-path and overlapping
requests also favored the existing path; repeated single-slot reads favored
VerifiedStorage reuse. These are MPT microbenchmarks, not proposal measurements.
No backend replacement follows from that prototype or its guest compatibility.

Review the shared consensus API upstream before starting substantial Raiko2
integration. Prefer an exact dependency commit after that API is reviewed and
merged into the Etna development mainline. #22390 is not this review gate.

## Intent and constraints

Implement complete Etna PoS range proving using the existing RISC0 and SP1
backends and the existing stateless EVM infrastructure. Reuse the node's actual
consensus rules, committee derivation and canonical codecs through upstream
crates rather than maintaining a Raiko2 translation. Do not add proposer-priority
arithmetic to the revised guest.
Keep Unzen inputs, APIs and proof outputs compatible. Start with fixed-range
aggregation; incremental extension, new proof systems, new zkGas pricing and live
activation are separate work.

Initial pinned inputs:

| Source                  | Commit                                     |
| ----------------------- | ------------------------------------------ |
| Raiko2 main             | `3f4ec3879b9f4b002e2a5e14b6ab6552da2ac2d1` |
| Etna Rust client #22280 | `df307bf411ef59b93fc7bdbee318df4f9d99e71d` |
| Etna protocol #22286    | `1ef5aa82c86538d9681b08dd63cb86a867543869` |
| Anchorless EVM #248     | `efc3c162ad54760a84c561da03f44af9c2adffdd` |

Consensus extraction source refresh: use client commit
`ea74662169262dc8ab6ada7028d2ef54137088d2`, one commit after the initial client
pin, for the new implementation branch. The inspected relevant delta restores
Unzen handoff, adds switching configuration, preserves generation-zero committee
0 above S, and made the then-current checkpoint grid relative to S. This records
the extraction baseline. The current target removes the grid while preserving
authenticated origin and committee timing; no #22390 commit is included.

The shared committee/parameter/schedule/ValState extraction checkpoint is
`4fb1de48c5870bf36c8bd6c7c756df0bf5435550`. It passed independent source review,
focused native behavior checks and actual RISC0 count-regression execution. The
native Comet/QC/body checkpoint is now
`ccc9221f7d5d3c33bb3d8a62387c8f2606698bfe`, published for early upstream API review
as draft [taiko-mono #22398](https://github.com/taikoxyz/taiko-mono/pull/22398).
Independent source reviews and representative actual RISC0/SP1 guest challenges
passed. Envelope/FI/transition and range proving remain later work, along with
full repository verification. At that historical checkpoint, the lander
rejected nonzero genesis heights. The later inspected client implements
switching-origin handling. Raiko2 must still authenticate that origin in the
guest. The draft checkpoint preceded the shared-API review gate for dependency
adoption.

The pre-merge compatibility diagnostic at
`e6e186fe6a2e3912739db9b9cae1740e31064824` passed in the actual Raiko2 RISC0
3.0.5 and SP1 6.8.1 guest workspaces with their existing crypto patch tables.
Both guests executed the QC, committee, ValState and recorded cut-preimage
fixture and emitted the same 684-byte journal as native and the stock oracle.
At that historical checkpoint, the updated guest locks affected production ELF
provenance. They have since been restored to the base production graph. The [historical diagnostic
results](../../research/2026-11-18-etna-shared-core-guest-compatibility.md) cover CPU compatibility.
The shared API subsequently squash-merged into feat/taiko-client-rs-abci at
`f31e7ec55345ac7faf99756504535b0cc9929ec0`, not taiko-mono main. That is a reviewed
development pin, with mainline provenance still a main-merge gate. Executable
diagnostics have since been removed from production guest workspaces; the
linked evidence is historical and commit-pinned.
Checkpoint `845ecbcb` pins that revision in `crates/protocol-etna` for the earlier
lower-level Comet consensus gate. Its caller authenticates deployment, origin,
expected height and parent EL hash, the active committees and the predecessor's
header and full BlockID. The gate verifies the current certificate and body and
independently checks the embedded LastCommit when present. Its output is the
borrowed committed transaction and signed Comet identity. Guest context
authentication, envelope/EL binding, execution, FI, DA, range verification,
proof generation and L1 verification remain open.

The extraction baseline generated protocol artifacts from
`35e2f8f191132e87d9a223554d0b699f4203f2e3`. The later inspected client
`48db1326d52495ba5f97dfca3ed8ccacbe92e85f` uses
`4c6424bc97b226d100cc9ce9dc828c9567acef4e` and implements the SignalService
switch. These are source references for the next integration step. Before
adopting a newer protocol pin, compare ABI, statement encoding, storage layout
and shared vectors, then regenerate artifacts through their existing entrypoint.

## Architecture

The shared protocol layer exposes deterministic checks, encodings and useful
computations. The verifier checks already supplied blocks and derives the complete
proof statement, using narrow adapters over Raiko2's existing deterministic EVM
execution and witness infrastructure. Proof-input, range and guest orchestration
may remain in existing Raiko2 modules; creating an upstream proof crate is a
choice based on concrete reusable checks, not a prerequisite. A host-provided
success/empty boolean is never an execution proof.

```text
existing node -> unchanged native implementation (pinned source/vector oracle)

Raiko2 witnesses -> existing guest -> etna-core primitives -> statement/proof
                                          |
                                existing EVM / witness DB
```

Port only useful computations and verifiers with their tests. Keep node validation
and construction unchanged for this upstream milestone; client adoption is a
separate migration. Equivalent verifier algorithms are permitted,
with shared protocol constants/primitives, differential cases and negative tests.
Keep RPC, Tokio, Engine transport, persistence, metrics, signing and CLI policy
outside the portable dependency closure.

Use the existing fork protocol/pipeline/guest module boundaries first. Shared
checks must have a portable dependency closure; no guest imports the complete
ABCI/lander service. A separate proof/orchestration crate requires a concrete
benefit over those existing module boundaries.

### Full-prover obligations, not mandatory upstream crate contents

- Canonical envelopes, execution headers, domain/generation and chain identity.
- L1 header/account/storage witness authentication and anchor progression.
- Registry snapshots, committee selection, voting powers, records and H+2 changes.
- Comet header/BlockID/body bindings, the range-derived required certificates,
  canonical secp256k1eth verification, unique signer accounting and strict
  `3*signedPower > 2*totalPower`.
- Complete ancestry and cuts at every interior height, without priority arithmetic
  or a validator-state settlement commitment.
- FI entry authentication, canonical eligibility, ordered cursor movement and
  deterministic singleton execution when a skip requires it.
- Rules schedules, origin/recovery boundaries and complete range continuity.
- Blob/KZG binding, cuts and the final Solidity-compatible statement.

A commit certificate is verified; the guest does not replay the BFT networking,
prevote/locking or timeout state machine.

### Existing FI skip semantics

This work adds no new skip policy. Preserve the existing Etna pure FI decision
and the acceptance relation of the EL singleton build. An ineligible entry may be
skipped. An eligible entry may be skipped only when the completed singleton
outcome on the authenticated parent, matching attributes and full block budgets
would be empty. The guest may verify an equivalent rejection condition rather
than reproduce irrelevant block assembly. Included EVM revert/OOG counts as
inclusion. A transaction discarded for zk-limit or metering overflow retains
necessary trial execution and witnesses for attempted reads; timeout, resource
exhaustion or unavailable state does not justify a skip. RPC Engine results are
preflight hints; the guest independently establishes the decision. New shortcuts
require explicit equivalence coverage against the pinned singleton semantics.

### Contract and prover interfaces

Align with the final accepted upstream `IEtnaInbox.LandInput` and settlement
implementation through P01. The current design target has eight input fields,
a 21-word / 672-byte statement preimage, a four-word cut leaf and three-word
checkpoint record. Its Keccak digest is the bytes32 statement passed to the
Inbox verifier. Final journals/public values must authenticate that digest in
the exact pinned backend verifier framing, including required domain wrapping.
It carries EL/Comet identities, generation, FI cursors, reference, blob hashes
and every interior cut, with no ValState fields or landing grid. The old 23-word
implementation is historical source, not a compatible substitute. Generate and
compare the exact ABI/encodings instead of inventing a parallel settlement ABI.
Every final backend proof covers the same complete statement; an execution-only
proof plus a QC-only proof is insufficient.

Define missing prover-side request/witness/journal interfaces in the existing
Raiko2 modules or a narrowly justified shared verifier library, and expose them
for review. Typed requests preserve immutable proof origin
and complete range even if the settled frontier advances. Bind all cache/artifact
identities to protocol/generation/rules/image/content. Transport collection never
substitutes for guest authentication.

For a switching-network proof whose parent is Etna start S, include the L2
SignalService proxy account and EIP-1967 implementation slot witness under the
settled S header's state root, and compare the slot with the implementation
built into the guest for the L2 chain id (PROOF-6.2-46a). The originWriter's
S timestamp precheck belongs to START-10-5a; reading the implementation's
`beaconRootsFrom` and comparing it with node-local fork configuration belongs
to the node trust path in SYNC-9-5a. Neither adds another guest witness or check.
Current PreparedBatch is not a complete GuestInput and its restore checks do not
replace witness rederivation. Blobs do not contain all original consensus history.

For the special Unzen-to-Etna transition, retain the agreed stop/settle/restart
boundary: settle final Unzen first, then start Etna at H+1. No cross-fork batch is
required. Normal Etna committee changes and recoveries remain in scope.

## Verification and resource policy

Treat this as high-risk proof/consensus work. Use independent adversarial review
and independent behavioral verification, with focused cross-language vectors and
negative cases. Resolve material findings and have their raiser recheck fixes;
use at most two review/fix waves before surfacing unresolved scope/disagreement.

First establish portable shared-crate and RISC0/SP1 target compatibility. Current
upstream host MSRV is 1.99, while installed guest compilers differ; host build
success is not guest compatibility. Preserve the existing backend crypto patch
configuration and inspect build entrypoints before running code.

Use one configured shared host target, bounded build concurrency and a common
build lock. Keep backend-specific guest outputs in the documented guest build
locations; never leak the host target override into guest build subprocesses.
Do not create per-worktree host targets, run duplicate full builds, remove others'
caches/worktrees or install unrelated SDKs. Record free space before expensive
work; pause costly execution and commit coherent work if resources become tight.
Only isolated credential-free execution may run repository tests/builds.

## Completion evidence

Completion requires the intended Raiko2 callers to use the reviewed crate,
preservation of existing client behavior, matching native/guest decisions and
full valid/reject coverage of committee/QC/FI/origin/cut
boundaries, actual proofs from both supported ZK backends, compatible aggregation
and L1 verification against the pinned contracts, and Unzen regression evidence.
No dummy/native execution is counted as a real proof. Report missing checks,
resource limits and author-reported evidence separately. Do not deploy or activate
the fork, register images or merge PRs as part of implementation.
