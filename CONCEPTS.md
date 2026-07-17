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
The immutable ownership boundary for one raiko2 instance. A GCS-backed namespace has one renewable
owner lease, and both environment and namespace participate in task fingerprints and object names.

### Stage Lease
The in-memory scheduler ownership of one executable stage. It remains held through provider
execution, proof checkpointing, durable artifact publication, and terminal runtime synchronization.
Losing it while the process is running fails the stage.

### Pending Network Proof
A Boundless or SP1 provider request that has been durably checkpointed but has not reached a final
proof outcome. Restart recovery resumes the recorded provider request and its attempt budget instead
of submitting a duplicate request.

### Proof Manifest
A create-only pointer from a logical proof reference to an immutable content-hash object. Identical
publication is idempotent; different bytes cannot replace the selected proof.
