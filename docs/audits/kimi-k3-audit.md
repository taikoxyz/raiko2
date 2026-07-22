# raiko2 Adversarial Soundness Audit (kimi-k3)

- **Date:** 2026-07-22
- **Scope:** Soundness of the Shasta proving pipeline — anything that could cause a *verifying* proof
  of an incorrect claim, with special focus on whether an **incorrect block hash could ever be proven
  into the on-chain Inbox**. Liveness/DoS issues are noted only when they border soundness.
- **Method:** Four parallel adversarial audit passes over disjoint code slices (guest+derivation,
  stateless witness, public-input binding, host input construction), followed by **independent
  verification agents** re-deriving every critical/high candidate from source and from the live
  taiko-mono contracts (`main`, fetched 2026-07-22). Reference material: taiko-mono `Derivation.md`,
  `taiko-client-rs`, alethia-reth, Inbox/Risc0Verifier/SP1Verifier contracts.
- **Audited code:** `guests/{risc0,sp1}`, `crates/guest-common`, `crates/protocol-shasta`,
  `crates/primitives-shasta`, `crates/stateless`, `crates/pipeline`, `crates/provider`.

---

## Executive summary

**No critical or high soundness findings were confirmed.** The end-to-end binding chain that
protects the Inbox from an incorrect block hash was verified to be closed, both in the guest and in
the on-chain contracts:

1. The on-chain `Inbox.prove` recomputes `hashCommitment(commitment)` from calldata and requires
   `commitment.lastProposalHash == getProposalHash(lastProposalId)` — the ring-buffer hash stored at
   propose time, which commits to the true `originBlockHash = blockhash(block.number - 1)`.
2. It also requires `state.lastFinalizedBlockHash == commitment.firstProposalParentBlockHash`
   (continuity) and advances `lastFinalizedBlockHash` to the proven end block hash.
3. The guest recomputes `hash_proposal(proposal)` (keccak of the full ABI-encoded `Proposal`,
   including `originBlockHash`/`originBlockNumber`) and requires it to equal the carry-data hash
   committed in the journal; the supplied L1 header must hash to `originBlockHash`; anchor
   checkpoints are verified header-by-header against a contiguous ancestor chain terminating at the
   origin.
4. The state witness is verified in-guest as a sparse Merkle Patricia trie rooted at the **parent
   state root**; fabricated accounts/slots or silent zero-defaults are not possible (missing nodes
   are hard errors, and absence is Merkle-proven).
5. The on-chain zk verifiers recompute the entire public input from calldata plus `address(this)`
   and the immutable chain id, so no prover-substituted values (verifier address, prover address,
   chain id) can sneak in.

Two findings initially rated HIGH/MEDIUM by the audit passes — an unchecked
`actual_prover`/`prover_address` link in aggregation, and a guest-journal `verifier` address derived
from host-controlled chain spec — were **independently verified to be neutralized by on-chain
behavior** (the verifier slot is pinned to `address(this)` and the signer slot to `address(0)`), and
are reported as Low hardening items. One further candidate (BLOCKHASH opcode error handling) was
**refuted**: the pinned revm 38.0.0 treats a missing-hash DB error as a fatal halt, not a zero push.

### Remaining trust assumptions (the actual root of trust)

These are architectural, not bugs, but they bound what "sound" means here:

| # | Assumption | Consequence if violated |
|---|------------|--------------------------|
| A1 | The `isImageTrusted` image IDs registered on the verifier contracts correspond to a verified build of this exact guest code. | A misregistered/malicious image ID invalidates everything below it. This is the single most important operational control. |
| A2 | keccak256 / SHA-256 collision resistance. | Proposal ring buffer and journal digests rely on it. |
| A3 | Inbox owner governance (`init2`/`activate` recovery functions can write `lastFinalizedBlockHash` directly). | Owner can set a false anchor — pure governance trust, outside proof scope. |
| A4 | The honesty of the L1 data path is enforced *transitively*: the guest never receipt-proves the `Proposed` event; authenticity comes solely from the on-chain ring-buffer hash match (see L-3). | If a future contract change ever accepts a commitment without an exact ring-buffer match, the guest has zero defense-in-depth. |

---

## Findings

