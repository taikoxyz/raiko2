# Raiko2 SGX Runtime Design

## Goal

Add a dedicated SGX runtime binary to `raiko2` that follows the historical `raiko` operator model
while staying consistent with `raiko2`'s architecture:

- `raiko2` main service remains the preflight/orchestration layer
- SGX proving runs in a separate remote service binary
- operators manage bootstrap, runtime startup, image build, and compose/env from this repository

The minimum supported operator surface is:

1. bootstrap
2. SGX sign/prove server
3. Docker image build
4. Docker compose environment

## Compatibility Target

The first compatibility target should match historical `raiko` SGX-facing proof types:

- `sgx`
- `sgxgeth`

This task does **not** implement both proving lanes inside `raiko2`.

Instead:

- `raiko2` repository implements the `sgx` runtime lane only
- `sgxgeth` remains an external runtime provided by the `gaiko2` repository

Operationally, the intended steady state is:

- `proof_type=sgx` routes to `raiko2-sgx-prover`
- `proof_type=sgxgeth` routes to an external `gaiko2` SGX server

So the compatibility goal is “fit into a `sgx | sgxgeth` world”, not “build both runtimes in this
repo”.

## Scope

### In scope

- A new repo-local binary, tentatively named `raiko2-sgx-prover`
- CLI subcommands: `serve`, `bootstrap`, `check`
- Gramine-backed `tee` mode plus a `native` operator/testing mode
- Shasta proposal proof serving
- Shasta aggregation proof serving
- SGX Docker image, compose file, env template, and operator docs
- explicit documentation that `sgxgeth` is owned by external `gaiko2` runtime infrastructure

### Out of scope

- Ontake/Hekla legacy `block` or `batch` modes
- Embedding bootstrap as an HTTP endpoint on the running server
- Non-Gramine SGX modes (`ego`, `dev`, `debug`)
- Implementing an `sgxgeth` runtime in this repository
- Modifying `crates/prover/src/gaiko2/*` in this task
- Changing the `raiko2` main service SGX remote integration in this task

## Key Constraint

Do not change the current `gaiko2` remote client path while another agent is working there.

Do not pull the `sgxgeth` proving lane into this repository. That lane belongs to the external
`gaiko2` service.

That leads to a split integration boundary:

- Proposal proving can be made directly compatible now by making `raiko2-sgx-prover` speak the
  existing `POST /prove/shasta` request/response contract already used by `raiko2`.
- Aggregation can be implemented in the SGX server now, but `raiko2` main service will not call it
  end-to-end until the separate remote-client work lands.
- `sgxgeth` compatibility is achieved by routing to an external server, not by expanding
  `raiko2-sgx-prover`.

So the server-side deliverable is “proposal ready and aggregation runtime ready”, not “full `raiko2`
SGX aggregation wired through the main service”.

## Decision

Use a dedicated binary inside this repo rather than embedding SGX runtime behavior into the main
`raiko2` server.

That binary represents the `sgx` lane only.

Recommended structure:

- `bin/raiko2-sgx-prover`
  - CLI entrypoint and subcommand wiring
  - server startup
  - bootstrap/check command handling
- `crates/sgx-runtime`
  - reusable SGX runtime logic
  - bootstrap file handling
  - proof/signing helpers
  - HTTP request/response adapters for the SGX server

This matches the old `raiko` operational model while preserving `raiko2`'s “remote prover service”
architecture.

## Binary Shape

The binary should expose explicit operator modes instead of putting lifecycle actions behind HTTP:

- `raiko2-sgx-prover bootstrap`
  - initializes the SGX runtime
  - generates or verifies local key material
  - emits bootstrap artifacts for later chain registration
- `raiko2-sgx-prover check`
  - verifies bootstrap artifacts, secrets, and runtime prerequisites are readable
- `raiko2-sgx-prover serve`
  - starts the HTTP proving server

This keeps the server runtime simple and makes bootstrap an explicit one-shot operator action, which
matches the behavior you asked for and is close to the older `raiko` flow.

The binary should also accept a runtime mode selector:

- `tee`: normal SGX/Gramine execution
- `native`: non-SGX operator/testing mode that keeps the same HTTP surface, treats `bootstrap` as
  a no-op, and uses the native signer identity

## Lane Ownership

The runtime split should be explicit in docs and operator assets:

- `raiko2-sgx-prover`: `proof_type=sgx`
- external `gaiko2` SGX server: `proof_type=sgxgeth`

