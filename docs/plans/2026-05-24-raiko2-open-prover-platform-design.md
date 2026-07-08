# Raiko2 Open Prover Platform Program Overview

## Status

Draft for discussion.

This document is the top-level navigation document for the open prover platform effort. It keeps
the shared architecture and workstream map in one place, then points to the first-level design
documents that should carry the detailed work.

## Goal

Make `raiko2` a platform where new proving systems can be integrated, benchmarked, and audited
against the same Shasta execution statement without each new provider having to reimplement:

- server routing
- provider identity handling
- proof metadata validation
- aggregation admission rules
- benchmark plumbing
- one-off security review logic

Known proving systems today include:

- `sp1`
- `risc0`
- `risc0` via Boundless
- `raiko2-sgx`
- `gaiko2-sgx`

## Non-Goals

- Do not introduce dynamic shared-library plugins in the first iteration.
- Do not replace the current Shasta guest input, preflight, or stateless validation model.
- Do not weaken existing aggregation identity checks for SGX or same-provider aggregate proofs.
- Do not treat `native` as a production verifier. It remains a local execution and mock-proof lane.
- Do not design a public marketplace, billing layer, or external provider discovery protocol yet.

## Current Foundation

The reusable internal pieces are already in place:

- `PipelineSpec` and `Pipeline` build and validate hardfork-specific guest inputs.
- `Prover<B>` normalizes `encode`, `prove_encoded`, and `aggregate`.
- `Engine<S>` schedules proposal and aggregation tasks for any `PipelineSpec + Prover`.
- Shasta guest code already proves the execution statement that matters:
  - derive proposal transactions
  - reconstruct the derived block
  - execute through the witness-backed path
  - compare the filtered block against the canonical block
- `alethia-reth` owns Taiko execution/filter/assembly semantics through the block-based prover
  helpers used by `raiko2-stateless`.

That means the core proving abstraction is already reusable. The remaining platform problem is at
the edges:

- route and provider identity
- proof metadata and aggregation contracts
- cross-provider benchmarking
- explicit security/conformance gates

## Chosen Direction

Use a static provider registry as the first platform iteration.

The top-level model should separate four concerns:

1. `ForkSpec`: Shasta input/preflight/validation semantics.
2. `ProviderDescriptor`: proving system identity, capabilities, and factories.
3. `ProofEnvelope`: versioned internal proof contract and verifier artifact shape.
4. `BenchmarkCase`: deterministic workload definition shared across providers.

This keeps the build and trust model simple while still removing hard-coded routing from the host.

## Key Decisions

### Provider Identity

Provider identity should become explicit and stable:

- `sp1.local`
- `sp1.network`
- `risc0.local`
- `risc0.boundless`
- `sgx.raiko2`
- `sgx.gaiko2`
- `native.local`

The current `PipelineKey` and route aliases can remain as compatibility shims during migration, but
new routing and validation logic should operate on provider ids, not on open-coded enum matches.

### Public API Stability

The first platform iteration should not rewrite the public API surface.

Recommended boundary:

- `provider_id` becomes the internal first-class identity.
- external requests continue to use today's `proof_type` and fork/network fields initially.
- server/config resolve `proof_type -> provider_id`.
- `ProofEnvelope` becomes the internal contract first.
- legacy `Proof` remains the public response shape until clients are ready to migrate.

This keeps platform work decoupled from an immediate API rewrite.

### Benchmarking And Security Are Separate Workstreams

These tracks should not block each other:

- benchmarking answers "how comparable and how fast"
- security invariants and mutation tests answer "what is acceptable"

Provider intake should first guarantee safe integration and conformance. Comparative benchmarking
can mature in parallel or immediately afterward.

## Workstream Map

The approved direction should now be split into three first-level design documents.

### 1. Provider Registry And Proof Envelope

Document:
- [2026-05-25-raiko2-provider-registry-and-proof-envelope-design.md](2026-05-25-raiko2-provider-registry-and-proof-envelope-design.md)

This workstream owns:

- provider ids and descriptor shape
- request/config resolution into provider ids
- server and engine registration through descriptors
- internal `ProofEnvelope` and aggregation admission ownership
- migration from `PipelineKey`-centric routing

Likely next-level split:

1. request selection and config mapping
2. provider descriptor and factory wiring
3. proof envelope and legacy `Proof` adapter
4. aggregation admission and provider-owned validation

### 2. Unified Benchmark Framework

Document:
- [2026-05-25-raiko2-benchmark-framework-design.md](2026-05-25-raiko2-benchmark-framework-design.md)

This workstream owns:

- benchmark levels and case taxonomy
- fixture and manifest shape
- benchmark report format
- local, remote, zkVM, and TEE comparability

Likely next-level split:

1. benchmark case schema and fixture layout
2. case generation and checked-in workload families
3. benchmark runner/reporting integration
4. public comparison suite and release reporting

### 3. Security Invariants And Mutation Suite

Document:
- [2026-05-25-raiko2-security-invariants-and-mutation-suite-design.md](2026-05-25-raiko2-security-invariants-and-mutation-suite-design.md)

This workstream owns:

- the Shasta proving statement and invariant matrix
- mutation-based negative testing
- provider conformance gates
- witness coverage reporting
- TEE-specific identity and attestation requirements

Likely next-level split:

1. invariant matrix and production acceptance rules
2. mutation harness and fixture mutation library
3. provider conformance command and CI gates
4. TEE runtime identity and quote-verification boundary

### Near-Term Starting Slice: Fixture Envelope And Public Input Check

Document:
- 2026-05-25-raiko2-fixture-envelope-and-public-input-check-design.md (not checked in yet)

This is not a separate top-level workstream. It is the first concrete slice that bridges:

- provider registry and proof-envelope work
- security invariant and conformance work

Its first implementation should stay intentionally narrow:

- proposal fixtures only
- local opening of `ProofCarryData`
- provider public-input conformance
- optional full proof verification
- aggregate fixtures deferred until the proposal contract is stable

## Recommended Execution Order

The order should be:

1. fixture envelope and public-input check slice
2. provider registry and proof envelope
3. security invariants and mutation suite
4. benchmark framework

Reasoning:

- the fixture slice gives the platform a concrete conformance unit quickly
- provider registry is still the architectural foundation
- security/conformance should gate new providers before the platform expands
- benchmarking becomes more valuable once providers share a stable intake contract

## Deferred Topics

These are intentionally out of scope for the first pass:

- dynamic plugin ABI
- external provider marketplace or discovery protocol
- billing, quotas, or scheduling economics
- immediate public API replacement with `ProofEnvelope`
- non-Shasta fork generalization beyond the existing `ForkSpec` abstraction

## Immediate Next Step

If the overall direction is accepted, review and refine the fixture-envelope slice first, then use
it to drive narrower implementation plans. After that, continue refining the provider-registry,
security, and benchmark design documents into smaller follow-up plans.
