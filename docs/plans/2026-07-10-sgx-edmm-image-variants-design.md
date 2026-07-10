# SGX EDMM Image Variants Design

## Goal

Produce both non-EDMM and EDMM variants of the locally built `raiko2-sgx` image in one TEE
provider release. Some SGX hosts cannot launch EDMM enclaves, while EDMM-capable hosts should be
able to select that capability explicitly.

## Release Contract

- `<release>` is the non-EDMM image and remains the compatibility tag.
- `<release>-edmm` is the EDMM-enabled image.
- Both images use the same source revision and Gramine signing key.
- Each image has its own rendered and signed Gramine manifest, image digest, and MRENCLAVE.
- External TEE provider builds and tags are unchanged.

The unsuffixed tag intentionally means non-EDMM. Operators must opt into EDMM through the explicit
tag so images scheduled on older SGX hosts do not fail at enclave startup.

## Build Flow

`Dockerfile.sgx` accepts a boolean `SGX_EDMM_ENABLE` build argument that defaults to `false`. The
argument is passed to `gramine-manifest`, which renders `sgx.edmm_enable` before
`gramine-sgx-sign` signs the manifest. The build fails before signing unless the value is exactly
`true` or `false`.

`release-tee-providers` builds the local provider twice:

1. Build and optionally push `<release>` with `SGX_EDMM_ENABLE=false`.
2. Build and optionally push `<release>-edmm` with `SGX_EDMM_ENABLE=true`.
3. Read attestation metadata independently from each built image.
4. Emit both local variants in the release TEE attestation manifest.

The local Compose build exposes the same argument and defaults it to `false`.

## Manifest Representation

The existing non-EDMM entry keeps provider name `raiko2-sgx`, lane `sgx`, and the unsuffixed tag.
The EDMM entry uses provider name `raiko2-sgx-edmm`, lane `sgx`, and the suffixed tag. Local image
entries include a machine-readable `sgx_edmm` boolean. External provider entries omit the field
when their EDMM capability is not managed by this repository.

This keeps existing consumers compatible while making image selection and verifier registration
unambiguous.

## Failure Handling

- Reject values other than literal `true` or `false` before manifest generation and signing.
- Do not publish the handoff manifest if either local variant fails to build, push, resolve its
  digest, or expose valid attestation metadata.
- Preserve the existing clean-source-tree and signing-key checks.
- Never reuse one variant's digest or attestation metadata for the other variant.

## Verification

Unit tests cover:

- unsuffixed and `-edmm` tag derivation;
- Docker build commands forwarding the correct EDMM value;
- two local release manifest entries with distinct provider names and `sgx_edmm` values;
- unchanged external provider behavior;
- stable JSON serialization of the optional `sgx_edmm` field.

Repository checks cover formatting, focused xtask tests, Clippy, Compose config rendering, and a
static check that the Gramine template receives the build argument before signing.
