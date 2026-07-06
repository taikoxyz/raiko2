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
