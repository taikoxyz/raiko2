# Raiko2 Security Invariants And Mutation Suite Design

## Status

Draft for discussion.

This document narrows the open prover platform work to the proving statement, invariant framework,
mutation-based rejection tests, and provider conformance gates.

## Goal

Turn existing Shasta security reasoning into an explicit, checked-in contract that every production
provider must satisfy before being accepted into the platform.

## Non-Goals

- Do not redesign the Shasta guest input model.
- Do not collapse TEE and zkVM trust models into one artifact type.
- Do not treat benchmark performance as proof of correctness.

## Statement To Prove

For Shasta proposal proving, every accepted production provider should establish:

`proposal data -> data sources/blobs -> derived txlist -> derived block execution -> invalid tx filtering -> canonical L2 block/state transition -> Shasta transition input`

The public output must bind to the canonical `ProofCarryData` hash.

## Invariant Matrix

Create a checked-in matrix with one row per invariant and one column per provider:

- `sp1.local`
- `sp1.network`
- `risc0.local`
- `risc0.boundless`
- `sgx.raiko2`
- `sgx.gaiko2`
- `native.local`

Suggested invariant groups:

- chain spec and hardfork activation are validated
- proposal id, proposal hash, parent proposal hash, proposer, and timestamp are bound
- blob usage and proposal data-source rules are enforced
- derived transactions are reconstructed from proposal sources
- anchor transaction fields and checkpoint are validated
- candidate block is executed against witness-backed pre-state
- invalid non-anchor transactions are filtered by execution semantics
- filtered block equals the canonical block
- parent/child block linkage is contiguous
- L1 anchor linkage and origin header are validated
- post-state root and block hash are recomputed
- `ProofCarryData` is recomputed or fully checked inside the guest/trusted runtime
- public input equals the Shasta carry hash
- aggregation verifies child proof payloads and child public inputs
- guest digest, verifying key, signer id, or enclave measurement is bound to the proof

`native.local` should be explicitly marked non-production wherever it cannot satisfy proof or TEE
attestation requirements.

## Mutation Suite

Build a mutation framework around valid `GuestInput` fixtures.

Each mutation should alter exactly one semantic field and assert rejection:

- proposal id
- proposal hash
- parent proposal hash
- actual prover
- transition timestamp
- parent block hash
- checkpoint block number
- checkpoint state root
- block transaction bytes
- transaction order
- anchor recipient
- anchor checkpoint
- blob commitment
- blob proof
- data source count
- data source ordering
- witness state node bytes
- missing bytecode
- missing storage proof
- L1 origin hash
- L1 ancestor continuity
- proof carry hash

The acceptance gate for a new provider should include:

1. positive fixture passes
2. all required mutations reject
3. public input matches expected carry hash
4. provider envelope validates
5. aggregation either works or is explicitly marked unsupported

## Witness Coverage

The witness system should distinguish:

- missing witness data, which must fail
- malformed witness data, which must fail
- unused extra witness data, which may be allowed but should be measured and reported

Existing witness materialization analysis can become part of the conformance report:

- supplied state nodes
- materialized state nodes
- unused supplied nodes
- storage trie count
- storage node count

The later product decision is whether unused witness data remains a warning or becomes a production
rejection.

## TEE Provider Requirements

TEE providers differ from zkVM providers because proof validity depends on attestation and runtime
identity.

For `raiko2-sgx` and `gaiko2-sgx`, the provider contract should include:

- enclave measurement or equivalent runtime identity
- signer id
- quote payload
- quote verification policy
- reproducible build metadata
- proof signing key model
- PCCS or quote-verification assumptions

The platform boundary should stay narrow:

- `raiko2` defines which attestation and verifier artifacts it requires to accept a proof
- provider-specific quote verification details should not automatically be centralized into one
  global implementation path unless the trust model clearly demands it

This prevents the platform layer from accreting per-provider TEE special cases too early.

## Conformance Philosophy

Benchmarking and conformance are related but separate.

This workstream defines:

- what every accepted provider must prove or bind
- which negative cases must reject
- which verifier artifacts must accompany a proof

It does not define:

- which provider is fastest
- which provider is cheapest
- which provider has the best ergonomics

## Migration Strategy

### Phase 1: Invariant Matrix Draft

- turn current audit reasoning into a checked-in invariant matrix
- mark production versus non-production expectations per provider

### Phase 2: Mutation Harness

- add valid fixture corpus
- add one-field mutation helpers
- assert provider rejection behavior

### Phase 3: Provider Conformance Gate

- add provider conformance command
- validate public input, carry hash, and aggregation support
- attach conformance to intake requirements for new providers

## Likely Next-Level Split

This document should later split into narrower designs or implementation plans:

1. invariant matrix and production acceptance rules
2. mutation harness and fixture mutation library
3. provider conformance command and CI gates
4. TEE runtime identity and quote-verification boundary

## Open Questions

- should unused witness nodes be a warning, a benchmark metric, or a hard production error
- how much TEE quote verification should happen in `raiko2` versus provider-specific services
- do we want one conformance command that can exercise both local and remote providers
- should aggregation conformance be mandatory for every provider, or only for providers that claim
  aggregation support
