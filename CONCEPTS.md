# Concepts

Shared domain vocabulary for this project — entities, named processes, and status concepts with project-specific meaning. Seeded with core domain vocabulary, then accretes as ce-compound and ce-compound-refresh process learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Build And Release

### Runtime Image
A deployable container image that packages the raiko2 server binary, runtime configuration defaults, and the guest artifacts the host process needs to serve proof requests.

### Release Image Build
The project-controlled process that refreshes required guest artifacts, builds a runtime image, captures its digest, and optionally publishes it for deployment.

### Toolchain Image
A container image used to build zkVM guest programs with the guest target's Rust toolchain, native compilers, and build helpers; it is distinct from the runtime image that runs raiko2.

### Guest ELF
A compiled zkVM guest program artifact consumed by the host and prover paths. Guest ELFs are checked-in release artifacts, so changes to them are reviewed as compatibility-affecting output rather than incidental build products.

### Guest Refresh
The process of rebuilding and exporting guest artifacts when guest source, toolchain, configuration, or expected output state changes.

### Guest Fingerprint
A reproducible digest of the inputs that decide whether a guest refresh can skip rebuilding. It is a build-cache decision aid, not a proof digest or verifier identity.

## Runtime And Proof Lifecycle

### Environment
The business and deployment boundary, such as `devnet` or `mainnet`. It scopes request identity and
storage, but does not identify a concrete server instance.

### Runtime Namespace
The immutable persistence boundary for one raiko2 instance. Exactly one process may use a namespace
at a time, old and replacement processes never overlap, and namespaces never share data. This is a
deployment invariant; the application has no distributed owner lease or owner epoch. Both
environment and namespace participate in task fingerprints and object names.

### Request Fingerprint
The mandatory, non-empty identity of one normalized proof request inside a runtime namespace. It is
stored directly on every root and is unique within authoritative runtime state. Registration,
deduplication, replacement, and public task IDs use it; there is no anonymous root lifecycle.

### Namespace Fence
The process-wide mutation authority for one namespace. It admits short authoritative commits and
external writes while active, rejects new work as soon as draining starts, and rejects every write
when inactive. One permit spans each admitted repository write or proof-object operation, and drain
waits for those operations plus request-ID checkpoints covered by provider permits acquired while
active. A separate process-local lifecycle transition gate serializes only the active-root decision
with one in-memory queue attach or detach. Neither spans a complete task, provider call, or saga;
neither is a multi-instance coordination protocol.

### Proof Lifecycle
The concrete application service that handles submission, cancellation, cleanup, invalidation, and
restart reconciliation. It commits authoritative runtime state first, then applies owner-aware queue
and exact proof-object effects idempotently. It coordinates sagas; it is not a cross-component lock
and does not hide storage operations behind a single generic interface.

### Runtime State Repository
The sole authority for task records, remote checkpoints, publication intents, artifact
registrations, and lifecycle transitions in one namespace. It validates typed preconditions,
performs short atomic mutations, handles runtime-state compare-and-swap and ambiguous-write
readback, and returns explicit lifecycle outcomes. Its storage revision is internal.

### Proof Object Repository
The boundary for immutable pending and canonical proof bytes, create-only manifests, validated
reads, and exact conditional deletion. Durable `Invalidated` runtime records fence proof reuse while
the repository conditionally deletes the selected manifest generation. It owns all use of artifact
object generations and content hashes. GCS and memory are alternative adapters, not concurrent
backends.

### Execution Projection
The in-memory owner-aware task graph derived from authoritative runtime state. Attaching a root owner
and its complete DAG is atomic, as is detaching that owner. A shared stage remains while any live
root owns it; the last owner leaving cancels or removes the stage. Cancellation and terminal failure
persist their exact root transition before detaching the owner; terminal failure holds the local
lifecycle transition gate until its detach finishes. Restart reconciliation rebuilds the projection
instead of treating it as durable authority. Proposal nodes have root-independent definitions and no
proposal-to-proposal dependencies; only aggregation depends on the proposal artifacts it consumes.

### Root Owner
The exact `TaskLifetime` whose root currently requires an execution graph. Root owners are local
queue relationships, not namespace owners. They allow distinct roots to share a stage without one
root's cancellation deleting work still required by another. Ownership comes from canonical task
membership in runtime metadata; a broad artifact reference may instead identify a storage consumer
and does not grant execution ownership.

### GCS Generation
The native storage version of one GCS object. Runtime-state generations are hidden inside the state
repository's compare-and-swap loop. A manifest generation appears only in an exact artifact
descriptor used for invalidation or conditional deletion. It is not an instance epoch, task
lifetime, or ownership token.

