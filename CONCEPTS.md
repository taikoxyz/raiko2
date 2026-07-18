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

### Global Runtime Fence
The process-wide shutdown authority for every task and external-store mutation in one namespace. It
allows writes while active and rejects ordinary writes as soon as draining starts. A remote request
accepted before that transition carries the sole capability that may persist its request-ID
checkpoint during the bounded drain. It is not a task-local lock or multi-instance coordination
protocol.

### Lifecycle Operation Gate
The process-local serialization boundary for a lifecycle operation that spans the authoritative
runtime, scheduler queue, and proof artifact store. Admission, replacement, cancellation, cleanup,
invalidation, and proof completion enter this gate before committing a cross-component change.
Snapshots taken before entry are revalidated with the task incarnation or artifact descriptor.
This gate prevents local ABA races; it is not a distributed lock or namespace ownership mechanism.

### GCS Generation
The native object-version token used for runtime-state compare-and-swap and exact manifest
invalidation or deletion. It versions one object and is not an instance epoch or ownership token.

### Task Incarnation
The immutable UUID of one runtime task-record lifetime. It prevents a delayed worker from attaching
a checkpoint to a replacement that reuses the same deterministic task ID. Pending proof outboxes
and completion permits bind to exact task incarnations until artifact activation commits. It does
not coordinate instances or grant namespace ownership.

### Stage Lease
The in-memory scheduler ownership of one executable stage. It remains held through provider
execution, proof checkpointing, durable artifact publication, and terminal runtime synchronization.
Every acquired lease has an opaque, non-reused lease token, so an old worker cannot complete a task
that was removed and recreated with the same deterministic ID, worker label, and attempt number.
The token is local execution identity, not runtime authority. Losing the lease while the process is
running fails the stage.

### Task Execution Permit
An in-process capability captured from the runtime immediately after a queue lease is acquired. It
maps each existing runtime task ID to the exact incarnation that the lease may observe or mutate. The
engine revalidates the queue lease token after capture, and every observer write checks the permit.
After the completed proof payload is conditionally checkpointed under that lease, a distinct shared
root that joined during execution may enter the proof completion owner set; a replacement using an
already captured task ID may not. The permit is a stale-callback guard, not namespace authority; the
global runtime lifecycle fence still decides whether any write is allowed.

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

### Runtime Store
The single authoritative backend for task state, remote checkpoints, publication state, manifests,
and proof artifacts. A deployment selects GCS or ephemeral memory; dual-write and automatic
failover are unsupported.

### Proof URI
The backend-neutral location of a published proof artifact exposed as `proof_uri`. GCS stores use
`gs://`; memory mode uses `memory://`. `proof_path` is not part of the API.
