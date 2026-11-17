# Shared upstream Etna proving implementation

## Direction and authorization

On 2026-11-17 the user selected an upstream common implementation consumed by
both the Etna node and Raiko2 guests. Committee derivation belongs in that common
implementation. Raiko2 remains the collection, witness, scheduling and backend
adapter. Development branches may be consumed at explicit commit pins; landing
upstream on main is not a prerequisite for development. Commit coherent stages
and retain enough provenance to resume after resource-related interruptions.

This replaces the L1-authenticated committee-authority proposal in Raiko2 #295
for this implementation. It does not require an additional L1 authority contract.
No implementation, deployment or merge readiness follows from this document.

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

The MPT prototype at taiko-mono #22390 is independent of Raiko2 integration.
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

The client currently generates protocol artifacts from
`35e2f8f191132e87d9a223554d0b699f4203f2e3`. Before adopting the newer protocol pin,
compare ABI, statement encoding, storage layout and shared vectors. Generated
artifacts must be regenerated through their existing entrypoint, never edited by
hand. A newer source or pin is accepted only after its delta is recorded/reviewed.

## Architecture

The upstream protocol layer owns deterministic consensus interpretation and the
complete proof statement. The execution layer owns deterministic EVM execution
and candidate-admission behavior. The common verifier combines them through
narrow adapters over the existing execution and witness infrastructure. A
host-provided success/empty boolean is never an execution proof.

```text
node RPC/store adapters ----> upstream Etna protocol/verification crates
                                      |
                               execution interface
                                      |
                           existing stateless EVM adapter
                                      ^
                                      |
Raiko2 collection -> witnesses -> RISC0/SP1 thin guest -> proof backend
```

Extract existing modules with their tests and make the node consume the extracted
code. Temporary compatibility exports may preserve node import paths, but cannot
retain a second implementation. Keep RPC, Tokio, Engine transport, persistence,
metrics, signing and CLI policy outside the portable dependency closure.

The concrete crate/file decomposition is recorded in the implementation plan
after the dependency analysis. Favor a pure protocol crate and a proof/orchestration
crate over importing the complete ABCI or lander service into a guest.

### Common deterministic responsibilities

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

This work adds no new skip policy. Reuse the existing Etna pure FI decision and
EL build semantics. An ineligible entry may be skipped. An eligible entry may be
skipped only when the complete singleton build on the authenticated parent, with
matching attributes and full block budgets, yields an empty block. Included EVM
revert/OOG counts as inclusion. A transaction wholly discarded on zk-limit or
metering overflow requires a witness covering its attempted reads; timeout,
resource exhaustion or unavailable state does not justify a skip. RPC Engine
results are useful for preflight, but are independently reproduced in the guest.

### Contract and prover interfaces

Use upstream `IEtnaInbox.LandInput`, its grid leaves and the existing 23-word
`LandStatement` as the starting L1 ABI. Do not invent a parallel settlement ABI.
The statement includes EL/Comet hashes, validator state, generation, FI cursors,
reference, blob hashes and cuts. Every final backend proof covers the full same
statement; an execution-only proof plus a QC-only proof is insufficient.

Define missing prover-side request/witness/journal interfaces in the common
library and expose them for review. Typed requests preserve immutable proof origin
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

Completion requires common-library consumers in both node and prover, matching
native/guest decisions, full valid/reject coverage of committee/QC/FI/origin/cut
boundaries, actual proofs from both supported ZK backends, compatible aggregation
and L1 verification against the pinned contracts, and Unzen regression evidence.
No dummy/native execution is counted as a real proof. Report missing checks,
resource limits and author-reported evidence separately. Do not deploy or activate
the fork, register images or merge PRs as part of implementation.
