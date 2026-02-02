# Agent Backend Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an agent-backed prover in raiko2, plus a refined proof/aggregation data model that supports proposal + aggregation flows (starting with RISC0 via agent), with clear extension points for SP1/TEE/TDX.

**Architecture:** Introduce a backend-agnostic `ProofEnvelope` and canonical `AggregationInput` in `raiko2-primitives`. Implement an `AgentProver` in `raiko2-prover` that uploads ELFs, submits async requests to raiko-agent, polls status, and decodes responses into `ProofEnvelope`. Wire it into the HTTP server config and pipeline factory alongside existing RISC0/SP1/native backends.

**Tech Stack:** Rust 2024, `reqwest` (HTTP), `serde`, `bincode`, `tokio`, existing raiko2 crates.

---

## Task 1: Add refined proof model types

**Files:**
- Modify: `crates/primitives/src/proof.rs`
- Test: `crates/primitives/tests/proof_envelope_roundtrip.rs` (new)

**Step 1: Write the failing test**

Create `crates/primitives/tests/proof_envelope_roundtrip.rs`:

```rust
use raiko2_primitives::proof::{ProofEnvelope, ProofPayload, VerifierArtifact, PublicInputs};
use serde_json::json;

#[test]
fn proof_envelope_roundtrip() {
    let envelope = ProofEnvelope {
        backend: "risc0".to_string(),
        public_inputs: PublicInputs {
            input_hash: Some("0x01".to_string()),
            instance_hash: None,
        },
        payload: ProofPayload {
            payload_kind: "risc0_journal".to_string(),
            bytes: vec![1, 2, 3],
        },
        verifier_artifacts: vec![VerifierArtifact {
            kind: "receipt_json".to_string(),
            value: json!("{...receipt...}"),
        }],
        carry_data: None,
        metadata: Some(json!({"fork": "shasta"})),
    };

    let encoded = serde_json::to_vec(&envelope).unwrap();
    let decoded: ProofEnvelope = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded.payload.bytes, vec![1, 2, 3]);
    assert_eq!(decoded.backend, "risc0");
    assert_eq!(decoded.payload.payload_kind, "risc0_journal");
}
```

**Step 2: Run test to verify it fails**

Run:
```
cargo test -p raiko2-primitives proof_envelope_roundtrip
```
Expected: FAIL (types not found).

**Step 3: Write minimal implementation**

In `crates/primitives/src/proof.rs`, add:
- `PublicInputs` struct
- `ProofPayload` struct
- `VerifierArtifact` struct
- `ProofEnvelope` struct
- `AggregationInput` struct

All should derive `Clone`, `Debug`, `Serialize`, `Deserialize`, `PartialEq`, `Eq` where possible.

**Step 4: Run test to verify it passes**

Run:
```
cargo test -p raiko2-primitives proof_envelope_roundtrip
```
Expected: PASS.

**Step 5: Commit**

```
git add crates/primitives/src/proof.rs crates/primitives/tests/proof_envelope_roundtrip.rs
git commit -m "feat: add proof envelope model"
```

---

## Task 2: Add aggregation input adapter for RISC0

**Files:**
- Create: `crates/prover/src/agent/aggregation.rs`
- Modify: `crates/prover/src/lib.rs`
- Test: `crates/prover/tests/aggregation_adapter_risc0.rs` (new)

**Step 1: Write the failing test**

Create `crates/prover/tests/aggregation_adapter_risc0.rs`:

```rust
use raiko2_primitives::proof::{AggregationInput, ProofEnvelope, ProofPayload, PublicInputs, VerifierArtifact};
use raiko2_prover::agent::aggregation::build_risc0_aggregation_input;
use serde_json::json;

#[test]
fn builds_risc0_aggregation_input() {
    let proof = ProofEnvelope {
        backend: "risc0".to_string(),
        public_inputs: PublicInputs { input_hash: None, instance_hash: None },
        payload: ProofPayload { payload_kind: "risc0_journal".to_string(), bytes: vec![] },
        verifier_artifacts: vec![VerifierArtifact {
            kind: "receipt_json".to_string(),
            value: json!("{\"receipt\":true}"),
        }],
        carry_data: None,
        metadata: None,
    };

    let agg = AggregationInput {
        proofs: vec![proof],
        expected_image_id: None,
        metadata: None,
    };

    let bytes = build_risc0_aggregation_input(&agg).expect("build input");
    assert!(!bytes.is_empty());
}
```

