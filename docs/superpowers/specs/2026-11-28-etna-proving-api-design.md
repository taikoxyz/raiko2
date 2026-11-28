# Etna proving API and 100-block SGX PoC

Status: **DRAFT for review**, 2026-11-28. No new runtime API or intermediate
proof relation is implemented by this document. The first-PoC strategy below is
an engineering recommendation awaiting the user's decision.

## Goal and scope

Produce an SGX proof for 100 consecutive real devnet L2 blocks and demonstrate
L1 acceptance of that exact range. Define an API that can also express individual
block proofs, reusable range proofs and final aggregation with lander-selected Q.
Complete the recent-window anchor witness connection before implementing the new
API. Client production integration is not a prerequisite: a fixture/request tool
may supply the same original client data through the backend boundary.

FI remains disabled. RISC0/SP1, at-least-14-day history, selective near-node proof
serving, IVC, queue redesign and trie/EVM replacement are outside this PoC.
The lander selects Q, constructs the final landing/DA request and submits it;
Raiko2 verifies supplied material and produces proof artifacts.

Baseline: Raiko2 #305 at `54ec279a`; client #22280 at `87f95412`; protocol #22286
at `9b8d5b4c`; active design #22425 at `4bb5cff1`. Recheck these pins before runtime
implementation. Existing `/prove/etna` v1 verifies a complete Q-bound range;
its trusted profile allows at most 64 blocks and its body cap is 64 MiB. There
is no Etna child-proof aggregation path today.

## Approaches and proposed first milestone

1. **Direct 100-block landing first (recommended).** Supply raw blocks and
   witnesses with Q, reuse the complete current acceptance relation, and extend
   the reviewed count limit to 100 with focused boundary/real-data checks.
   This exercises the full SGX/L1 path with the smallest relation change. It
   does not claim to consume precomputed child proofs.
2. **Reusable children in the first milestone.** Produce Q-independent block
   and range certificates, then authenticate their assumptions and aggregate
   with Q. This is the target three-level protocol, but requires a new child
   statement, verified-child trust rules and a split of the current Q-dependent
   acceptance relation. It is additional proof logic, not an HTTP wrapper.
3. **Q-bound children.** Keep today's statement on each child. Simpler local
   reuse, but Q must be chosen before the children and changing Q invalidates
   them. This does not satisfy late Q selection and is not the target design.

The proposed implementation order is anchor witness -> direct 100-block SGX/L1
acceptance -> reusable child proofs. If the user requires approach 2 immediately,
its child-statement and trust review becomes a prerequisite to first acceptance.

## One submission interface, three proof kinds

Keep one backend submission endpoint, `POST /prove/etna`, with a versioned tagged
request. Use `schema = "raiko2-etna-request-v2"` and `kind` as the discriminator:

| Kind | Proven interval and input | Output | Q |
| --- | --- | --- | --- |
| `block` | One supplied L2 block `(p,p+1]` plus required context/witnesses | Intermediate block certificate | Absent |
| `range` | Contiguous supplied blocks or verified child certificates `(p,h]` | Intermediate range certificate | Absent |
| `landing` | Complete supplied range or verified children, original DA and L1 context | Final on-chain statement and SGX subproof | Required |

A block is a one-element range semantically. The tags express proof purpose;
they do not create three EVM implementations or three orchestration services.
The direct first milestone implements `landing` with `source.type = "blocks"`.
Unimplemented combinations explicitly return `unsupported_operation`; they never
return replay results labeled as successful child-proof aggregation.

Illustrative landing request (field names are proposed, not an existing codec):

```json
{
  "schema": "raiko2-etna-request-v2",
  "kind": "landing",
  "request_id": "sample-request-001",
  "profile": "etna-devnet-sgx-fi-disabled-v1",
  "expected_statement_hash": "0x...",
  "source": {"type": "blocks", "payload": "existing owned range and execution witnesses"},
  "landing": {
    "statement": "current 19-word statement inputs",
    "data_source": "original ordered blobs, KZG commitments and proofs",
    "l1_history": "L1HistoryInput, including exact Q header and all required anchors",
    "reference_state": "existing Q-rooted Inbox account/storage witnesses",
    "verifier_address": "0x..."
  }
}
```

