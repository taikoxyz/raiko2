# Raiko2 Provider Registry And Proof Envelope Design

## Status

Draft for discussion.

This document narrows the open prover platform work to provider identity, routing, and proof
contract ownership. It should be the first implementation-facing workstream because the other
platform pieces depend on it.

## Goal

Replace hard-coded prover routing and aggregation admission with a static provider registry and a
provider-owned proof contract, without rewriting the public API in the first iteration.

## Non-Goals

- Do not introduce runtime-loaded plugins.
- Do not change the Shasta preflight or guest input semantics.
- Do not replace the public `Proof` response shape immediately.
- Do not require all providers to support aggregation.

## Current Problems

### Provider Identity Is Not First-Class

`PipelineKey`, `GuestSystem`, and `RunnerKind` currently encode a small fixed set of lanes:

- `shasta-risc0-local`
- `shasta-risc0-network`
- `shasta-sp1-local`
- `shasta-native-local`
- `shasta-sgx-remote`
- `shasta-sgxgeth-remote`

This mixes several dimensions:

- hardfork
- proof system
- runner class
- concrete implementation

Every new provider currently requires enum growth and repeated match updates across routing, config,
server state, task metadata, and tests.

### Proof Metadata Is Backend-Specific But Not Owned By Backends

The public `Proof` shape is a loose collection of optional fields:

- `proof`
- `input`
- `quote`
- `uuid`
- `kzg_proof`
- `extra_data`

Aggregation admission then infers meaning from different field combinations based on `PipelineKey`.
That scales poorly as providers diversify.

`ProofEnvelope` and `AggregationInput` already exist in primitives, but they are not the primary
platform contract yet.

## Design Direction

Use a static provider registry compiled into the server binary.

Each provider descriptor should declare:

- stable provider id
- supported fork and stages
- runner class
- prover and backend factories
- proof envelope schema and verifier artifact rules
- aggregation support
- benchmark support metadata

This preserves the simple build and security model while removing server hard-coding.

## Provider Ids

Provider ids should be explicit and stable:

- `sp1.local`
- `sp1.network`
- `risc0.local`
- `risc0.boundless`
- `sgx.raiko2`
- `sgx.gaiko2`
- `native.local`

This is more precise than route names like `sgx/remote`, which hide implementation and trust model.

## Descriptor Shape

The exact Rust API can evolve, but the logical shape should be:

```rust
pub struct ProverProviderDescriptor {
    pub id: &'static str,
    pub fork: ForkId,
    pub runner: RunnerClass,
    pub stages: &'static [ProofStage],
    pub capabilities: ProviderCapabilities,
    pub proof_contract: ProofContract,
    pub benchmark_profile: Option<BenchmarkProfile>,
}
```

The descriptor should answer:

- can this provider produce proposal proofs
- can this provider produce aggregation proofs
- can it verify child proofs locally
- does it need guest ELF bytes or remote URLs
- which verifier artifacts must be emitted
- which public input shape is committed by the guest or trusted runtime

## Request And Config Resolution

The first iteration should keep the public API stable.

Recommended boundary:

- external requests continue to carry today's `proof_type` plus fork/network fields
- server/config resolve those into a concrete `provider_id`
- `provider_id` becomes the internal first-class identity
- the current `PipelineKey` values remain as compatibility aliases until migration is complete

This avoids turning the platform effort into an immediate API rewrite.

## Registration Flow

Server startup should become:

1. load config
2. resolve network pairs
3. load provider registry
4. for each configured provider and network pair, build provider engine
5. expose routes based on registered provider capabilities

This removes the need for `AppState::new` to know every proving system by name.

## Proof Envelope Contract

`ProofEnvelope` should become the internal provider contract. The public `Proof` struct should stay
as a legacy response shape until clients migrate.

Example logical envelope:

```json
{
  "schema": "raiko2-proof-envelope-v1",
  "provider_id": "sp1.local",
  "fork": "shasta",
  "stage": "proposal",
  "public_inputs": {
    "shasta_subproof_input": "0x...",
    "shasta_aggregation_input": null
  },
  "payload": {
    "kind": "sp1_proof",
    "bytes": "0x..."
  },
  "verifier_artifacts": [
    {
      "kind": "sp1_vk_hash_bytes",
      "value": "0x..."
    }
  ],
  "carry_data": {
    "shasta": {
      "proof_carry_data": {}
    }
  },
  "metadata": {}
}
```

Important rules:

- `provider_id` must be part of the envelope
- public inputs should be named by semantic meaning, not backend-specific field names
- guest digest, image id, verifying key, signer id, or enclave measurement belong in
  `verifier_artifacts`
- Shasta carry data remains fork-owned
- aggregation admission should validate envelopes through the provider descriptor, not through a
  global `PipelineKey` match

## Migration Strategy

### Phase 1: Descriptor Skeleton

- add provider id naming
- introduce provider descriptors for existing providers
- keep current `PipelineKey` values as compatibility aliases

### Phase 2: Registration Refactor

- route server registration through descriptors
- move aggregation admission rules into provider-owned validators
- stop adding new hard-coded lane matches

### Phase 3: Envelope Adoption

- make providers produce internal `ProofEnvelope`
- keep legacy `Proof` as an adapter/output view
- preserve public API compatibility until clients migrate

## Likely Next-Level Split

This document should later split into narrower designs or implementation plans:

1. request selection and config mapping
2. provider descriptor and engine factory wiring
3. `ProofEnvelope` schema and legacy `Proof` adapter
4. aggregation admission ownership and validation flow

## Open Questions

- should provider ids be configured globally per server or selected per request
- should `sp1.network` be a separate provider id from `sp1.local`, or a runner mode under `sp1`
- how much legacy `PipelineKey` compatibility should survive after the registry lands
- should a future public API expose `provider_id`, or remain `proof_type`-centric indefinitely
