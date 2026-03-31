# Native Proof Generator (Shasta SGX Format) — Design

> Historical design document. It may not match the current implementation. Use `README.md`,
> `docs/API.md`, and `config.example.toml` as the current source of truth.

## Goal
Align raiko2 native proof outputs with the **old raiko Shasta SGX proof format** so raiko2 can replace raiko without changing verification logic. This applies to **proposal proofs** and **native aggregation proofs**. The format is:

```
4 bytes  instance_id (big-endian)
20 bytes instance address
65 bytes recoverable secp256k1 signature (v = 27/28)
```

The signature must be over the correct Shasta hash (proposal instance hash or PCD aggregation hash). The **instance id is fixed** to `0xDEAD_C0DE` for this phase.

## Scope
- Proposal native proofs (`NativeProver::prove_encoded`) emit `Proof.proof` as the SGX-format bytes (hex string), `Proof.input` as the proposal instance hash, and `Proof.extra_data` as the PCD JSON.
- Aggregation native proofs (`NativeProver::aggregate`) compute the **Shasta PCD aggregation hash**, sign it, and emit the same SGX-format bytes. `Proof.input` is the aggregation hash.
- Use the fixed private key `92954368afd3caa1f3ce3ead0069c1af414054aefe1ef9aeacc1bf426222ce38` and derive the instance address from it.

Non-goals: zk aggregation, changing proof envelope schema, HTTP API flows.

## Design Notes
- Keep the signing logic local to `raiko2-prover` with a small helper: sign hash → recoverable signature bytes.
- Build proof bytes as a Vec with fixed length 89 bytes.
- The proof bytes are opaque to other systems; no parsing is done in raiko2.
- For aggregation, compute the PCD aggregation hash using existing Shasta helpers (commitment + `shasta_aggregation_output`).

## Testing
- Unit test verifies signature recovery matches expected address `0x0000777735367b36bC9B61C50022d9D0700dB4Ec`.
- Unit test verifies proof layout: length 89, instance id prefix, correct instance address, and signature recovery.

## Deliverables
- Update `crates/prover/src/native.rs` to emit SGX-format proof bytes for proposal + aggregation.
- Fixed instance id constant `0xDEAD_C0DE`.
- Tests for signature recovery and proof format.

## Rollout
- Implement and land as an isolated PR.
- Regression harness will rely on this output format in a follow-up PR.
