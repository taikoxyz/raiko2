# Raiko2 SGX Build Attestation Metadata Design

## Goal

Make the dedicated `raiko2-sgx` image emit and preserve its SGX measurement metadata at image
build time so operators can read the image identity directly from the built image without first
running `bootstrap`, `serve`, or a proving request.

The primary operator need is the enclave measurement used for chain registration:

- `mr_enclave`

The design should also preserve enough room to expose other SIGSTRUCT-derived fields when useful.

## Problem

The current `raiko2-sgx` image signs the Gramine manifest at container startup in
`docker/sgx-entrypoint.sh`.

That has two operational drawbacks:

1. SGX signing failures are discovered late, only when the container starts.
2. The resulting `mr_enclave` is not available as a stable image artifact, so operators cannot
   inspect the built image and immediately know which measurement to register.

This is weaker than the `gaiko2` operator surface, where deployment artifacts already include an
attestation metadata file and operators can read the image identity directly from release outputs.

## Decision

Move the `raiko2-sgx` signing flow from container startup to image build.

During `Dockerfile.sgx` build:

1. render the Gramine manifest
2. sign the enclave with the Gramine signing key
3. extract SIGSTRUCT fields with `gramine-sgx-sigstruct-view --output-format=json`
4. write a baked metadata file into the image

The image should expose a fixed metadata path:

- `/opt/raiko2-sgx/etc/attestation.raiko2.json`

This file becomes the canonical operator source for the SGX measurement of the `raiko2-sgx` image.

## Why Build-Time Instead Of Bootstrap-Time

`mr_enclave` is a property of the signed enclave image, not of bootstrap state or a running
instance.

Generating it at bootstrap time would still delay failure discovery and would not let operators
inspect a built image before initialization.

Build-time generation gives the desired behavior:

- signing fails during build if the image cannot be signed
- the built image always carries the measurement it was signed with
- operators can read the registration value without executing the service

## Build Key Handling

The build must have access to the Gramine enclave signing key.

This should not be copied into the image filesystem or passed through a normal build argument.
Instead, the image build should use a build secret for the enclave key.

Recommended source:

- host file: `${HOME}/.config/gramine/enclave-key.pem`

Recommended transport:

- Docker BuildKit secret mounted only for the signing `RUN` step

This keeps the key available at build time without baking it into image layers.

## Runtime Behavior After The Change

For `tee` mode:

- the image should already contain the rendered manifest, signed manifest, and SIGSTRUCT
- container startup should stop regenerating or re-signing these artifacts
- the runtime should only start the prover or run bootstrap/check commands

For `native` mode:

- behavior stays unchanged
- no SGX signing is required

This means the runtime no longer depends on a mounted Gramine signing key when running a prebuilt
`raiko2-sgx` image.

## Metadata Shape

The baked file should be minimal and registration-focused.

Required field:

- `mr_enclave`

Useful optional fields:

- `mr_signer`
- `isv_prod_id`
- `isv_svn`
- `debug_enclave`
- `date`

The format should be plain JSON to match existing operator expectations from nearby tooling.

Example:

```json
{
  "mr_enclave": "81a675e9a408818b430be4b259f3e11e6f8cacdb4c971c3114ee79fe53076893",
  "mr_signer": "0dedbe47afb6955e5f6109637c1fbd9cc4b4e073e1396da8ce2091075e5b0a3b",
  "isv_prod_id": 0,
  "isv_svn": 0,
  "debug_enclave": false
}
```

## Files To Change

### Docker Build Surface

- `Dockerfile.sgx`
- `docker/docker-compose.sgx.yml`
- `docker/docker-compose.sgx.regression.yml`

These changes are needed so builds can consume the Gramine signing key as a build secret and so
runtime containers no longer need the key mounted for normal startup.

### Runtime Entry Surface

- `docker/sgx-entrypoint.sh`

This file should stop doing runtime signing in the normal `tee` path and instead require the baked
manifest artifacts to already exist in the image.

### Metadata Helper

Add a dedicated helper script under `docker/` to convert a generated `.sig` into
`attestation.raiko2.json`.

This keeps Dockerfile logic readable and mirrors the existing `gaiko2` approach of writing a
machine-readable attestation metadata file.

### Docs

- `docs/development.md`
- `docs/operations.md`

Document:

- where to read `mr_enclave`
- that `xtask register-image` is for zk guest digests only
- that SGX registration uses the baked attestation metadata and external verifier tooling

## Non-Goals

- moving SGX registration logic into `raiko2 xtask`
- changing `gaiko2` registration flow
- adding quote parsing or on-chain registration calls into the `raiko2-sgx` server binary
- changing zk guest digest registration semantics

## Validation Strategy

The implementation should prove all of the following:

1. `Dockerfile.sgx` can build successfully when a signing key secret is provided.
2. The built image contains `/opt/raiko2-sgx/etc/attestation.raiko2.json`.
3. The metadata file contains a non-empty `mr_enclave`.
4. `raiko2-sgx` can still start in `tee` mode without runtime signing.
5. Existing local SGX regression compose flows still work after the build/runtime split.