The example uses descriptive placeholders; the actual codec must reuse owned
Rust carrier types, put each field in exactly one authoritative location and
reject unknown fields/illegal tag combinations. No duplicated Q or statement
value may override the original input. The complete source payload follows the
existing execution/witness pooling types and canonical native block bytes.

`profile` selects an image-supported immutable policy; it cannot supply arbitrary
chain specifications, genesis/fork constants or a trusted signing identity.
For `block`/`range`, `landing` is forbidden. For `landing`, Q and the expected
landing digest are mandatory. A proof source contains ordered child statement
preimages and proof artifacts, not bare untyped proof bytes or remote URLs.

The response has an Etna-v2 typed artifact carrying `kind`, statement preimage,
statement digest, backend/program-policy identity and backend proof. A landing
artifact additionally carries the existing raw SGX subproof and the concrete
verifier-domain input hash. The statement digest and SGX signature-domain digest
are separate fields. Intermediate artifacts are never valid L1 subproofs merely
because their backend also uses SGX.

Retain the existing synchronous backend request/response ownership for the PoC.
A host caller may use existing scheduling/retry infrastructure; this document
does not add a new durable job store or expose Etna through the current `/v4`
queue without implementing that integration. V2 replaces the unlaunched diagnostic
v1 when implemented; no permanent dual interpretation is required.

## Q-independent intermediate statement: obligations before implementation

An intermediate certificate proves a local relation under explicitly committed
L1/context assumptions. It does not yet assert that those anchors are canonical
under an L1-accepted Q. The final prover must close every deferred obligation.
A later Q may make those obligations unsatisfiable; accepting a child certificate
does not guarantee that any future landing is valid.

The typed child statement must bind at least:

- Version, proof-purpose domain, L1/L2 deployment, generation, rules and verifier
  program/profile identity. A child-purpose certificate cannot authorize landing.
- `(p,h]`, both EL and Comet endpoint hashes, parent/post execution state roots,
  and any boundary state needed for consensus/derivation continuity.
- The ordered original block/envelope commitments and records needed to construct
  the exact final cut leaves and DA bytes. A bare pair of state roots is insufficient.
- Every consumed distinct L1 anchor's original header hash/number/state root and
  all per-block associations. Equal anchor heights cannot name conflicting hashes.
- Exact context assumptions used by local checks, including origin/configuration,
  anchor-age/timing/rules/U predicates and relevant committee bootstrap material.
  Separate predicates checked in the child from predicates deferred to landing.

The first child format may carry the ordered per-block/anchor records. Parents
recompute commitments from those records; hashing child digests together is not
claimed to reproduce a hash of the flattened original records. Exact typed
encoding/hash domains and the verified-versus-deferred predicate table must be
reviewed before child-producing code is implemented. There is no host boolean
that marks an unverified child or context assumption as trusted.

For the initial SGX child implementation, reuse the current same-instance rule
in `crates/sgx-runtime/src/aggregation.rs`: the parent obtains its own identity
from the existing TEE provider, requires every child to name that exact instance
ID/address and verifies its signature against the typed child statement/domain.
The caller cannot choose the expected signer. Add Etna purpose/profile checks;
do not reuse Shasta carry-data or accept cross-purpose signatures. Existing
software tests do not establish actual enclave isolation or L1 registration.

Cross-instance child acceptance is deferred and explicitly unsupported until
attested/registered-key authorization and revocation are specified and tested.
A self-declared address/quote or arbitrary enclave signature is insufficient.
ZK recursion later uses authenticated image/VK policy, not SGX signature rules.

## Aggregation and finalization rules

All intervals use `(parent_height,last_height]`. One hundred blocks means
`last_height - parent_height == 100`; the excluded parent header is context.
Children must exactly tile the requested interval in order, with no overlaps,
gaps, duplicate blocks or implicit truncation. Joins check EL and Comet hashes,
execution state continuity and all relevant consensus/context boundaries.
Generation, deployment and rule/program policy mismatches reject.

