# Raiko2 Readiness Roadmap

> Historical roadmap. It predates the current canonical route/config naming and may describe
> superseded concepts. Use `README.md`, `docs/API.md`, and `config.example.toml` as the current
> source of truth.

Date: 2026-02-03
Status: Draft

## Executive Summary
This document turns the "Raiko2 Readiness" task list into an execution plan based on the current
codebase state. The primary blocker is implementing Shasta **proposal derivation** end-to-end
using **L1 proposal id** (not L2 block number). Once derivation is correct and tested, we can
complete system lifecycle testing, production deployment (single-process), and Golden Touch signing
plus an on-chain verifier for **all** proof backends.

## Key Decisions (Locked)
- **Request identifier:** `proposal_id` in the API refers to **L1 proposal id**.
- **Golden Touch:** sign **all** proofs (native, risc0, sp1), and verify on-chain.
- **Deployment shape:** single-process (HTTP API + workers in one `raiko2` process).

## Current Implementation Snapshot (As-Is)
### What exists
- **Engine/queue orchestration:** queue-driven pipeline stages (preflight → validate → encode → prove)
  with workers managed in-process.
- **Backends:** `risc0`, `sp1`, and `native` are supported via config and wiring.
- **Stateless block verification:** implemented and unit-tested.
- **Proof data model:** `ProofEnvelope` + `AggregationInput` exist in primitives.
- **E2E tests:** in-process e2e exists for native proof using repo fixture input.

### Known gaps / mismatches
- **Proposal derivation is not implemented.**
  - Shasta preflight currently treats `proposal_id` as a *block number*:
    `crates/pipeline/src/forks/shasta/spec.rs:66`.
  - Manifest building still depended on request config injection:
    `crates/pipeline/src/forks/shasta/manifest.rs:234`.
- **Blob verification is not wired into the pipeline validation.**
  - Helper exists (`crates/primitives-shasta/src/blob.rs:28`), but must be invoked as part of the
    validation stage to match readiness expectations.
- **Root Dockerfile is not aligned with the repo layout** (appears to reference legacy paths) and
  should be replaced by a workspace-aligned build for `bin/raiko2` plus guest ELFs.

## Readiness Checklist → Planned Milestones
| Task List Item | Target Milestone | Notes |
| --- | --- | --- |
| Find a better name | M5 | Branding decision; can be deferred until public release. |
| SGX proof generation backend binary compilation | M6 | Not in critical path for readiness. |
| Golden Touch native proof generation as default | M3 | Also requires signing all proofs + on-chain verifier. |
| Test: block verification | Done | Implemented in stateless validation. |
| Test: blob verification | M1 | Wire into pipeline validation; add deterministic tests. |
| Test: proposal derivation verification | M1 | Core blocker: L1 event → payload → manifest → blocks. |
| Test: task lifecycle (system-wise) | M2 | Cancel/retry/timeout/aggregation; memory + redis. |
| open source (maybe together with taiko-mono) | M5 | Dependency + documentation audit. |
| TDX backend?? | M6 | After proof envelope + signer/verifier are stable. |
| Docker file (backend binaries, gadgets) | M4 | Replace root Dockerfile with workspace-aligned build. |
| K8s script | M4 | Single-process deployment; scale via replicas. |
| Infra design | M2/M4 | Single process locked; add persistent stores and SLOs. |
| backend DB? | M2 | Add persistent metadata + payload store plan. |

## Roadmap

### M0 — Spec Lock & Test Baseline (1–2 days)
**Goal:** lock semantics and define deterministic test fixtures for derivation and lifecycle.

**Deliverables**
- Update API documentation to state `proposal_id` is L1 proposal id.
- Define a “derivation fixture format” (captured L1 event + blob payload(s) + expected derived L2
  block numbers + expected manifest hashes).
- Document Golden Touch signature payload versioning strategy.

**Acceptance Criteria**
- Spec reviewed and agreed (inputs, outputs, invariants, versioning).
- Minimal fixture can be used without external infra.

---

### M1 — Proposal Derivation Verification (Core Blocker) (1–2 weeks)
**Goal:** derive the proving inputs from L1 proposal id, validate correctness, and remove config
injection as a production path.

**Scope**
- Implement “L1 proposal → derivation sources → payload retrieval → manifest decode → L2 blocks list”.
- Update preflight to fetch the correct L2 block(s)/range based on derived data, not `proposal_id`.
- Wire blob verification into validation as required.

**Implementation Notes / Touchpoints**
- Replace `block_numbers = vec![ctx.request.proposal_id]` with derived block numbers:
  `crates/pipeline/src/forks/shasta/spec.rs:66`.
- Replace config-injected proposal event/payloads as the default production path:
  `crates/pipeline/src/forks/shasta/manifest.rs:56` and `crates/pipeline/src/forks/shasta/manifest.rs:234`.
