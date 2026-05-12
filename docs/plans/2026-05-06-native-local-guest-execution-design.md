# Native Local Guest Execution Design

## Goal

Change `native/local` so it executes the same Shasta guest-core logic as the zk proving paths,
while still returning a deterministic host-local mock proof envelope instead of a zk proof.

## Problem

The current `NativeProver` does not execute the guest-core Shasta proposal logic. It deserializes
`GuestInput`, reads `proof_carry_data`, computes the final public input hash directly, and returns
a mock SGX-format proof envelope. That is useful for envelope and routing tests, but it does not
exercise the same block reconstruction path used by the guest programs.

For proposal proofs this means `native/local` skips the logic behind
`prove_shasta_proposal_for_proof_type(...)`. For aggregation proofs it skips the guest aggregation
helper shape and only reassembles the final hash from proof-carry metadata.

The desired semantics are:

- keep the existing host-side request, preflight, witness, and `GuestInput` flow
- execute the same guest-core proposal logic that `risc0` and `sp1` run
- keep aggregation aligned with the zk aggregation host path
- continue returning a deterministic local mock proof envelope instead of a zk proof

## Current Reference Points

- Proposal guest entrypoints:
  - `guests/risc0/src/shasta_proposal.rs`
  - `guests/sp1/src/shasta_proposal.rs`
- Shared proposal helper:
  - `crates/guest-common/src/lib.rs:prove_shasta_proposal_for_proof_type`
- Shared aggregation helper:
  - `crates/guest-common/src/lib.rs:aggregate_shasta_zk_with_verifier`
- Current native prover:
  - `crates/prover/src/native.rs`

## Proposed Behavior

### Proposal Path

`NativeProver::prove_encoded` should:

1. Deserialize `GuestInput` as it does today.
2. Execute `raiko2_guest_common::prove_shasta_proposal_for_proof_type(&guest_input, ProofType::Native)`.
3. Use the returned `B256` as `Proof.input`.
4. Preserve the current proof envelope format:
   - 4-byte mock instance id
   - 20-byte mock instance address
   - 65-byte deterministic mock signature
5. Keep `extra_data` sourced from `proof_carry_data`, as it is today.

This makes `native/local` execute the same reconstruction and validation logic used by the guest
programs, without introducing zk proving.

### Aggregation Path

`NativeProver::aggregate` should align with the zk host flow:

1. Build `ShastaZkAggregationGuestInput` using the existing
   `build_shasta_aggregation_input(&input.proofs)`.
2. Execute `raiko2_guest_common::aggregate_shasta_zk_with_verifier(...)`.
3. Supply a native verifier closure that validates each child native proof envelope against the
   expected `block_input`.

The verifier closure should check:

- the child proof contains `proof.proof`
- the child proof envelope length and fixed instance id are correct
- the child proof address matches the deterministic mock native instance address
- the child mock signature matches the expected `block_input`

This mirrors the zk aggregation structure:

- zk guests call `aggregate_shasta_zk_with_verifier(...)`
- the verifier closure is the only backend-specific part
- native should do the same, with a deterministic local mock verifier instead of `env::verify(...)`
  or `verify_sp1_proof(...)`

### What Does Not Change

- `pipeline.validate(...)` remains in the host flow and still validates `GuestInput` before prover
  execution.
- The `native/local` route remains host-local and non-zk.
- The native proof envelope shape remains compatible with the current Shasta SGX-format mock
  output.
- Public HTTP API behavior changes only for the explicit `native/local` regression route:
  `proof_type=native` is accepted there for internal/local regression. Normal zk routes still reject
  native proofs, and external aggregation still validates native child proofs only for
  `native/local`.

## Design Details

### Shared Workspace Crate

Previously, shared guest logic lived under `guests/common` as a nested workspace root, so
`crates/prover` could not depend on it directly as a path dependency. The clean fix was to move the
shared logic into a normal workspace crate, then make both host and guest code depend on that crate.

Current shape:

- `raiko2-guest-common` lives at `crates/guest-common`
- `guests/risc0` and `guests/sp1` depend on the workspace crate
- `crates/prover` depends on the same crate

This is preferable to:

- copying proposal reconstruction logic into `crates/prover`
- teaching `native/local` to drive a specific zk backend in execute-only mode
- keeping two subtly different implementations of the same proving core

This preserves a single source of truth for proposal and aggregation execution semantics.

### Native Proof Verification Helper

Add a small helper in `crates/prover/src/native.rs` to validate a native proof envelope against an
expected input hash. This helper should be used by:

- aggregation verifier closure
- native-specific tests
- optionally future route-level native aggregate validation tightening

This keeps the deterministic mock proof contract explicit rather than implicit.

## Testing Strategy

### Unit Tests

Update native prover tests so they verify:

- proposal `Proof.input` equals the result of guest-common proposal execution
- aggregation `Proof.input` equals the result of guest-common aggregation execution
- proof envelope layout is unchanged
- native child proof verification accepts valid child proofs and rejects malformed envelopes

### Focused Integration Safety

Run focused tests in `crates/prover` and any route/runtime tests that depend on `native/local`
proof shape or aggregation acceptance.

## Risks

- Moving `raiko2-guest-common` into the workspace changes dependency paths for both guest build
  trees and host crates. This must be done carefully to avoid breaking guest compilation.
- Aggregation verifier behavior must not silently weaken. The new native verifier should validate
  proof envelope contents, not only trust `proof.input`.

## Success Criteria

The change is correct when:

- `native/local` proposal proofs execute guest-common proposal logic instead of directly hashing
  proof-carry data
- `native/local` aggregation executes guest-common aggregation logic with a native verifier closure
- proof output envelope format remains stable
- focused native prover tests pass while keeping `proof_type=native` gated to `native/local`
