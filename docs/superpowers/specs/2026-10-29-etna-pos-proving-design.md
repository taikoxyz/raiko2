# Etna PoS + ZK Proving Design

## Status

**Draft for discussion.** This branch contains design documentation only. Neither this document
nor a documentation-only merge authorizes implementation, deployment, guest-image registration,
or fork activation. Protocol and prover maintainers must review the open interfaces below before
an implementer prepares a separate implementation plan.

The selected responsibility split is:

> L1 authenticates the committee and its applicability. Proving verifies that the QC signatures
> are valid, their distinct signers belong to that committee, and their voting power exceeds 2/3.

This is a proposed change to the current upstream design, not a claim that the upstream contracts
already expose the required committee authority. Reuse the existing zkVM pipeline; no new proof
system, folding scheme, or IVC dependency is required.

## 1. Intent and scope

Support anchor-less Etna execution and PoS-certified block ranges in Raiko2 while preserving the
existing Unzen service. Keep the familiar collection, validation, execution, proof generation and
aggregation flow. Separate committee selection from certificate verification so the same staking
selection logic does not need to be implemented inside every execution guest.

In scope for the eventual implementation:

- authenticated anchor-less execution inputs and exact execution/blob binding;
- stock CometBFT commit verification against L1-authenticated committee contexts;
- forced-inclusion (FI) checks, including the deterministic singleton execution for a legal skip;
- composable range proofs and the final settlement statement;
- provider collection, typed inputs, task/artifact identities and backend integration.

Out of scope for this document's implementation authority:

- staking policy, top-n selection, exits, slashing, consensus operation or DAO recovery policy;
- implementing or deploying the L1 committee-authority mechanism;
- changes to the Bridge, SignalService, validator signers, or live infrastructure;
- network activation, verifier/image registration, dependency bumps or guest ELF generation;
- choosing new zkGas coefficients, a new zkVM, IVC, or an unmeasured throughput promise.

Those protocol components remain dependencies, not checks that may be silently omitted.

## 2. Source snapshot

Initial check on 2026-10-29. All Taiko design and implementation PRs in this table were open at
that check. The ancestry follow-up below records later revisions separately. These sources are
design inputs, not evidence of deployment or real ZK proof generation.