Required QC coverage is derived from the actual complete range relation. Splitting
work into children neither drops required QCs nor invents new consensus rules at
artificial child boundaries. Raiko2 verifies supplied certificates; it never
collects votes or signs Comet proposals.

The final lander-selected Q must authenticate **every** used anchor and required
reference context. Verify each a_i -> Q through SSZ (or exact direct-Q equality),
including all original header/root bindings and anchor ordering. Shared Merkle
helpers may be deduplicated; no anchor's membership may be omitted. Q is not a
replacement for the synchronized L1 boundary of the L2 blocks.

Finalization also checks the original ordered blob/KZG binding, exact current
landing statement and cut leaves, configuration/origin and remaining Q-dependent
predicates. A flattened anchor-list hash alone does not authenticate the anchors.
The final SGX proof retains the current contract's `VERIFY_PROOF` domain and
concrete verifier address; no contract ABI change is proposed here.

Changing Q leaves valid Q-free children reusable, but requires new reference
witnesses and a new final proof. The lander owns Q expiry checks/reselection.
A changed L1 origin/configuration can invalidate a landing despite unchanged
children. A competing settled prefix is not subtracted from an old proof:
request a new matching suffix or discard/recompute according to existing policy.

## Identity, failures and bounds

`request_id` is correlation, not proof authority or a sufficient cache key.
Any task/cache identity must bind kind, protocol/profile/program policy, exact
ordered source content/child identities and requested output; landing identity
also binds Q, DA and concrete verifier domain. Reusing an external id for changed
content must not return an old artifact. Do not introduce a second runtime epoch,
lease or task-lifetime scheme alongside the existing architecture.

Reject malformed bytes, invalid proofs, statement mismatch, discontinuities,
wrong profiles and out-of-window anchors explicitly. Distinguish missing provider
data from cryptographic rejection so the host can retry appropriately. Successful
response means an actual artifact of the requested kind, not mock execution.

The direct-PoC count target is 100, replacing the current 64-block profile only
after boundary tests and a real 100-block resource check. The 64-MiB body cap and
existing byte/node budgets are not silently removed. Measure the real range's
request size/memory and set reviewed bounds if needed; a count target is not a
guarantee that every maximally sized 100-block range fits the service.


Also preflight the actual deployment's unsettled limit U and checkpoint
synchronization. Existing acceptance requires
`block_height - anchor.checkpoint <= unsettled_limit` for every block. A valid
100-block landing cadence needs enough U headroom for all those differences,
including landing/anchor synchronization delay. Merely raising the service count
cap does not make that cadence valid. Read current authenticated configuration
and observe the checkpoint path before claiming this PoC schedule works; do not
relax the predicate or mutate deployment settings as part of this design task.

## Delivery order and acceptance evidence

1. **Anchor connection:** trusted actual devnet L1 profile; a recent-window host
   witness builder; native `L1HistoryInput` verification for the real 4h/24h
   samples and multiple distinct anchors in one request. Include skipped slots,
   mismatched Q/root/height, omitted/substituted anchors and out-of-window cases.
2. **Direct 100-block proof:** construct a real complete request without client
   production integration; execute current EVM/QC/context checks in SGX; verify
   100 allowed / 101 rejected under the proposed profile and all other bounds.
3. **Actual SGX/L1 acceptance:** build/run the enclave, confirm registered instance
   and deployed single-SGX acceptance policy, encode the exact landing call and
   demonstrate receipt plus expected contract state progression. Operational
   execution remains separately scoped; no transaction is authorized by this doc.
4. **Reusable child implementation:** after statement/trust review, implement
   block/range certificates and final consumption, covering altered child
   statements/signatures, unauthorized signers, joins, omitted anchors and Q
   changes. Example acceptance: four 25-block ranges -> one 100-block landing.

Steps 1 and design review can proceed before the user chooses the direct-versus-
child-first milestone. API implementation follows anchor connection and design
confirmation. No calendar promise substitutes for native/SGX/L1 evidence.