- Ensure the derived `InputDataSource` matches Shasta `DerivationSource` semantics and blob slicing.

**Tests**
- Unit tests for:
  - manifest decode + `data_sources` derivation from `sources[]`.
  - blob slice extraction and boundary conditions (empty sources, multi-source).
- Integration tests for:
  - `proposal_id (L1)` → derived manifest + derived L2 blocks list (fixture-based).
  - “wrong blob” / “wrong event” negative tests.

**Acceptance Criteria**
- For a pinned fixture, derivation deterministically produces:
  - a `TaikoManifest` with correct `proposal_event`, `data_sources`, and chain spec
  - a correct list of L2 block numbers
- Pipeline validation fails fast on:
  - invalid/missing blobs, mismatched blob hashes, invalid offsets, or manifest decode failure
- No production flows rely on “request config injection” for proposal events/payloads.

---

### M2 — System-Wise Task Lifecycle & Persistence Plan (1 week)
**Goal:** prove the system behaves correctly under real lifecycle conditions and define storage
policy to handle >1000 concurrent requests safely.

**Scope**
- Expand lifecycle coverage: submit → run → status polling → cancel → retry/backoff → timeout.
- Decide and document persistence strategy:
  - Redis for queue leasing + task state
  - durable storage for proof payloads (object store) and metadata (DB) if payload sizes exceed
    Redis comfort.

**Acceptance Criteria**
- E2E tests cover:
  - successful completion for `native` and at least one zk backend
  - cancellation behavior at each stage
  - retry behavior on transient failure
- Clear storage policy is documented (payload size thresholds, retention, indexing).

---

### M3 — Golden Touch Signing (All Proofs) + On-Chain Verifier (1–2 weeks)
**Goal:** every completed proof is signed and verifiable on-chain using a stable message schema.

**Scope**
- Define `GoldenTouchMessageV1` (versioned, canonical encoding).
  - Must bind: chain ids, L1 proposal id, pipeline key, backend, public inputs, verifier artifacts
    (e.g., image id / vkey hash), and proof payload hash.
- Implement signer integration in raiko2 (single place after proof completion).
- Implement and deploy Solidity verifier (in `taiko-mono`).

**Acceptance Criteria**
- For each prover backend (`native`, `risc0`, `sp1`):
  - raiko2 returns a Golden Touch signature alongside proof material
  - on-chain verifier accepts valid signatures and rejects invalid/mismatched payloads
- Default configuration for the server sets prover to `native`, but signing remains enforced for all.

---

### M4 — Production Deployment (Single Process) (1 week)
**Goal:** ship a reproducible container and K8s deployment for a single `raiko2` process that
handles both API and workers.

**Scope**
- Replace root Docker build with:
  - workspace build of `bin/raiko2`
  - packaging of guest ELFs from `crates/guests/elf/*`
  - minimal runtime image (CA certs, OpenSSL, etc.)
- Add K8s manifests:
  - Deployment/Service, HPA, resources, probes, config/secrets, PDB
- Observability baseline:
  - structured logs, request IDs, queue metrics, stage latency

**Acceptance Criteria**
- `docker build` produces an image that can run `raiko2` with a mounted config and serve `/ready`.
- K8s deployment runs as a single process and can scale replicas safely when Redis queue is enabled.

---

### M5 — Open Source Readiness + Naming (1 week, can overlap M3/M4)
**Goal:** make the project publishable with clear docs and safe defaults.

**Scope**
- Decide final name and update docs/paths accordingly.
- Audit dependencies and licensing; ensure no internal-only artifacts are required.
- Expand docs:
  - API (including Golden Touch signature fields)
  - local dev, docker/k8s, troubleshooting

**Acceptance Criteria**
- Clean OSS build path documented and validated in CI.
- Release checklist exists (versioning, changelog, tags, artifacts).

---

### M6 — SGX / TDX Backends (Future)
**Goal:** add TEE backends without rewriting proof representation or signing.

**Scope**
- SGX: implement compilation + proof format + integration (prefer agent-backed where possible).
- TDX: define attestation artifacts in `ProofEnvelope.verifier_artifacts` and implement verifier path.

**Acceptance Criteria**
- End-to-end proof generation + Golden Touch signing + on-chain verification works for the new backend.

## Risks & Mitigations
- **Protocol drift:** Shasta event/schema changes or blob slicing rules drift.
  - Mitigation: fixtures pinned to chain deployments + versioned message and manifest decode.
- **Payload size:** proofs/receipts may exceed comfortable Redis sizes.
  - Mitigation: external payload store with metadata pointers; compress + size limits.
- **Key management:** signer key handling must be production-safe.
  - Mitigation: dev uses env keys; prod uses KMS/HSM/remote signer; rotation supported.

## Execution Commands (When Implementing)
```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
```