| Source | Revision | Use |
| --- | --- | --- |
| [Raiko2 main](https://github.com/taikoxyz/raiko2/commit/66fffe58821161c0730d59178d91f0fb360d4d5b) | `66fffe58821161c0730d59178d91f0fb360d4d5b` | Branch base and existing pipeline |
| [Anchor-less design #22189](https://github.com/taikoxyz/taiko-mono/pull/22189) | `cf338b1d76eed3736645f4163293f343cc10ad23` | L1 state root in the L2 header, derivation and migration |
| [PoS design #22303](https://github.com/taikoxyz/taiko-mono/pull/22303) | `387bc49f928d4b0634dbd967a5f77044ceb10d86` | CometBFT, landing, FI, recovery and proof statement |
| [Anchor-less protocol #22222](https://github.com/taikoxyz/taiko-mono/pull/22222) | `7d431abb879358ab02ca7239f6f1b2f1083fc1e8` | Contract integration reference |
| [Anchor-less client #22263](https://github.com/taikoxyz/taiko-mono/pull/22263) | `e599edd604fe53ee66fde9535216dae193d431c4` | Client integration reference |
| [Anchor-less relayer #22242](https://github.com/taikoxyz/taiko-mono/pull/22242) | `5564f8a0f0295bd17c0bf252cd0776f569a5fdb8` | Signal proof consumer reference |
| [PoS protocol #22286](https://github.com/taikoxyz/taiko-mono/pull/22286) | `9ba657df9093a61ddc5832640ffb05d7b58ed9f3` | L1 landing and registry reference |
| [PoS client #22280](https://github.com/taikoxyz/taiko-mono/pull/22280) | `0dbc996ffbcfc61c2576285eb9200bd78b4c033f` | Envelope, validation and lander reference |

Authoritative design passages at this snapshot:

- [Anchor-less header, root authentication and EIP-4788](https://github.com/taikoxyz/taiko-mono/blob/cf338b1d76eed3736645f4163293f343cc10ad23/packages/protocol/docs/etna-anchorless/DESIGN.md).
- [PoS specification](https://github.com/taikoxyz/taiko-mono/blob/387bc49f928d4b0634dbd967a5f77044ceb10d86/packages/protocol/docs/etna-pos-zk/SPEC.md), especially sections 3, 4.3, 5, 6 and 7.
- [Acceptance checklist](https://github.com/taikoxyz/taiko-mono/blob/387bc49f928d4b0634dbd967a5f77044ceb10d86/packages/protocol/docs/etna-pos-zk/ACCEPTANCE.md).
- [Current client proof boundary](https://github.com/taikoxyz/taiko-mono/blob/0dbc996ffbcfc61c2576285eb9200bd78b4c033f/packages/taiko-client-rs/README.md#future-validity-proof-boundary): the provider returns dummy `0x00`; real guests are not implemented there.

Upstream PoS section 6.2 currently asks each guest to authenticate registry snapshots and derive
committees. This document deliberately replaces that responsibility with L1-authenticated committee
contexts. The current landing statement does not include the proposed committee-context commitment.
Protocol approval and corresponding L1 changes are prerequisites; this design is not wire-compatible
with the current PoS verifier interface as written.

Older sketches that put an L1 **block hash** in `parentBeaconBlockRoot`, omit the anchor number from
`extraData`, or require an `OlderThanWindow` inference do not describe the current anchor-less design.
Implementers must refresh this source table before planning against upstream code.

Ancestry follow-up on 2026-10-29: PoS design #22303 subsequently merged; its
[updated section 6.3](https://github.com/taikoxyz/taiko-mono/blob/689c1d50cbe2b65f14620cc90288ea64de1743c2/packages/protocol/docs/etna-pos-zk/SPEC.md)
still describes a linear ancestry budget of 131,072 headers. The
[L1 reference reader at #22286](https://github.com/taikoxyz/taiko-mono/blob/1b14b634c8d213b5b30d8fc61ca6f6dcec344389/packages/protocol/contracts/layer1/etna/impl/EtnaInbox.sol)
authenticates a recent execution block hash using BLOCKHASH or EIP-2935. Section 5.1.1 proposes
another way to authenticate older anchors; it does not claim that the current upstream ancestry
policy or guest implementation already supports that path.

Reference-window follow-up on 2026-10-30: the current
[#22286 implementation](https://github.com/taikoxyz/taiko-mono/blob/405b5afba4bba604509a425fea159fca9fa63c9d/packages/protocol/contracts/layer1/etna/impl/EtnaInbox.sol#L783)
retains that reader and lets the caller choose any queryable reference. Its
[contract tests](https://github.com/taikoxyz/taiko-mono/blob/405b5afba4bba604509a425fea159fca9fa63c9d/packages/protocol/test/layer1/etna/EtnaInboxLand.t.sol#L427)
cover ages 256, 257, 8191 and 8192. The PR remains open: this is source-level support, not a claim
of merge or deployment. Guest historical-Merkle validation remains unimplemented.

## 3. Responsibility boundary

| Layer | Responsibility |
| --- | --- |
| L1 committee authority | Authenticate membership, voting powers, total power, deployment/generation, snapshot and the effective height range; authorize changes and recovery contexts |
| Raiko2 preflight | Collect blocks, original envelopes, commits, committee material, L1 headers, witnesses and blobs; diagnose missing or contradictory inputs |
| Guest | Validate execution and derivation rules; verify QC signatures, membership, distinctness, threshold and context applicability; bind all outputs |
| Aggregation | Verify child proofs, rule/image identity and boundary continuity; bind the complete final landing statement |
| L1 landing verifier | Resolve authoritative committee contexts, construct the exact public input, verify both backend proofs and apply the permitted settlement suffix |

The guest does not select the top n, infer eligibility from a host-provided stake table, or accept
an arbitrary committee because its signatures verify. Host-side checks are diagnostics, never the
source of authority for a proof.

L1 must authenticate a committee **before or atomically with accepting the landing**. A caller's
claimed root, an event not authenticated to L1 state, or a value first stored merely because the
same proof asserts it is not an independent committee authority. A separately verified committee
proof may supply authority if the protocol explicitly selects that mechanism.

## 4. Committee contexts and public inputs

A committee context conceptually binds:

```text
deployment domain: L1 chain, L2 chain, Inbox and protocol/rules version
recovery generation
committee identity and authenticated selection snapshot
effective first and last L2 heights
member-set commitment and total voting power
```

These are semantic fields, not a finalized Solidity struct or hash encoding. Canonical encodings
must be shared by L1, clients and guests. Reuse upstream committee ordering and set commitments
where possible; do not introduce a second incompatible membership format just for Raiko2.

The initial recommendation is to supply the complete canonical member list for each distinct
context, recompute its commitment and total power in the guest, and reference members by index.
The guest validates key encoding, unique identities, positive bounded powers, canonical ordering
and agreement with the public commitment/total. This verifies the supplied set, not how L1 selected
it. Merkle membership witnesses can replace full lists later if measurements justify them, without
weakening total-power authentication or signer uniqueness.

For a cross-committee range, public inputs bind an ordered context schedule, for example through
one `committeeContextsHash`. The guest proves that every certified height has exactly one applicable
context, with no gaps, ambiguous overlaps or substitution by an old generation. L1 resolves and
authenticates the same schedule against its authority when constructing the proof input. The
implementation may use a compact schedule representation, but cannot validate only the terminal set.

The proposed final statement is the upstream landing statement plus a binding to the authenticated
committee schedule and its interpretation/version. This requires a coordinated statement-domain
and image change; it is not a host-only extra field. L1 derives the authority values rather than
trusting calldata, and the proof generated earlier must already bind those exact values.

### Protocol dependency that must be resolved

Upstream ordinary selection uses the L1 anchor of D(t), with H+2 activation and recovery-specific
rules. Storing a valid member set for an arbitrary snapshot does not establish that it applies at
a particular L2 height. The protocol must specify how L1 authenticates this snapshot-to-height
relationship, or explicitly retain guest validation of the schedule against authenticated context
while L1 supplies the member-set authority. Neither choice requires the guest to rerun top-n
selection. The exact authority interface, historical retention and final statement encoding remain
review items; no implementation may assume a mapping that current contracts do not provide.

## 5. What the guest proves

### 5.1 Execution and anchor-less validation

- Execute the committed transaction list using the reviewed, pinned alethia-reth implementation.
  Compare the resulting block hash, state root, receipts, gas used and zk gas with the committed block.
- For post-fork blocks, `parentBeaconBlockRoot` is the authenticated L1 anchor's **execution state
  root**; the 13-byte `extraData` carries the anchor number. EIP-4788 records that root before user
  transactions. It is not an Ethereum beacon root or an L1 block hash.
- Authenticate anchor headers through the L1 reference ancestry and enforce the mode's anchor
  progression, time and inherited-root rules. P2P or host data alone cannot authenticate an L1 root.
- Do not require an anchor transaction or its reserved gas for Etna. Preserve standard system-call
  processing, including empty blocks, and keep pre-Etna checks on the existing path.
- A committed transaction list cannot be silently shortened after a zk-limit exceedance. Proposer
  candidate selection and verification of the final certified body have different semantics.

There are two input modes, not two competing execution implementations: the anchor-less Shasta
proposal mode retains proposal/source derivation; the PoS mode validates decided CometBFT envelopes.
PoS `extraData[1..7]` is a generation, not a legacy proposal ID. Do not force PoS ranges through the
old proposal-event decoder. Share execution primitives while keeping these protocol bindings explicit.

### 5.1.1 L1 ancestry: link nearby references, use a historical proof for long delays

Let the distinct L1 anchor heights used by the range start at N and end at L. Let Q be the recent
L1 execution reference authenticated by L1 when the PoS landing is submitted at height P,
with `N <= L <= Q < P`. Q is chosen before final proof generation; it need not equal P-1.
Separate the range's own anchor span `L-N` from the submission-delay tail `Q-L`.

The selected initial policy for **Etna PoS landing** is:

- **L stays queryable through the expected landing:** choose `Q=L`, provided no other L1 root
  whose ancestry the guest must authenticate is newer than L. The guest preserves the existing
  parentHash linkage from N through L; there is no delay tail after L to hash or prove.
- **Q=L is ineligible or expected to expire:** choose a newer queryable Q covering all required
  L1 roots. Preserve N..L linkage and authenticate only header L against Q's beacon root, using
  `block_roots` for a recent beacon block or `historical_summaries` for a completed period. Do not attach one proof
  per anchor or hash every header in the submission-delay tail.

The contract already supplies the selected reference hash through this interface:

| Age at actual landing `P-L`, when `Q=L` | L1 hash source | Extra guest bridge after L |
| --- | --- | --- |
| 1..256 execution blocks | BLOCKHASH | None |
| 257..8191 execution blocks | EIP-2935 system contract | None |
| 8192 or more | Q=L is no longer queryable | Select a newer Q and authenticate L to it |

The EIP-2935 call is performed by L1. The normal guest path does not need an account/storage
proof for that contract. At preparation head H, Q=L has `8191-(H-L)` future execution blocks of
headroom. Budget final proving, queueing, retries and inclusion; a currently readable reference
at age 8191 has no headroom for the next block. No fixed margin or product config is selected here.

```text
normal: N -- parentHash links --> L = Q, whose hash L1 reads directly
old:    N -- parentHash links --> L -- Merkle proof --> Q's beacon root
```

8191 is a reference-query bound, not a measured zkVM performance crossover or an anchor-gap
validity cap. The initial policy chooses a Merkle bridge when the direct reference cannot cover
the expected landing. A short header tail to a newer Q remains an equivalent valid witness and
can be selected as a cost optimization: at actual age 8192, Q=L+1 needs only one link, although
that Q itself has no remaining headroom. Do not make Merkle exclusive above an age threshold.
The host chooses the representation; the guest verifies the same canonical ancestry either way.

This reference choice does **not** automatically apply to existing Shasta/proposal mode, whose
origin number/hash is fixed by proposal metadata. Standalone anchor-less proposal mode must retain
that source binding unless its protocol changes. Nor may Q=L omit a newer L1 root required by the
guest's statement. Any additional root whose ancestry the guest must prove still needs its own
authenticated witness under Q; choosing a sufficiently new Q alone does not authenticate that
root. The one-bridge claim here covers the N..L anchor headers. Committee authority/applicability
remains governed by sections 3 and 4.

Keep the existing execution reference number/hash as the public authority. The proposed far-path
witness authenticates header Q against that hash, then uses Q's **Ethereum L1**
`parentBeaconBlockRoot` as the trusted beacon root. This is different from the L2 field of the same
name, which carries the anchor's execution state root. A beacon root must never be substituted
into a public field that means execution block hash. The experiment below also checked a direct
L1 EIP-4788 lookup; that lookup does not by itself require a new production statement ABI.

For a completed historical period, the guest verifies:

```text
authenticated beacon root -> BeaconBlock.state_root -> BeaconState.historical_summaries
  -> period.block_summary_root -> old BeaconBlock -> execution_payload.block_hash
  == keccak256(RLP(header L))
```

Bind the chain/fork interpretation, slot, historical-list index and length, and exact SSZ paths.
Check the authenticated header's number, timestamp and state root as required by existing anchor
rules; an unauthenticated host root or a successful RPC response is not ancestry evidence.
For a recent beacon block not yet included in a completed period, use the corresponding
`block_roots` membership path and its strict slot window. The vector covers 8192 **beacon slots**;
EIP-2935 covers 8191 **execution blocks**. Missed slots mean those limits are not interchangeable.

The far-path cost is proportional to `L-N` header checks plus a small historical proof, rather
than all `Q-N` headers. Crossing an 8192-slot historical-period boundary within N..L does not
require another Merkle proof: only L receives the historical proof. Repeated anchors are deduplicated.
Allowed anchor age, progression and unusually large gaps within N..L belong to protocol/L2 block
validity rules. Proving must enforce those rules without inventing an additional anchor-gap cap.

**Measured witness, not a ZK throughput claim:** a complete Fulu historical proof connected
execution header 25,198,983 to a beacon root authenticated at execution height 26,298,983, a gap
of 1,100,000 blocks / 153.37 days. It used 60 SHA-256 Merkle siblings: **1,920 bytes for the entire
historical bridge**, plus headers and metadata, not 1,920 bytes per anchor. The roughly 339 MB
state downloaded to construct the witness was host-side source data, not GuestInput. A second
recent root reused the same lower 25 siblings. Follow-up tests also verified recent `block_roots`
witnesses with 34 siblings / 1,088 bytes and the 256/257/8191/8192 reference boundaries. See the
[experiment and committed proof fixture](../../research/2026-10-29-ethereum-historical-header-proof.md).

Providers must supply and retain the historical branch material: ordinary beacon RPCs may expose
old blocks while disabling state export or pruning the data needed to construct branches. The
fixed old-block-to-period branch is available after the period completes and can be archived;
the upper branch is generated for the selected recent state. A full proof is bound to that root,
not automatically reusable with every future root. Reference lookup windows still apply to Q;
if a direct EIP-4788 lookup is selected, its root lookup window also applies. An expired reference
needs a newly bound wrapper or an explicitly authenticated retained reference, not changed calldata
on an existing proof. Archive availability and large-input resource handling remain engineering
work, not grounds to treat missing data as a valid witness.

The upstream linear ancestry budget must be reconciled with this policy before implementation:
`Q-N` must not force rejection/recovery solely because the submission delay is large when the
historical path can authenticate it. This does not silently remove other recovery rules or
freeze a new landing interface. Committee authority/applicability remains a separate dependency.

### 5.2 BFT certificate verification

For every CometBFT block in the proved range:

1. Authenticate its original envelope against `data_hash` and bind the envelope's execution block
   byte-for-byte to the block that was executed. Verify Comet header/BlockID construction and the
   EL/Comet parent links, including the parent EL hash in `app_hash` under stock ABCI semantics.
2. Use the L1-authenticated committee context applicable to that height and generation. Check
   `validators_hash` and `next_validators_hash` against the authenticated current/pending schedule.
3. Verify stock secp256k1eth canonical vote sign bytes and signatures under the deployment's full
   `chain_id`. A counted precommit must certify this block's complete BlockID at the correct height
   and commit round; a nil vote never counts as approval of that block.
4. Authenticate each counted signer's key and assigned power against the member commitment.
   Count each member at most once; reject malformed, substituted or duplicate counted identities.
5. Enforce the strict threshold with checked arithmetic: `3 * signedPower > 2 * totalVotingPower`.
   The denominator is the complete authenticated committee, not only the supplied signers.

A QC does not directly sign an EL block hash. The Comet header/envelope/execution binding above is
essential. The guest verifies a commit certificate and protocol validity, not the validator's
entire prevote/locking history, proposer election execution, or slash evidence.

### 5.3 Forced inclusion

Authenticate FI queue entries, raw transaction bytes, due status and cursor transitions under the
same L1 roots and execution context. An included FI transaction must occupy the required position.
For a due eligible transaction that is skipped, reproduce the complete deterministic singleton
build on the committed parent, with full block budgets and the same attributes.

An ordinary EVM revert/OOG counts as inclusion if it fits the zk budget. A fully discarded zk-limit
trial can justify an empty singleton result only under the pinned shared rules. Its witness must
cover attempted state reads and any authorizations; host timeouts or resource exhaustion do not
prove a legal skip. Actual proof cost includes this counterfactual work and the first crossing
atomic operation, even when final block zk gas is small.

## 6. Range proofs and final settlement binding

Keep `p`, `c` and `h` distinct:

- proofs and blobs cover the complete range `(p,h]`;
- L1 reads its actual checkpoint `c` atomically at landing;
- only the suffix `(c,h]` advances settlement and the FI head.

Every grid cut binds height, EL hash, Comet hash and FI cursor. Each backend authenticates the
entire `cutHash`; L1 compares the cut at c with its retained landing record before verification.
For p <= c < h, a frontier advance does not require changing the existing proof or blob payload.
For c >= h, the target is already settled. A generation transition invalidates the old job.

The final wrapper also binds the complete ordered execution payload to the blob versioned hashes,
under the upstream encoding/KZG rules, and authenticates L1 reference ancestry. Blob commitment
validation is a cryptographic binding, not a trusted host checksum.

These semantics reuse the existing proof-and-blob model, but the Etna landing journal differs from
the existing Shasta transition-array commitment. Reuse orchestration and backend machinery, not an
unchanged Shasta public-output ABI.

### Incremental composition without requiring IVC

Retain fixed-range aggregation as a supported first implementation. Design internal journals so
adjacent segments can also be composed or extended recursively:

```text
segment(a,b) + segment(b,d) -> segment(a,d)
```

Each child binds its domain/generation/rules/image identity, start/end heights and EL/Comet hashes,
FI cursors, committee/schedule continuation and an ordered commitment to its executed content/cuts.
The composition proof verifies both child proofs, exact boundary equality, complete range coverage
and ordered content/schedule commitment composition. An endpoint hash alone cannot authorize
rewriting or omitting the internal history.

Perform expensive execution and certificate work before final blob packing and reference selection
where practical. A final wrapper proves that authenticated segment content is exactly the chosen
blob payload, grid leaves and committee schedule, then binds the reference ancestry. Collect the
necessary ordered data/commitment openings; a segment summary does not magically provide the bytes
needed for final blob construction or a flat final hash. No particular accumulator encoding or
incremental backend API is approved here; an implementer must specify it before implementing
recursive extension. This is optional optimization, not a prerequisite for the first real proof.

Each final backend subproof must cover the complete statement. Execution and QC may be separate
internal modules/child proofs, but an execution-only backend plus a QC-only backend does not satisfy
the upstream two-proof rule. Keep exactly two final proofs, at least one ZK. Mixed backends require
their own matching guest/provider support; dummy or native execution is not a validity proof.

## 7. Integration with the existing repository

| Existing location | Reuse / proposed Etna responsibility |
| --- | --- |
| [Primitives](../../../crates/primitives) and [Shasta inputs](../../../crates/primitives-shasta/src/input.rs) | Share execution witnesses; introduce explicitly versioned Etna inputs/journals rather than disguising them as legacy proposal carry data |
| [Protocol hashing](../../../crates/protocol-shasta/src/libhash/shasta.rs) and [instances](../../../crates/primitives-shasta/src/instance.rs) | Preserve Shasta commitments; implement the agreed Etna statement separately with Solidity-compatible vectors |
| [Pipeline](../../../crates/pipeline/src/forks/shasta/spec.rs) | Reuse preflight/validation/prover stages; add an explicit Etna protocol binding and committee-context collection |
| [Guest common](../../../crates/guest-common/src/lib.rs) and [stateless execution](../../../crates/stateless) | Share execution primitives and validation logic; add certificate/FI/range checks without copying a second STF |
| [Providers](../../../crates/provider) | Fetch and authenticate original Comet history, L1 contexts and execution/FI witnesses; blob-only replay is insufficient for full proving |
| [Provers](../../../crates/prover), [RISC0 guests](../../../guests/risc0), [SP1 guests](../../../guests/sp1) | Reuse backend adapters and recursive verification; add versioned Etna programs and compatible external-provider contracts |
| [Engine](../../../crates/engine), [runtime](../../../crates/runtime) and [queue](../../../crates/queue) | Retain single-instance namespace fencing, durable remote request IDs and immutable artifact publication |

The table assigns responsibilities, not final Rust type or crate names. Avoid copying the whole
Shasta pipeline. New protocol-specific input/journal types are expected; preserving the existing
Shasta serialized GuestInput at the expense of sound Etna binding is not a requirement.

Etna task/cache/artifact identities must include the protocol version, deployment/chains,
generation, complete proof origin/range, committee-context identity, rule/image identity and the
appropriate content/reference commitments for that stage. A live settlement frontier is not a
replacement for the immutable proof origin. Keep parent-independent execution artifacts reusable
only when all their actual proof inputs match. Do not reuse Unzen artifacts as Etna artifacts.

Public HTTP schema and external provider changes need a separate explicit contract review. The
eventual implementation must preserve existing Unzen routes, request identities and output formats;
silently changing the meaning of the v4 proposal identifiers is not an acceptable integration.

## 8. Failure behavior and resource constraints

- Missing envelopes, commits, historical contexts or witnesses mean unavailable inputs, not a
  valid empty certificate, empty block or legal FI skip. Retry bounded collection errors; surface
  contradictory authenticated data as validation failures.
- Blobs can reconstruct the landed EL chain but do not contain all consensus/proving history.
  Define an authenticated archive source and independently reproduce a real proof from it.
- Follow section 5.1.1 for L1 reference and ancestry-path selection. Missing/pruned historical
  material is a collection failure; never accept an unverified anchor, reinterpret the reference
  hash, or introduce a new anchor-gap validity limit.
- Do not silently rebind an in-flight job to a new generation, image, committee or reference block.
  Build a new final wrapper/job identity when its statement changes; reuse only compatible children.
- Proving lag may be nonzero but must remain stable under sustained load. Measure execution,
  certificates, FI counterfactuals, aggregation and wrapper costs separately, then together for the
  two required proofs. After an outage, available throughput must exceed production to catch up.
- Resource acceptance uses the worst legal grid segment, committee size and FI workload, not only
  the gas of successful final transactions. Do not loosen runtime validity rules to pass a benchmark.
- No throughput, recovery duration or deployment readiness is claimed by this documentation.

Follow merged [Raiko2 #274](https://github.com/taikoxyz/raiko2/pull/274) for subsequent compilation:
share the primary host target, check for experiment writers before building, do not run cargo clean,
and do not leak the shared host target into RISC0/SP1 guest builds. This design work builds nothing.
Production dependency pins must reference reviewed alethia-reth main commits, not moving PR heads.

## 9. Review decisions and implementation gate

The user-selected direction is the L1 committee authority plus guest QC verification split.
Alternatives considered:

| Approach | Tradeoff / decision |
| --- | --- |
| Guest re-derives top-n and registry snapshots | Matches current upstream spec, but duplicates selection in each proof path; not the selected direction |
| L1 authenticates committee contexts; guest verifies QC | Selected responsibility split; requires protocol agreement on authority, applicability and public input |
| L1 verifies every QC signature directly | Moves certificate work out of zkVM but adds onchain work proportional to blocks/signers; not selected |

Protocol/prover reviewers must resolve these before an implementation plan is accepted:

1. **Committee authority:** which contract/mechanism authenticates member sets and total power,
   how ordinary D(t)/H+2 applicability is established, and how historical/recovery contexts are
   retained and resolved. Decide which schedule checks remain inside the guest.
2. **Shared statement:** canonical committee/schedule encoding, the updated L1 statement/domain,
   height-scoped guest-image acceptance and one cross-language vector covering a handover.
3. **Input/API contract:** the authentic source for original consensus history, an explicit Etna
   range request and the external prover-provider format. Specify standalone anchor-less support
   versus full PoS support and the Unzen transition boundary.
4. **Delivery scope:** first support fixed-range aggregation. If recursive extension is included,
   freeze its complete continuation journal and content commitment before writing it.
5. **L1 ancestry:** confirm the reference-header/beacon-root binding, historical-witness collection
   and retention, fork-correct SSZ paths, and alignment of the upstream linear ancestry budget
   with section 5.1.1. Use Q=L with sufficient landing headroom when eligible; keep the later
   link-versus-Merkle optimization a proving-cost choice, not a validity rule.

The implementing owner may choose crate factoring, compact witness storage and bounded collection
concurrency, provided these do not change the approved statement or verification responsibilities.

Implementation is deferred until protocol and prover maintainers accept the document, the required
cross-repository contract changes are agreed, and an implementation owner is assigned. This task
stops at the design-review handoff. Documentation approval does not mean the future implementation
has passed validation.

## 10. Eventual verification and handoff

The later implementation plan must include at least:

- Shared Rust/Solidity/client vectors for committee commitments, total power, context applicability,
  canonical Comet sign bytes/BlockIDs and the final statement. Wrong keys, weights, duplicate signers,
  exactly 2/3, wrong height/round/domain and substituted roots must fail.
- H+2 current/next committee boundaries, cross-epoch ranges, historical committees and recovery
  generations. A terminal-only committee check must not accept an invalid earlier QC.
- Exact envelope/execution/blob binding, changed order/count/headers, empty blocks, anchor number
  or root tampering and standard EIP-4788/EIP-2935 effects.
- Near/far L1 ancestry witnesses establishing the same anchor history; late submission with a
  short N..L span; ranges crossing historical-period boundaries; repeated anchors; SSZ list-length,
  index, fork, header and root tampering; unavailable archives and expired references. Replay the
  committed historical fixture, then measure the actual guest paths on supported backends.
- Reference ages 256/257/8191/8192; Q=L with no post-L guest bridge; proof/inclusion headroom and
  reference expiry during work; newer required L1 roots; fixed proposal-origin mode; equivalent
  short-link and Merkle witnesses after the direct-reference window. If a guest-side EIP-2935
  storage witness is ever added, enforce its height window before interpreting a wrapped slot.
- FI inclusion, invalid encoding/nonce/balance, ordinary revert/OOG, over-limit rollback and the
  counterfactual trial witness. Real guest execution must independently reproduce each decision.
- Valid overlapping landings with the identical proof/blob transaction, p/c/h boundary cases,
  cut tampering, cross-generation rejection and successful independent public EL reconstruction.
- Real proofs from both final proof paths, successful L1 verification and finalized landing,
  independent archive reproduction, sustained throughput/backlog measurements and failure recovery.
- Separate adversarial review and independent behavioral verification of the implemented complete
  diff. Native replay, mocks, green client CI and a running devnet are insufficient substitutes.

No Rust, dependency, ELF, provider configuration or deployed system changes accompany this draft.
The next owner should start from this branch's reviewed document and refreshed upstream revisions,
not from an assumed production interface or an earlier anchor-less sketch.
