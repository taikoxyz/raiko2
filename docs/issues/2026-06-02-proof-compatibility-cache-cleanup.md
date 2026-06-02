# Proof Compatibility Cache Cleanup

## Status

TODO.

## Problem

Aggregation requires sub-proofs to share the same backend compatibility identity. For ZK
backends this is the guest image ID or verifier key identity. For TEE/remote backends this is the
bootstrap or attestation identity used by the verifier.

If a remote prover restarts or upgrades, previously produced sub-proofs may no longer be
compatible with newly produced sub-proofs. A remote-side active notification is not reliable enough
to drive correctness because the host may miss restarts, upgrades, or in-flight replacement.

## Desired Direction

Use proof receipt as the source of truth.

- Extract a normalized `proof_compatibility_id` from the returned `Proof` and its existing backend
  metadata.
- Store the extracted identity with task metadata and proof artifacts.
- Group aggregate inputs by `proof_compatibility_id`; never aggregate mixed identities.
- When a newly received proof has a different identity from the active identity for the route,
  mark old cached artifacts stale for that route and prevent accidental reuse.
- Treat any remote identity endpoint as an observability or readiness hint only, not as the
  correctness mechanism.

## Backend Notes

- SGX/remote proofs should derive compatibility from proof metadata such as bootstrap identity,
  instance address, public key, signer, or quote-derived attestation fields.
- SP1 proofs should derive compatibility from the verifier key or image identity already carried
  by the proof.
- RISC0 and Boundless proofs should derive compatibility from their image identity.
- Native/mock proofs need an explicit fixed compatibility identity or should be excluded from
  persistent cross-version cache reuse.

## Open Questions

- Which exact SGX fields should be canonical for aggregate compatibility: instance address,
  public key, signer plus measurement, or a hash of several fields?
- Should old-identity cached artifacts be retained for old-identity aggregates, or immediately
  pruned after a remote identity rotation?
- Should pure-cache aggregate requests require an explicit target compatibility identity when no
  fresh proof is received?
