# Native Local Guest Execution Implementation Plan

> Execute this plan task-by-task, validating each step before moving to the next.

**Goal:** Make `native/local` execute the same guest-core Shasta proposal and aggregation logic as
the zk proving paths while preserving the existing deterministic mock native proof envelope.

**Architecture:** Move `raiko2-guest-common` into the main workspace as a normal shared crate, then
reuse it from both `crates/prover` and the zk guest entrypoint crates. Proposal proofs should call
the shared proposal helper, and aggregation should call the shared aggregation helper with a native
proof-envelope verifier closure. Keep the route, envelope format, and public API semantics
unchanged.

**Tech Stack:** Rust workspace crates, `raiko2-guest-common`, Shasta guest-common helpers,
`raiko2-prover`, focused cargo tests

---

### Task 1: Move guest-common into the main workspace

**Files:**
- Create: `crates/guest-common/**`
- Delete: `guests/common/**`
- Modify: `Cargo.toml`
- Modify: `guests/risc0/Cargo.toml`
- Modify: `guests/sp1/Cargo.toml`

**Steps:**
1. Move the `raiko2-guest-common` crate from `guests/common` into `crates/guest-common`.
2. Add the new crate to the root workspace members.
3. Update `guests/risc0` and `guests/sp1` path dependencies to point to the new location.
4. Keep the crate name `raiko2-guest-common` unchanged so the guest entrypoints do not need code
   churn beyond dependency paths.

### Task 2: Wire the shared crate into the host native prover

**Files:**
- Modify: `crates/prover/Cargo.toml`
- Modify: `crates/prover/src/native.rs`

**Steps:**
1. Add a path or workspace dependency from `crates/prover` to `crates/guest-common`.
2. Import `prove_shasta_proposal_for_proof_type` and `aggregate_shasta_zk_with_verifier` into
   `crates/prover/src/native.rs`.
3. Keep existing mock proof envelope constants and helpers intact until tests are updated.

### Task 3: Change proposal native proving to execute guest-common logic

**Files:**
- Modify: `crates/prover/src/native.rs`
- Test: `crates/prover/src/native.rs`

**Steps:**
1. Update `NativeProver::prove_encoded` to compute proposal input hash via
   `prove_shasta_proposal_for_proof_type(&guest_input, ProofType::Native)`.
2. Keep `extra_data` generation unchanged.
3. Keep proof envelope construction unchanged except that the signature input should be the
   guest-common output hash.
4. Update or add a focused test asserting native proposal `Proof.input` matches guest-common output.

### Task 4: Add a reusable native proof-envelope verifier

**Files:**
- Modify: `crates/prover/src/native.rs`
- Test: `crates/prover/src/native.rs`

**Steps:**
1. Add a helper that validates a native proof envelope against an expected `B256` input hash.
2. Verify:
   - proof presence
   - byte length
   - fixed instance id
   - fixed mock instance address
   - deterministic mock signature for the expected hash
3. Add focused tests for valid and invalid native proof envelopes.

### Task 5: Change native aggregation to use guest-common aggregation logic

**Files:**
- Modify: `crates/prover/src/native.rs`
- Modify: `crates/prover/src/lib.rs` (only if route-level aggregate validation needs tightening)
- Test: `crates/prover/src/native.rs`

**Steps:**
1. Keep `build_shasta_aggregation_input(&input.proofs)` as the host-side aggregation input builder.
2. Replace the current direct aggregation hash path with
   `aggregate_shasta_zk_with_verifier(...)`.
3. Implement the verifier closure using the new native proof-envelope verifier for each child proof.
4. Keep output proof envelope format unchanged.
5. Add a focused test asserting native aggregation `Proof.input` matches guest-common aggregation
   output.

### Task 6: Tighten or document native aggregate admission behavior if needed

**Files:**
- Modify: `crates/prover/src/lib.rs` (optional)
- Test: `crates/prover/src/lib.rs`

**Steps:**
1. Review whether `validate_external_aggregate_proofs(route, proofs)` for `native/local` should
   remain metadata-only or also enforce presence of `proof.proof`.
2. If tightening is safe, add the minimal validation and update tests.
3. If not tightening now, leave behavior unchanged and keep verifier strict inside native
   aggregation.

### Task 7: Verify focused native behavior

**Steps:**
1. Run focused native prover tests, at minimum:
   - `cargo test -p raiko2-prover native_ -- --nocapture`
2. Run any additional focused tests touched by aggregate validation behavior.
3. Run:
   - `cargo fmt --all -- --check`
   - `git diff --check`

### Task 8: Commit

**Steps:**
1. Review the final diff for route semantics and proof envelope stability.
2. Commit with a Conventional Commit message describing the native/local guest execution change.
