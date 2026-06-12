# Post-Merge Todo Priority

## Context

This snapshot was taken after the chain-spec fork overlay and opcode workload metric work had landed
on `origin/main`. It separates active correctness work from research follow-ups and historical
planning notes.

## Priority Table

| Priority | Item | Source | Status | Next action |
| --- | --- | --- | --- | --- |
| P0 | Proof compatibility cache cleanup | `docs/issues/2026-06-02-proof-compatibility-cache-cleanup.md` | Implemented locally | Review the compatibility-id cache changes; run full `raiko2` server tests once the RISC0 artifact download issue is cleared. |
| P1 | Shasta driver derivation logic drift | `docs/issues/2026-06-02-shasta-driver-derivation-logic-drift.md` | Open | Move derivation preparation into a shared Taiko client/protocol API and reuse it from preflight. |
| P1 | ZK gas realistic revm lab | `experiments/opcode-gas/README.md` | Follow-up | Add Taiko/reth-context inputs around the existing `revm-opcode-lab` path. |
| P1 | ZK gas real workload attribution | `docs/plans/2026-06-08-zkgas-workload-damage-model-design.md` | Follow-up | Report real proposal/app `zk_util` and top opcode/precompile contributors. |
| P2 | Precompile wrapper and CALL family scenarios | `experiments/opcode-gas/README.md` | Follow-up | Add `STATICCALL`, warm/cold, return-data copy, CALL, and CREATE wrapper cases. |
| P2 | Additional zkVM metric adapters | Discussion follow-up | Candidate | Evaluate ZisK profiling/final cost first, then OpenVM meter/segment metrics. |
| P2 | CI native smoke lane | `.github/workflows/ci.yml` | TODO | Add once a deterministic CI-safe native fixture/regression entrypoint exists. |
| P3 | Regression L1 event-based discovery | `scripts/regression/shasta_regression.py` | TODO | Use `event_abi` / `anchor_abi` when event-based proposal discovery is added. |
| P3 | Witness database hashing simplification | `crates/stateless/src/witness_db.rs` | TODO | Consider replacing the trie-backed lookup with a simple map after profiling. |
| P3 | Public packaging hygiene | `docs/issues/2026-05-06-open-source-readiness-review.md` | Optional | Keep as a separate cleanup pass from proving correctness and zk gas research. |

## P0 Scope

The proof compatibility cleanup is first because aggregation correctness depends on not mixing
sub-proofs produced under incompatible verifier identities. The implementation should treat proof
receipts as the source of truth:

- extract a backend-specific compatibility identity from returned proof metadata
- store it with proof artifact/task metadata
- require aggregate inputs in one aggregate request to share the same compatibility identity
- keep remote identity endpoints as readiness/observability hints, not correctness inputs

Backend identity candidates:

- SP1: verifier key or verifier key hash carried by `Proof::uuid`. Local and network SP1 paths
  construct `Sp1Response` with `vkey_hash` and `vkey`; `Sp1Response -> Proof` serializes the full
  verifying key into `uuid` and falls back to `vkey_hash`.
- RISC0 local: proposal image ID carried by `Proof::uuid`. The local prover computes the image ID
  from the proposal ELF, writes it into `Risc0Response.image_id`, and `Risc0Response -> Proof` moves
  it into `uuid`.
- RISC0 Boundless/network: proposal image ID carried by `Proof::uuid` and mirrored in
  `extra_data.risc0.image_id` when available.
- SGX/TDX/remote TEE: quote/bootstrap/signing identity from proof metadata
- Native/mock: explicit fixed local identity, or no persistent cross-version cache reuse

Implementation note: the persisted name should stay generic (`proof_compatibility_id`), not
`image_id`, because SP1, RISC0, and TEE expose different identity material even though the aggregate
compatibility rule is the same.

Current local implementation:

- Computes a normalized `proof_compatibility_id` from the proof receipt and stores it with runtime
  proof artifact records.
- Rejects external aggregate requests and engine aggregate execution when child proofs mix
  incompatible identities.
- During batch planning, if cached proposal artifacts for the same aggregate batch reveal multiple
  compatibility identities, treats the non-target identity as stale, removes that artifact from the
  runtime cache registry, removes the corresponding engine proposal task, and replans it as pending.
- If aggregate execution fails with a compatibility mismatch, the runtime observer prunes stale
  aggregate input artifacts as a fallback for races such as a remote prover restart between child
  proof generation and aggregation.
- If the current expected child-proof identity is known from the aggregate request or a remote
  prover error payload, aggregate execution/error handling also prunes uniformly stale inputs such
  as `A+A` when the aggregate prover now expects `B`.
- Records a small prune marker when observer-driven cleanup removes an artifact. The next planning
  pass consumes that marker and removes the matching proposal task even though the artifact record is
  already gone, avoiding reuse of an in-memory succeeded task with stale proof bytes.

Remaining follow-ups:

- Refine stale-proof recovery into the reprove/input-retention model in
  `docs/plans/2026-06-11-proof-reprove-input-retention-design.md`, so stale proof cleanup preserves
  reusable `GuestInput` or encoded input artifacts.
- Add a remote prover identity/readiness endpoint or equivalent startup hook so a remote restart can
  proactively prune all old artifacts for the affected route before the next aggregate request.
- Keep the aggregate-failure cleanup as a defensive fallback; the preferred steady-state path is
  plan-time filtering against the current expected identity.
