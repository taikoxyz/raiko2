# TDX GuestInput Replay Implementation Plan

**Goal:** Add a TDX remote proving lane that reuses the existing SGX/Gaiko2 guest-input replay protocol instead of introducing node-based witness or local-node logic.

**Architecture:** Raiko2 remains responsible for preflight, validation, and canonical `GuestInput` construction. The TDX provider uses the same remote prover HTTP API as SGX (`/prove/shasta` and `/prove/shasta-aggregate`) and differs only by proof type, pipeline key, base URL, verifier mapping, and provider-side TDX bootstrap/signing identity.

**Tech Stack:** Rust workspace, axum server routing, raiko2 pipeline/proof type enums, existing Gaiko2 remote prover adapter.

---

### Task 1: Add TDX Proof Type And Pipeline Lane

**Files:**
- Modify: `crates/primitives/src/proof_type.rs`
- Modify: `crates/pipeline/src/lib.rs`

**Steps:**
1. Write failing tests for parsing `tdx` proof type and `shasta-tdx-remote` pipeline key.
2. Run targeted tests and confirm they fail.
3. Add `ProofType::Tdx` and `PipelineKey::ShastaTdx`.
4. Run targeted tests and confirm they pass.

### Task 2: Route Public Requests To The TDX Remote Lane

**Files:**
- Modify: `bin/raiko2/src/server/handlers/proof_types.rs`
- Modify: `bin/raiko2/src/server/handlers/proof_route.rs`

**Steps:**
1. Write failing tests for `proof_type=tdx` JSON parsing and route selection.
2. Run targeted tests and confirm they fail.
3. Map `proof_type=tdx` to `PipelineKey::ShastaTdx`, `ProofType::Tdx`, and the `tdx/remote` route while reusing the existing remote prover adapter.
4. Run targeted tests and confirm they pass.

### Task 3: Register TDX Remote Engine Using Existing Gaiko2 Adapter

**Files:**
- Modify: `bin/raiko2/src/cli.rs`
- Modify: `bin/raiko2/src/config/mod.rs`
- Modify: `bin/raiko2/src/config/prover.rs`
- Modify: `bin/raiko2/src/server/state/mod.rs`
- Modify: `bin/raiko2/src/server/startup.rs`

**Steps:**
1. Write failing tests for `remote_tdx.base_url` config/env and startup summary.
2. Run targeted tests and confirm they fail.
3. Register `ShastaTdx` with the same remote prover adapter used by SGX/Gaiko2.
4. Run targeted tests and confirm they pass.

### Task 4: Keep Aggregate Validation And Chain Spec Parsing TDX-Aware

**Files:**
- Modify: `crates/prover/src/lib.rs`
- Modify: `crates/primitives/src/chain_spec.rs`
- Modify docs only if needed.

**Steps:**
1. Keep TDX on the same `sgx/remote` route-level TEE aggregate metadata contract.
2. Add TDX to chain spec proof type parsing and aggregate validation exhaustiveness.
3. Run targeted tests and then a broader `cargo test` subset.
