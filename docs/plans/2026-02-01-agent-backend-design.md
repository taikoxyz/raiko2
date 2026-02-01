# Agent Backend Integration for Raiko2

Date: 2026-02-01

## Summary
Introduce an agent-backed prover implementation in raiko2 that delegates proof generation to `raiko-agent` while keeping raiko2 responsible for pipeline preflight/validation, guest input encoding, and ELF selection. The integration supports proposal and aggregation proofs across backends. To reduce long-term ambiguity and support aggregation consistently, refine the proof data model into a backend-agnostic `ProofEnvelope` and define a canonical `AggregationInput` that each backend adapter can transform into its zkVM-specific guest input.

## Goals
- Add an `agent` prover backend that submits proof requests to raiko-agent and polls for results.
- Support proposal + aggregation flows, starting with RISC0 (agent’s current boundless backend), and future SP1/TEE/TDX backends without reworking core types.
- Define a stable, backend-agnostic proof representation to enable cross-backend aggregation and reduce schema ambiguity.
- Keep raiko2 free of proof-generation SDKs where possible; agent owns SDK integration.

## Non-goals
- Implementing SGX/TDX agent backends in this iteration (only defining the data model to support them).
- Overhauling the pipeline/engine scheduling model.
- Changing guest program logic in this phase (beyond input adapters).

## Current Architecture (Host/Guest Boundary)
- Host builds `GuestInput` via pipeline `Preflight` + `Validation`.
- Host encodes `GuestInput` via `Prover::encode`, which may be backend-specific (e.g., RISC0 uses explicit bincode; SP1 can use its native input path). ELF selection remains via `ProverBackend`.
- Prover adapter runs zkVM locally (RISC0/SP1) and returns `Proof` with opaque fields.
- Aggregation uses backend-specific assumptions (RISC0 receipts; SP1 proofs + VKs).

## Agent API (Current)
- `POST /upload-image/{prover_type}/{batch|aggregation}`: upload ELF bytes (agent computes image ID).
- `POST /proof`: submit async proof request
  - fields: `prover_type`, `input` (bytes), `output` (bytes), `proof_type`, optional `elf` (Update), optional `config`.
- `GET /status/{request_id}`: poll; returns `proof_data` bytes on completion.
- `GET /images`: inspect cached images.

Agent already supports RISC0 aggregation by accepting a `BoundlessAggregationGuestInput` bincode payload (includes receipts). No API extension is needed for RISC0 aggregation.

## Refined Proof Model
Replace the current overloaded `Proof` representation with an explicit, backend-agnostic envelope. Suggested new types in `crates/primitives`:

- `ProofEnvelope`
  - `backend: ProofBackend` (risc0/sp1/sgx/tdx/etc)
  - `public_inputs: PublicInputs` (e.g., input hash / instance hash)
  - `payload: ProofPayload` (opaque bytes + `payload_kind` describing the payload within the same backend, e.g., `risc0_journal`, `sp1_proof_bincode`, `tee_quote`)
  - `verifier_artifacts: Vec<VerifierArtifact>` (typed items like receipt JSON, vkey hash, image ID)
  - `carry_data: Option<ProofCarryData>` (serialize/deserialize via a versioned schema)
  - `metadata: Option<serde_json::Value>`

This model keeps proof material explicit while staying backend-agnostic. It also carries `ProofCarryData` to preserve previous behavior and aggregation dependencies.

### Aggregation Input
Define a canonical `AggregationInput`:
- `proofs: Vec<ProofEnvelope>`
- `expected_image_id: Option<String>` (or backend-specific ID)
- `metadata: Option<serde_json::Value>`

Each backend provides an adapter that translates `AggregationInput` into the zkVM-specific guest input:
- RISC0: extract receipt JSON from `verifier_artifacts` and build `BoundlessAggregationGuestInput`.
- SP1: extract `SP1ProofWithPublicValues` + verifying key artifacts.
- TEE/TDX: use `payload` and `verifier_artifacts` as required.

## Agent Prover Integration (raiko2)
### High-level flow
1. Ensure agent has correct ELF for proposal/aggregation (upload on change or cache miss).
2. For proposal:
   - Use `encode` output as `input`.
   - Submit `POST /proof` with `proof_type = batch` (agent uses boundless terminology).
   - Poll `GET /status/{request_id}` until proof completes.
   - Decode agent’s `proof_data` into `ProofEnvelope` (initially RISC0: receipt + journal + image ID).
3. For aggregation:
   - Build canonical `AggregationInput` from prior proofs.
   - Convert to RISC0 aggregation guest input (receipts) and bincode encode.
   - Submit to agent with `proof_type = aggregate`.
   - Poll status; decode proof into `ProofEnvelope`.

### File/Module Changes (raiko2)
**New or changed modules**
- `crates/primitives/src/proof.rs`
  - Add `ProofEnvelope`, `ProofPayload`, `VerifierArtifact`, `AggregationInput`.
  - Keep backward compatibility with existing `Proof` if needed (or provide conversion).
- `crates/prover/src/lib.rs`
  - Add `agent` module implementing `Prover` trait.
  - Add helper for decoding `proof_data` into `ProofEnvelope`.
- `crates/prover/src/agent/mod.rs` (new)
  - HTTP client to agent, submit + poll, upload ELF, decode response.
- `crates/prover/src/agent/types.rs` (new)
  - Agent request/response schema and mapping.
- `crates/prover/src/agent/aggregation.rs` (new)
  - Convert canonical `AggregationInput` into backend-specific guest inputs.
- `bin/raiko2/src/config/prover.rs`
  - Add `agent` option with endpoint URL, timeout, API key, prover_type mapping.
- `bin/raiko2/src/server/state/mod.rs`
  - Wire `AgentProver` into pipeline factory and supported prover list.
- `docs/API.md` or `README.md`
  - Document new prover type and config for agent backend.

### Agent Interaction Details
- Upload ELFs when:
  - raiko2 detects ELF version change (via hash/image ID), or
  - agent returns an “image not uploaded” error on proof submission (upload then retry).
- For now, omit `Update` proof_type and rely on explicit `/upload-image`.
- Add configurable polling interval/backoff.
- Treat agent request IDs as external; map them into raiko2 tracking.

## Potential Issues / Risks
- **Proof decoding mismatch**: agent’s `proof_data` format must be decoded correctly into `ProofEnvelope`.
- **Aggregation format drift**: if agent or guest input schema changes, adapters must update.
- **Latency / retry**: long proof times require robust polling and timeout handling.
- **Versioning**: add version tags to `ProofEnvelope` and `AggregationInput` to avoid compatibility issues.

## Testing Strategy
- Unit tests for:
  - `ProofEnvelope` serialization/deserialization.
  - RISC0 aggregation adapter (builds expected `BoundlessAggregationGuestInput`).
- Integration tests (local):
  - raiko2 + agent with a small fixture input for proposal proof.
  - aggregation proof flow using stored receipts.

## Migration Plan
1. Introduce new proof models and adapters while preserving existing `Proof` responses.
2. Implement `AgentProver` and add config in `raiko2`.
3. Run integration tests against a local agent (boundless backend).
4. Gradually switch production to use `agent` prover type.