**Step 2: Run test to verify it fails**

Run:
```
cargo test -p raiko2-prover builds_risc0_aggregation_input
```
Expected: FAIL (module/function missing).

**Step 3: Write minimal implementation**

- Add `mod agent` in `crates/prover/src/lib.rs` and re-export module.
- Create `crates/prover/src/agent/aggregation.rs` with:
  - `build_risc0_aggregation_input(agg: &AggregationInput) -> RaikoResult<Vec<u8>>`
  - Extract `receipt_json` from `verifier_artifacts`, deserialize into `risc0_zkvm::Receipt`, and build a `BoundlessAggregationGuestInput` (copied from agent schema). Bincode serialize to bytes.

**Step 4: Run test to verify it passes**

Run:
```
cargo test -p raiko2-prover builds_risc0_aggregation_input
```
Expected: PASS.

**Step 5: Commit**

```
git add crates/prover/src/lib.rs crates/prover/src/agent/aggregation.rs crates/prover/tests/aggregation_adapter_risc0.rs
git commit -m "feat: add risc0 aggregation adapter"
```

---

## Task 3: Add agent prover module skeleton

**Files:**
- Create: `crates/prover/src/agent/mod.rs`
- Create: `crates/prover/src/agent/types.rs`
- Modify: `crates/prover/src/lib.rs`
- Test: `crates/prover/tests/agent_request_build.rs` (new)

**Step 1: Write the failing test**

Create `crates/prover/tests/agent_request_build.rs`:

```rust
use raiko2_prover::agent::types::AsyncProofRequestData;
use raiko2_primitives::proof::AggregationInput;

#[test]
fn builds_agent_request() {
    let req = AsyncProofRequestData::new("boundless", "batch", vec![1,2], vec![3,4]);
    assert_eq!(req.prover_type, "boundless");
    assert_eq!(req.proof_type, "batch");
}
```

**Step 2: Run test to verify it fails**

Run:
```
cargo test -p raiko2-prover builds_agent_request
```
Expected: FAIL (types not found).

**Step 3: Write minimal implementation**

In `crates/prover/src/agent/types.rs`, define:
- `AsyncProofRequestData` (serde serialize) with fields matching agent API.
- `AsyncProofResponse`, `StatusResponse` with minimal fields.
- `impl AsyncProofRequestData::new(...)` for convenience.

In `crates/prover/src/agent/mod.rs`, add:
- `AgentConfig` (base URL, api key, poll interval, timeout).
- `AgentClient` with `submit_proof` and `poll_status` signatures (can be `todo!()` for now).

**Step 4: Run test to verify it passes**

Run:
```
cargo test -p raiko2-prover builds_agent_request
```
Expected: PASS.

**Step 5: Commit**

```
git add crates/prover/src/agent/mod.rs crates/prover/src/agent/types.rs crates/prover/src/lib.rs crates/prover/tests/agent_request_build.rs
git commit -m "feat: add agent prover types"
```

---

## Task 4: Implement AgentProver (proposal flow)

**Files:**
- Modify: `crates/prover/src/agent/mod.rs`
- Modify: `crates/prover/src/lib.rs`
- Test: `crates/prover/tests/agent_submit_proposal.rs` (new, mocked HTTP)

**Step 1: Write the failing test**

Create `crates/prover/tests/agent_submit_proposal.rs` using `wiremock` or `httpmock`:

```rust
// pseudo, choose one mock crate
// - mock POST /proof -> { request_id }
// - mock GET /status/:id -> { status: "fulfilled", proof_data: [1,2,3] }
// assert AgentProver returns decoded ProofEnvelope
```

**Step 2: Run test to verify it fails**

Run:
```
cargo test -p raiko2-prover agent_submit_proposal
```
Expected: FAIL (logic not implemented).