All findings are **Low** severity (defense-in-depth / hardening). No Critical or High findings
survived independent verification.

### L-1: Aggregation guest does not assert `prover_address == address(0)` for ZK flows

- **Where:** `crates/guest-common/src/lib.rs:1077-1102` (`aggregate_shasta_zk_with_verifier`),
  `crates/primitives-shasta/src/instance.rs:148-187`
- **What:** The aggregation journal hashes a host-supplied `prover_address` (the `sgx_instance` slot
  of `hashPublicInputs`) with no guest-side constraint. `commitment.actualProver` comes from the
  sub-proof carry data, and the two are never compared or constrained.
- **Why it's Low (independently verified):** On-chain, `Risc0Verifier.verifyProof` and
  `SP1Verifier.verifyProof` hardcode this slot to `address(0)`
  (`hashPublicInputs(_aggregatedProvingHash, address(this), address(0), taikoChainId)`), and
  `LibPublicInput.sol` documents "for ZK this variable is not used and must have value address(0)".
  Any nonzero guest value fails journal matching → revert. The host already sets
  `prover_address: Address::ZERO` (`crates/prover/src/lib.rs:461`). Note the audit's initially
  proposed fix (`assert actual_prover == prover_address`) would *break* honest proofs, since
  `actual_prover` is a real address while `prover_address` must be zero.
- **Recommendation:** Add `ensure!(input.prover_address == Address::ZERO)` in the ZK aggregation
  guest so the contract requirement is enforced in-circuit rather than by host convention. This
  matters if SGX-compose flows ever reuse this code path with a nonzero expected value.

### L-2: Journal-bound `verifier` address is derived from `GuestInput`-controlled chain spec in one builder variant

- **Where:** `crates/primitives-shasta/src/proof.rs:74-85` (`build_proof_carry_data`, untrusted
  variant) vs. `build_proof_carry_data_with_chain_spec` (trusted variant)
- **What:** The untrusted variant resolves the `verifier` address — which is hashed into the
  sub-proof journal — from `first_witness.chain_spec`, i.e. data inside the host-built `GuestInput`.
  The guest consumes `proof_carry_data.verifier` without re-deriving it from the compiled-in
  `TaikoRuntime` spec (it does pin `chain_id` and execution rules from the runtime).
- **Why it's Low (independently verified):** (a) production admission validates carry data with the
  trusted variant (`crates/pipeline/src/forks/shasta/spec.rs:1786`, with a regression test for
  exactly this tampering); (b) on-chain, the verifier contract substitutes `address(this)` for the
  verifier slot when recomputing `hashPublicInputs`, so a forged verifier address yields a journal
  mismatch → **liveness failure, not a soundness hole**. The guest itself never calls either
  builder.
- **Recommendation:** Have the guest re-derive `verifier` from the `TaikoRuntime` compiled-in spec
  (or assert equality against it) to remove the dependence on host validation staying wired
  correctly.

### L-3: The `Proposed` event is never receipt-verified in the guest; L1 anchoring is transitive via the ring buffer

- **Where:** `crates/guest-common/src/lib.rs` (no `receipts_root`/log verification anywhere);
  event decoding at `crates/protocol-shasta/src/shasta/mod.rs` (~L295-315)
- **What:** The guest takes the proposal event from the host and proves consistency with it, but
  never proves the event was actually emitted in the claimed L1 origin block. Soundness rests
  entirely on the on-chain check `commitment.lastProposalHash == getProposalHash(id)`: since
  `hashProposal` covers `originBlockHash`, a fabricated proposal cannot match a real ring-buffer
  slot. Verified against the contracts — this holds, and no bypass path exists in `prove` (forced
  inclusion only affects proposal content, already covered by `hashProposal`).
- **Why it's Low:** Correct today, but it is a single-anchor design. Any future contract change
  (recovery paths, alternate prove flows) that accepts a commitment without the exact ring-buffer
  match would instantly become a soundness bug with zero guest-side defense. Also note
  `proposal.timestamp` is never cross-checked against the anchored `l1_header.timestamp` — today
  both come from the same RPC, but if the anchor binding is ever strengthened independently, the
  timestamp becomes separately falsifiable and it drives fork-rule selection
  (`derive_expected_shasta_blocks`).
