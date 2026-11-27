# Etna client to Raiko2 integration

This is the initial internal-devnet, SGX-first integration contract. FI is omitted
from this PoC; RISC0/SP1 and production acceptance are later work. Existing
`shasta` API and pipeline names cover the Unzen path. Etna reuses its execution,
witness, signer, quote and response infrastructure with an explicit Etna request
because the statement and consensus inputs differ.

Baseline: Raiko2 draft PR #305; client PR #22280 at `87f95412`; design PR #22425
at `4bb5cff1`; protocol PR #22286 at `9b8d5b4c`. These are development sources,
not a frozen release or a deployed verifier claim.

## HTTP boundary

Run the existing SGX service with `--fork etna` and its registered Etna instance
ID. The initial execution profile is the built-in 167001 devnet with Etna active
throughout the supplied range, a fixed 45,000,000 block gas limit and ruleset 0.
No request-supplied arbitrary chain specification is accepted. A switching-start
range still needs the correct compiled deployment constants; an absent profile
is rejected. The generic acceptance verifier supports trusted switch profiles.

`POST /prove/etna` accepts `Raiko2EtnaRequest` from
`crates/prover/src/remote_prover/protocol/etna.rs`:

- `schema`: `raiko2-etna-request-v1`.
- `payload.input`: `ConsensusRangeInput<ExecutionWitness>`; original blocks,
  statements, native commits, fallback lists, parent bootstrap and state proofs.
- `payload.data_source`: existing `InputDataSource`, including original ordered
  blobs, KZG commitments and proofs.
- `payload.executions`: one existing execution witness and complete ancestor
  header window for every covered block, in height order.
- `payload.shared_state_nodes`: the existing optional shared witness pool.
- `payload.staking`, `payload.unsettled_limit`: configuration preimage, checked
  against the statement and proven Inbox state rather than trusted as settings.
- `payload.verifier_address`: concrete SGX subverifier, not the compose/router
  address. A mismatching address produces a signature the intended L1 verifier
  cannot accept.
- `payload.expected_statement_hash`: immutable client job identity, compared
  with the digest returned after complete range verification.

Etna request bodies are capped at 64 MiB. The initial profile caps a range at 64
blocks, native block bytes at 2 MiB, standalone commit bytes at 512 KiB, private
envelope bytes at 1,500,000, L1 headers at 1,024 and SSZ helper nodes at 65,536.
The execution-item codec retains its separate 124,000-byte limit. These are
initial service resource limits, not proposed changes to consensus validity.

The existing `raiko2-proof-v1` response envelope is reused. `result.proof` is the
raw 89-byte SGX subproof, with the quote supplied separately. `result.input` is
the **SGX signature-domain hash**, not the bare landing statement. It hashes five
ABI words: `VERIFY_PROOF`, L2 chain ID, concrete SGX verifier, landing digest,
actual instance address. The existing generic `hash_public_input` implementation
and signature/quote provider are reused. The client wraps this subproof according
to its selected on-chain verifier; a raw subproof is not a complete compose proof.

Requests must pass complete DA/SSZ/EVM/QC and context verification, including
origin, per-anchor generation/checkpoint/U/rules, exact timestamp, anchor age,
fixed gas/mix hash and applicable switch-start checks, before key/quote access.
The statement's domain, retained parent record and canonical Q identity are also
bound by L1's recomputed public input. Raiko2 does not add a second RPC approval
of those public words, collect QCs, choose Q or submit transactions.

## Existing client inputs to reuse

`ProofProvider::prove(&ProofRequest)` is already the backend boundary; implement
a Raiko2 provider there instead of changing block production. `PreparedBatch`
already contains the immutable statement hash, ABI landing input, cut leaves,
blobs, exact block/commit pairs, parent EL/Comet headers, signer sets, origin
fallback keys and raw starting/per-anchor/switch witnesses.

Preserve original canonical protobuf and EIP2718 bytes; do not reconstruct them
through a lossy domain adapter. Derive and verify exact required-certificate
coverage; `commit_heights` is metadata. At the first two generation heights the
origin fallback supplies the bootstrap sets omitted by optional client fields.
Decode the seven-field `LandInput` ABI exactly and preserve full-width words.

Convert EIP-1186 proof nodes to existing `ExecutionWitness`/`WitnessStateNode`
resources. Use the established shared-pool index convention; unused pool nodes
are not implicitly part of an inline witness. `staking_reads` values and
`AnchorState` metadata are advisory and cannot substitute for authenticated nodes.

## Client implementation checklist

- [ ] Implement the HTTP `ProofProvider` adapter, error/retry handling and exact
  immutable statement identity comparison; replace the current dummy backend
  only in explicitly selected SGX mode.
- [ ] Map the existing batch fields and original witness bytes to the request
  above. Merge switch-start implementation proof nodes into the existing parent
  execution resources when required.
- [ ] Attach actual per-block execution witnesses using existing debug witness
  APIs. Supplement untouched staking snapshot and missing-validator seat slots
  with standard historical `eth_getProof`, pinned to each exact parent hash.
  `HeightReads` alone contains values, not the required MPT nodes. This does not
  require a new custom execution-node RPC. Existing Raiko2 host code already
  exposes `L2Provider::batch_storage_proof_witnesses`; reuse existing fetching
  and node types rather than duplicate trie verification.
- [ ] Supply a fallback-key preimage for each anchor required by the current
  Raiko2 composer, including anchors where the candidate wins. The client's
  existing list-by-hash lookup can provide it; `fallback_key_lists` currently
  retains only lists for actual fallback decisions.
- [ ] **Extend history witness transport:** supply `L1HistoryInput` for the
  lander-selected Q, including beacon reference header, execution payload
  openings and historical/state multiproof helpers for older anchors. Current
  `reference_headers` are a raw EL parent chain and cannot be passed as SSZ
  evidence. An anchor equal to Q needs no beacon opening. Keep Q selection and
  retry in the lander.
- [ ] Confirm the actual testnet SGX verifier address, instance registration,
  single-SGX acceptance configuration and compose-proof encoding. Provide any
  switching-start deployment constants before using that profile.

Only the SSZ transport/builder is a new witness capability in this inventory.
Other items use existing data, existing RPCs or ordinary backend adaptation.
No missing API is represented by a host-approved flag inside the verifier.

## Verified source checkpoint

At `f94ece85`, [native/SGX CI](https://github.com/taikoxyz/raiko2/actions/runs/45978411297)
passed all five relevant jobs, including SGX all-target Clippy, genuine native
request export and explicit HTTP/software-signature acceptance. Independent
review and behavioral verification passed all 111 focused functions and matched
31 source blobs against the actual effective merge. The RISC0-dependent xtask
job remains red at a third-party recursion-artifact download (HTTP 403).

## Validation boundary

Native tests, independent source review and software signing tests do not prove
an enclave measurement, attestation registration or successful L1 landing. Those
require an actual testnet fixture and separately authorized operational execution.
The full FI relation and production networks remain outside this initial profile.