### Task Lifetime
The tuple of a deterministic task ID and the immutable `incarnation_id` of one runtime record
lifetime. It prevents a delayed worker, cancellation, cleanup, or publication callback from mutating
a replacement that reused the same task ID. Publication intents bind to exact task lifetimes until
activation commits. It does not coordinate instances or grant namespace ownership.

### Stage Lease
The in-memory scheduler ownership of one executable stage. It remains held through provider
execution, proof checkpointing, durable artifact publication, and terminal runtime synchronization.
Every acquired lease has an opaque, non-reused lease token, so an old worker cannot complete a task
that was removed and recreated with the same deterministic ID, worker label, and attempt number.
The token is local execution identity, not runtime authority. Observer events also carry exact task
lifetimes; both the lease and lifetime must still match before a callback is accepted.

### Artifact Descriptor
The exact selected artifact version: logical key, content hash, and manifest object generation. It
identifies one publication for reads, activation, durable runtime invalidation, and conditional
manifest deletion.

### Artifact Expectation
A typed precondition containing an artifact key, expected descriptor, and expected lifecycle state.
It prevents a delayed read, cancellation, cleanup, or invalidation effect from acting on a newer
publication at the same logical key.

### Lifecycle Outcome
An explicit repository result such as `Applied`, `AlreadyApplied`, `Stale`,
`BlockedByLiveOwner`, `Missing`, or `Conflict`. Callers use the outcome to continue, stop, retry, or
reconcile instead of interpreting an ambiguous boolean.

### Pending Network Proof
A Boundless or SP1 provider request that has been durably checkpointed but has not reached a final
proof outcome. Restart recovery resumes the recorded provider request and its attempt budget instead
of submitting a duplicate request.

### Proof Manifest
A create-only pointer from a logical proof reference to an immutable content-hash object. Identical
publication is idempotent; different bytes cannot replace the selected proof.

### Proof Task Identity
The identity of proof work for one normalized request, concrete proof type, execution route,
environment, runtime namespace, and effective prover configuration.

### Proof Artifact Identity
The identity of one published proof for a concrete proof type, execution route, environment, and
proof request. Artifacts are never shared across concrete proof types, execution routes, or
environments. The first valid publication wins; a later conflicting artifact is discarded without
replacing the canonical artifact or regressing a completed task.

### Completed Proof Task
A proof task whose normalized final proof is durably published, registered, readable, and
synchronized to its runtime root. Successful proof computation alone does not complete the task.

### Proof Publication
The transition that makes a successfully computed proof durably available to callers. It is the
completion boundary of a proof task, not a separate public task status; the task remains
non-terminal until publication succeeds.

### Publication Intent
The authoritative runtime record created before pending proof materialization and canonical
publication. It binds the typed artifact identity and content hash to the exact task lifetimes
observed at checkpoint. Restart reconciliation uses it to resume a retained pending payload, activate
the canonical descriptor, or invalidate an ownerless result. Normal completion refreshes owners under
the local lifecycle gate before activation: a distinct root registered after the checkpoint may join,
but a replacement incarnation for an already observed task ID may not. If pending materialization
never completed, the root may recompute the proof without leaving an untracked object.

### Proof URI
The backend-neutral location of a published proof artifact exposed as `proof_uri`. GCS stores use
`gs://`; memory mode uses `memory://`. `proof_path` is not part of the API.

## Proving Trust Anchors

### Single-Anchor Invariant
The sole authenticity anchor for a proposal is the on-chain ring-buffer check: `Inbox.prove`
requires `commitment.lastProposalHash == getProposalHash(id)`, and `hashProposal` covers
`originBlockHash` and `originBlockNumber`. The guest deliberately performs no receipt or log proof
of the `Proposed` event; it proves consistency with the supplied event, not that the event was
emitted. Any future contract path that accepts a commitment without an exact ring-buffer match
would silently remove the only anchor and must add a second anchor in the guest (kimi-k3 audit
L-3 / assumption A4).

### Proof-Verifier Trust Roots
RISC0 binds proof acceptance to block and aggregation image IDs through `isImageTrusted`; SP1 uses
`isProgramTrusted` for the corresponding program verification keys. SGX instead trusts registered
instances under the current enclave policy. Stale or deprecated image IDs, program keys, instances,
or enclave policies must be revoked promptly, because their proofs remain acceptable while that
authorization stays active (audit assumption A1 and the L-7 image-lifecycle note).