**Step 3: Write minimal implementation**

- Use `reqwest` to call agent endpoints.
- `prove_encoded` should:
  - Build request data with `proof_type = batch`.
  - Submit to `/proof`.
  - Poll `/status/{request_id}` until `status == fulfilled`.
  - Decode `proof_data` (RISC0: bincode into agent `Risc0Response` equivalent) and map to `ProofEnvelope`.

**Step 4: Run test to verify it passes**

Run:
```
cargo test -p raiko2-prover agent_submit_proposal
```
Expected: PASS.

**Step 5: Commit**

```
git add crates/prover/src/agent/mod.rs crates/prover/src/lib.rs crates/prover/tests/agent_submit_proposal.rs
git commit -m "feat: add agent proposal prover"
```

---

## Task 5: Implement AgentProver aggregation flow

**Files:**
- Modify: `crates/prover/src/agent/mod.rs`
- Test: `crates/prover/tests/agent_submit_aggregation.rs` (new)

**Step 1: Write the failing test**

Test should:
- Build `AggregationInput` with a RISC0 receipt artifact.
- Mock agent `/proof` + `/status`.
- Assert `AgentProver::aggregate` returns a `ProofEnvelope`.

**Step 2: Run test to verify it fails**

```
cargo test -p raiko2-prover agent_submit_aggregation
```
Expected: FAIL.

**Step 3: Write minimal implementation**

- Use `build_risc0_aggregation_input` to construct `input` bytes.
- Submit with `proof_type = aggregate`.
- Poll status and decode `proof_data` into `ProofEnvelope`.

**Step 4: Run test to verify it passes**

```
cargo test -p raiko2-prover agent_submit_aggregation
```
Expected: PASS.

**Step 5: Commit**

```
git add crates/prover/src/agent/mod.rs crates/prover/tests/agent_submit_aggregation.rs
git commit -m "feat: add agent aggregation prover"
```

---

## Task 6: Wire AgentProver into server config

**Files:**
- Modify: `bin/raiko2/src/config/prover.rs`
- Modify: `bin/raiko2/src/server/state/mod.rs`
- Modify: `bin/raiko2/src/server/handlers/info.rs`
- Test: `bin/raiko2/src/config/mod.rs` tests (add agent parser case)

**Step 1: Write failing test**

Add test in `bin/raiko2/src/config/mod.rs`:

```rust
#[test]
fn prover_type_accepts_agent() {
    let cfg = ProverType::from_str("agent").unwrap();
    assert!(matches!(cfg, ProverType::Agent));
}
```

**Step 2: Run test to verify it fails**

```
cargo test -p raiko2 -- config::tests::prover_type_accepts_agent
```
Expected: FAIL.

**Step 3: Implement minimal changes**

- Add `Agent` to `ProverType`.
- Add `agent` config fields (endpoint URL, API key, poll interval).
- Update server state to construct `AgentProver` and register pipeline.
- Update `/info` supported_provers list.

**Step 4: Run test to verify it passes**

```
cargo test -p raiko2 -- config::tests::prover_type_accepts_agent
```
Expected: PASS.

**Step 5: Commit**

```
git add bin/raiko2/src/config/prover.rs bin/raiko2/src/server/state/mod.rs bin/raiko2/src/server/handlers/info.rs bin/raiko2/src/config/mod.rs
git commit -m "feat: add agent prover config"
```

---

## Task 7: Update docs

**Files:**
- Modify: `README.md`
- Modify: `docs/API.md`

**Step 1: Update docs**
- Add `agent` prover type to supported provers.
- Document config fields (agent URL, API key, polling interval).
- Mention ELF upload strategy (upload on change or on error).

**Step 2: Commit**

```
git add README.md docs/API.md
git commit -m "docs: document agent prover config"
```

---

## Verification

Run (when network access is available):
```
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
```

---

## Notes / Dependencies
- The build currently fails in this environment due to a DNS/GitHub fetch issue; retry once network is available.
- If mocking HTTP is heavy, we can feature-gate tests for agent with `mock-agent` and use minimal integration tests later.