- **Recommendation:** Document the single-anchor invariant explicitly in `docs/API.md` /
  `CONCEPTS.md`; consider a guest-side cross-check of `proposal.timestamp` against
  `taiko.l1_header.timestamp`; add a comment/test pinning that proposal authenticity derives solely
  from the ring-buffer hash match.

### L-4: Derivation contains a reachable inline-calldata manifest path whose only guard is an out-of-band policy check

- **Where:** `crates/protocol-shasta/src/shasta/derivation.rs:238-252` (`decode_inline_manifest` is
  fully implemented and reachable when `blobSlice.blobHashes.is_empty()`), guarded only by
  `crates/primitives-shasta/src/blob.rs:58-66` ("inline payloads are not accepted for ZK proposal
  source").
- **What:** If a proposal source with empty `blobHashes` ever reached derivation, the guest would
  happily derive from host-supplied calldata that is **not bound to anything on-chain** (no KZG, no
  calldata hash check against the L1 origin). The policy check currently blocks this on all call
  paths (verified).
- **Why it's Low:** Mitigated today by one check in one place; regression-prone if new validator
  paths are added that don't funnel through `verify_proposal_mode_blob_usage`.
- **Recommendation:** Move the rejection into `decode_inline_manifest`/`prepare_source_manifest`
  itself (or gate it behind an explicit feature), so the unbound-data path is unreachable by
  construction. Also: invalid-offset blob sources degrade to `DerivationSourceManifest::default()`
  rather than erroring — matches driver parity, but worth a comment.

### L-5: `WitnessDatabase::block_hash` halts on legitimately-zero BLOCKHASH reads (liveness edge, flagged for completeness)

- **Where:** `crates/stateless/src/witness_db.rs:130-134`
- **What:** Originally reported as a potential soundness bug ("host omits ancestor hash → BLOCKHASH
  returns 0 → wrong transition proven"). **Independently refuted:** revm 38.0.0 (pinned) maps a DB
  error from `block_hash` to `FatalExternalError` — the guest halts and no proof is produced. The
  residual observation is the opposite edge: BLOCKHASH of a genuinely out-of-range number
  (>256 blocks back, or ≥ current block) must return 0 per consensus, but `WitnessDatabase` errors,
  so a block containing such a read could be unprovable if the witness window is under-provisioned.
- **Recommendation:** Optionally return `Ok(B256::ZERO)` for numbers genuinely outside the
  256-block window — only after the caller has verified window coverage, since doing it naively
  would reintroduce the exact bug that was checked for.

### L-6: Host-side checkpoint verification trusts the RPC-reported `Header.hash` field

- **Where:** `crates/pipeline/src/forks/shasta/checkpoint_verify.rs:65-71`
- **What:** The pre-proving cross-check compares `rpc_last.hash` (an RPC-reported field, never
  recomputed) against the preflight-computed checkpoint hash. A compromised verification RPC could
  forge the field to pass/fail the check spuriously.
- **Why it's Low:** Host-side pre-proving check only; the guest re-verifies everything that matters.
- **Recommendation:** Use `rpc_last.header.hash_slow()` (recomputed) instead of the reported field.

### L-7: Smaller hardening items

- **`u48_to_b256` silent truncation** (`crates/protocol-shasta/src/libhash/encode.rs:4-9`): masks to
  48 bits inside the hash primitive. All current callers guard with `fits_shasta_uint48` in both the
  proposal and aggregation guests — safe today, but the primitive would silently alias two values if
  a future caller forgets the guard. Prefer making the function fail or take a `U48` newtype.
- **Dead `callers` state / unverified `StatelessInput.accounts`** (`crates/stateless/src/sparse.rs:86,349-355`;
  read at `crates/guest-common/src/lib.rs:524-533`): `accounts` is host-supplied and never verified
  against the pre-state root; the anchor-signer nonce check that reads it is security theater (the
  real check happens in the verified trie during execution). Delete or wire up the dead `callers`
  path before any future code starts trusting it.
- **Compact `WitnessHeader` metadata is host-trusted** (`crates/primitives/src/stateless.rs:139-149`):
  hash/timestamp are not recomputed for the compact form. Safe today because
  `ensure_full_ancestor_headers` (`crates/stateless/src/validation.rs:151-162`) rejects compact
  headers on every consensus path — a fragile invariant worth a comment or debug-assert.
- **Trusted image lifecycle**: if an old/deprecated proposal image ID remains in `isImageTrusted`,
  proofs from the old program remain acceptable. Operational hygiene, not a binding bug.
- **Designated-prover economics (informational)**: the guest binds `actual_prover` but there is no
  proof-of-assignment; in permissionless mode anyone can prove and set `actualProver`, receiving
  50% of a slashed liveness bond for late proofs. Verified against `Inbox._processLivenessBond` —
  matches the contract model (griefing/donation vector at worst, not state soundness).

---

## What was checked and found sound

These are the load-bearing checks an attacker would need to break; each was read in code and the
key ones re-verified by an independent agent against the live contracts:

- **Anchor / L1 block hash binding (the audit's central question):** guest enforces
  `l1_header.hash_slow() == proposal.originBlockHash` (`crates/guest-common/src/lib.rs:152-161`);
  `originBlockHash` is inside the `hashProposal` preimage; the ancestor header chain must be
  contiguous, parent-hash linked, and terminate at the origin by number and hash
  (`lib.rs:234-282`); anchor checkpoints must match real verified headers or (in the stalled-anchor
  bypass) equal the parent CheckpointStore state read via Merkle proof against the verified parent
  state root.
- **On-chain anchor enforcement:** `Inbox.prove` recomputes `hashCommitment` from calldata;
  `LastProposalHashMismatch` against the ring buffer; `ParentBlockHashMismatch` continuity against
  `lastFinalizedBlockHash`; `hashProposal` includes `originBlockHash`; no proof path bypasses these
  checks. Guest↔contract encodings (`hash_proposal`, `hash_commitment` incl. 0x20/0xe0 offsets,
  `hash_public_input` word order and `VERIFY_PROOF` domain, zk aggregation wrap) match word-for-word.
- **Witness integrity:** sparse MPT built from prehashed nodes resolves only against the committed
  parent state root; unresolved digests are hard errors (panic-caught); absence is Merkle-proven;
  account/storage/code all fail closed.
- **Block validity:** per-block parent-hash linkage and +1 numbering; reconstructed headers must
  equal the canonical block header byte-for-byte; post-state root and receipts root recomputed and
  compared; anchor tx must be golden-touch signed with correct chain id, base fee, zero priority
  fee, empty access list, and must succeed.
- **Derivation parity:** timestamp bounds (saturating arithmetic), anchor window/monotonicity with
  forced-inclusion exception, gas-limit ±200ppm clamped to [10M, 45M] via u128 intermediates,
  fork-aware per-source max-block limits, undecodable-blob→default-manifest matching driver
  behavior. No overflow found.
- **Blob binding:** KZG proof-of-equivalence plus versioned-hash equality against the on-chain
  `blobSlice.blobHashes` for every blob, inside the guest — blob substitution is impossible without
  changing the on-chain proposal hash.
- **Aggregation integrity:** sub-proof journals re-verified against the expected image id (risc0) /
  `verify_sp1_proof` (sp1); per-index `block_input == hash_shasta_subproof_input(carry)`; sequential
  proposal ids, proposal-hash chaining, checkpoint continuity, chain-id/verifier consistency all
  enforced before the commitment is built; uint48 overflow guarded at both layers.
- **Replay resistance:** chain id bound in sub-proof and aggregation public inputs; on-chain
  `hashPublicInputs` uses the immutable `taikoChainId` — cross-chain replay not possible.
- **BLOCKHASH semantics:** revm 38.0.0 halts on DB errors (no silent zero); ancestor-hash window
  contiguity enforced and anchored to the verified parent header chain.

## Suggested next steps

1. Implement the cheap in-guest assertions: L-1 (`prover_address == 0` in ZK aggregation), L-2
   (re-derive `verifier` from `TaikoRuntime`), L-4 (make the inline-calldata path unreachable).
2. Add a differential test of `blob_coder.rs` offset/frame decoding against `taiko-client-rs`'s
   shasta blob coder (the one area not fully traced against the reference implementation).
3. Document the single-anchor invariant (L-3) and the trusted-image-ID root of trust (A1) in
   operator-facing docs, including an image-deprecation runbook.
