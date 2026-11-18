# Shared upstream Etna proving implementation

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

Raiko2 can compose QC, ancestry, applicable validator hashes and ValState checks
inside its existing guest modules. A monolithic shared authenticated-transition
API, or adoption of that orchestration by the client, is not a prerequisite.
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

The upstream shared library focuses on native BFT/QC verification, committee
selection, scheduling, ValState and the associated Etna rules. Generic account
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
   hashes/codecs, schedules, committee derivation and ValState arithmetic. Keep
   the current #22398 scope while its API review proceeds.
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
consensus rules, committee derivation, canonical codecs and validator-state
arithmetic through upstream crates rather than maintaining a Raiko2 translation.
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
0 above S, and makes checkpoint grid offsets relative to S. Keep these semantics
when extracting shared types and schedule logic. No #22390 commit is included.

The shared committee/parameter/schedule/ValState extraction checkpoint is
`4fb1de48c5870bf36c8bd6c7c756df0bf5435550`. It passed independent source review,
focused native behavior checks and actual RISC0 count-regression execution. The
native Comet/QC/body checkpoint is now
`ccc9221f7d5d3c33bb3d8a62387c8f2606698bfe`, published for early upstream API review
as draft [taiko-mono #22398](https://github.com/taikoxyz/taiko-mono/pull/22398).
Independent source reviews and representative actual RISC0/SP1 guest challenges
passed. Envelope/FI/transition and range proving remain later work, along with
full repository verification. The existing lander rejects nonzero genesis
heights, so switching-network `S > 0` integration must be completed separately.
Raiko2 dependency adoption waits for the shared-API review gate.

The pre-merge compatibility diagnostic at
`e6e186fe6a2e3912739db9b9cae1740e31064824` passed in the actual Raiko2 RISC0
3.0.5 and SP1 6.8.1 guest workspaces with their existing crypto patch tables.
Both guests executed the QC, committee, ValState and recorded cut-preimage
fixture and emitted the same 684-byte journal as native and the stock oracle.
The updated guest locks affect production ELF provenance. The [diagnostic
results](../../../crates/etna-guest-compat/README.md) cover CPU compatibility.
Production guest composition, proof generation and L1 verification remain
downstream work subject to the shared-API review gate.

The client currently generates protocol artifacts from
`35e2f8f191132e87d9a223554d0b699f4203f2e3`. Before adopting the newer protocol pin,
compare ABI, statement encoding, storage layout and shared vectors. Generated
artifacts must be regenerated through their existing entrypoint, never edited by
hand. A newer source or pin is accepted only after its delta is recorded/reviewed.

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
- Comet header/BlockID/body bindings and canonical secp256k1eth commit verification,
  unique signer accounting and strict `3*signedPower > 2*totalPower`.
- Per-block validator-state updates, priorities and grid-cut state commitments.
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

Use upstream `IEtnaInbox.LandInput`, its grid leaves and the existing 23-word
`LandStatement` as the starting L1 ABI. Do not invent a parallel settlement ABI.
The statement includes EL/Comet hashes, validator state, generation, FI cursors,
reference, blob hashes and cuts. Every final backend proof covers the full same
statement; an execution-only proof plus a QC-only proof is insufficient.

Define missing prover-side request/witness/journal interfaces in the existing
Raiko2 modules or a narrowly justified shared verifier library, and expose them
for review. Typed requests preserve immutable proof origin
and complete range even if the settled frontier advances. Bind all cache/artifact
identities to protocol/generation/rules/image/content. Transport collection never
substitutes for guest authentication.

When proving starts at Etna start S on a switching network, include the Anchor
proxy account and EIP-1967 implementation witness required by PROOF-6.2-46a.
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