This repo only owns the first line item.

## Server API

The server should stay Shasta-only.

It should also stay `sgx`-only.

### Required endpoints

- `POST /prove/shasta`
- `POST /prove/shasta-aggregate`
- `GET /health`

### Not included

- `/bootstrap`
- legacy `/prove/block`
- legacy `/prove/batch`

## Protocol Strategy

### Proposal prove

For proposal proving, the SGX server should reuse the existing `gaiko2` Shasta request/response
schema already present in `raiko2`:

- request: `raiko2_prover::gaiko2::protocol::Gaiko2ShastaRequest`
- response: `raiko2_prover::gaiko2::protocol::Gaiko2ProofResponse`

That gives us immediate protocol compatibility with the current `sgx/remote` client path without
editing the client.

### Shasta aggregation

Aggregation should be implemented in the SGX server as a first-class capability, but the transport
contract should live on the SGX runtime side for now rather than by modifying the current remote
client immediately.

Practical implication:

- the SGX server exposes `POST /prove/shasta-aggregate`
- the runtime can prove aggregation inputs and return a proper SGX proof envelope
- wiring the main `raiko2` service to call that endpoint remains a separate follow-up owned by the
  remote-client track

## Reused Code

The SGX runtime should reuse existing `raiko2` code wherever possible:

- Shasta proof/public-input hashing from `crates/protocol-shasta` and `crates/primitives-shasta`
- shared proof types from `crates/primitives`
- existing proposal packet types from `crates/prover::gaiko2::protocol` for proposal serving
- existing Shasta aggregation input types already used in `raiko2`

The runtime crate should avoid re-implementing Shasta hash logic copied from `../raiko` unless
there is a hard SGX-specific reason.

`sgxgeth` support should not be added by generalizing this runtime crate; that belongs to the
separate external service.

## Docker and Operations

The operator-facing deliverables should mirror the old `raiko` usability but with `raiko2` naming.

### Image

Add a dedicated SGX runtime image, likely via `Dockerfile.sgx`, based on a Gramine runtime image
with the required Intel SGX/DCAP packages and config wiring.

### Compose

Add a dedicated compose file, likely `docker/docker-compose.sgx.yml`, that:

- mounts SGX devices
- mounts PCCS/QCNL config
- mounts config, secrets, and bootstrap output directories
- supports running either `bootstrap` or `serve`

### Env

Add an operator template such as `docker/.env.sgx.sample` for:

- SGX config and secret paths
- server bind address/port
- PCCS settings
- image tag selection
- command mode selection if needed by entrypoint scripts

### Docs

Document the flow explicitly:

1. prepare SGX/PCCS host prerequisites
2. run bootstrap one-shot
3. inspect bootstrap output
4. perform chain registration out-of-band
5. start `serve`
6. point `raiko2` at the SGX runtime endpoint

The SGX operator docs should also call out that `sgxgeth` uses a different service managed outside
this repository.

## File Layout

Expected new or changed areas:

- `bin/raiko2-sgx-prover/`
- `crates/sgx-runtime/`
- `Dockerfile.sgx`
- `docker/docker-compose.sgx.yml`
- `docker/.env.sgx.sample`
- `docs/operations.md`
- optional dedicated SGX operator doc such as `docs/operations-sgx.md`

## Risks

### 1. Aggregation wiring split across tracks

The SGX runtime can support aggregation before the main `raiko2` remote client does. That is
acceptable, but the docs must call it out so nobody mistakes server readiness for full end-to-end
service readiness.

### 2. Bootstrap format drift

Bootstrap artifacts should stay close to the old `raiko` operator expectations; otherwise chain
registration scripts and operator muscle memory will break.

### 3. Over-importing old architecture

We should borrow the `raiko` operational model, not copy old repo-specific abstractions wholesale.
`raiko2` should keep one source of truth for Shasta hashing and proof data.

### 4. Confusing `sgx` and `sgxgeth` ownership

If the docs are vague, future work may accidentally try to turn `raiko2-sgx-prover` into a shared
runtime for both lanes. That is not the intended first version.

## Recommendation

Proceed with a `raiko2-sgx-prover` binary plus a small reusable `crates/sgx-runtime` crate,
implement proposal and aggregation runtime support on the `sgx` server side only, document
`sgxgeth` as an external `gaiko2` dependency, and leave main-service aggregation wiring to the
separate remote-client track.
