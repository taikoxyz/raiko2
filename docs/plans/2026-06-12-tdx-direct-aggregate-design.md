# TDX Direct Aggregate Proof Design

## Goal

Add a direct aggregate proof path for the `tdx_dcap` remote prover lane.

For TDX, the proposal proof is only an intermediate TEE signature. It does not
have the expensive sub-proof production cost that zk proof systems have. The
direct aggregate path should let raiko2 submit one aggregate request containing
the canonical Shasta proposal parameters and L2 block ranges, and let the TDX
remote prover build the aggregate `ProofCarryData` vector and sign it once.

The final proof statement must remain equivalent to the current aggregate proof:

```text
TDX signs shasta_aggregation_output(
    commitment(proof_carry_data_vec),
    chain_id,
    verifier,
    tdx_instance
)
```

The on-chain verifier should not need a new entrypoint. It still verifies the
same final commitment hash and the same `instance_address || signature` proof
format.

## Problem

The current TDX aggregation path is:

1. raiko2 builds a proposal proof request.
2. the remote TDX provider signs a single-proposal carry vector.
3. raiko2 persists the proposal proof artifact.
4. raiko2 builds an aggregate request from one or more proposal proof artifacts.
5. the remote TDX provider verifies those sub-proof signatures and signs the
   aggregate carry vector.

That shape is useful for zk provers because sub-proofs are expensive artifacts.
For TDX, it adds mostly versioning and cache coupling:

- aggregate proving can fail because it consumes stale proposal proof artifacts
  from an older remote schema or older host-side encoding;
- raiko2 has to orchestrate proposal tasks even when the caller only wants the
  aggregate proof;
- a remote/host upgrade can create artificial `v2 aggregate` vs `v1 proposal`
  incompatibilities that do not exist in the TDX security model.

There is also a provider-side protocol gap to close while doing this: the
current TDX remote request payload does not carry `l2_block_numbers`. The
remote must not infer L2 block selection from `proposal_id`; raiko2 already has
the canonical L2 block range in the Shasta request, and the direct TDX request
must carry that range explicitly.

## Decision

For `tdx_dcap` aggregate requests, make the default path a direct aggregate
request:

- external raiko2 API stays the same: `proof_type = "tdx_dcap"` and
  `aggregate = true`;
- raiko2 does not enqueue proposal proof subtasks for that path;
- raiko2 sends the canonical proposal fields plus `l2_block_numbers` directly
  to the remote TDX prover;
- the remote TDX provider fetches header-only L2 data from its local L2 node for
  those exact block numbers;
- the remote TDX provider builds the full `proof_carry_data_vec`, signs one
  aggregate hash, and returns the TDX proof response schema;
- raiko2 validates the returned carry vector against the request and recomputes
  the signed input before packaging the proof.

Keep the existing single proposal endpoint and the old proof-based aggregate
endpoint during migration. They remain useful for debugging and backward
compatibility, but raiko2 should prefer direct aggregate for `tdx_dcap` batch
proving once both sides support it.

The remote implementation does not have to be Nethermind's `reth-tdx`
repository specifically. The same protocol can be implemented in gaiko2's TDX
mode or in `reth-tdx`. The important boundary is the protocol and measurement:
the measured TDX service must own the L2 header fetch, proposal/block
validation, carry construction, and final signature.

## Remote Protocol

Use a new schema discriminator for the direct payload:

```text
reth-tdx-shasta-direct-aggregate-request-v1
```

Use a dedicated HTTP endpoint:

```text
POST /prove/shasta-direct-aggregate
```

This is better than overloading `/prove/shasta-aggregate` because the old
endpoint means "aggregate already-produced sub-proofs", while the direct
endpoint means "derive the full carry vector from proposal parameters and local
L2 headers, then sign once". Keeping the routes separate reduces compatibility
branches, makes metrics/logging clearer, and avoids accidental use of stale
proposal proof artifacts.

`/prove/direct-aggregate` would also work for a Shasta-only service, but the
recommended route keeps the existing Shasta naming convention:
`/prove/shasta`, `/prove/shasta-aggregate`, and
`/prove/shasta-direct-aggregate`.

Proposed request shape:

```json
{
  "schema": "reth-tdx-shasta-direct-aggregate-request-v1",
  "payload": {
    "proposals": [
      {
        "chain_id": 167001,
        "verifier": "0x75e0225807Aa5c7876E486ED8C454c1134Fd7C83",
        "proposal_id": 16,
        "proposal_hash": "0x...",
        "parent_proposal_hash": "0x...",
        "actual_prover": "0x0000000000000000000000000000000000000000",
        "transition": {
          "parentTransitionHash": "0x...",
          "checkpoint": {
            "blockNumber": "0x...",
            "blockHash": "0x...",
            "stateRoot": "0x..."
          }
        },
        "l2_block_numbers": [16]
      }
    ]
  }
}
```

The exact `transition` JSON should reuse the existing
`ShastaTransitionInput` encoding. The important new field is
`l2_block_numbers`.

The response stays:

```text
reth-tdx-proof-v1
```

and must include:

- `proof`
- `quote`
- `input`
- `instance_address`
- `proof_carry_data_vec`

## Guest-Side Checks

Here "guest" means the TDX-measured remote prover process and its local L2 RPC
environment, not the zk guest.

The remote TDX provider must treat the request as untrusted input and perform
these checks inside the TDX boundary:

- Require `schema == "reth-tdx-shasta-direct-aggregate-request-v1"`.
- Require at least one proposal and enforce a configured maximum proposal count.
- Require each proposal's `chain_id` to equal the prover's configured L2 chain
  id.
- Require each proposal's `verifier` to equal the prover's configured verifier
  address.
- Require proposal ids to be unique and in the request order raiko2 wants to
  aggregate.
- Require each `l2_block_numbers` list to be non-empty, strictly increasing,
  and contiguous.
- Fetch each requested header and decode the Shasta proposal id from header
  `extraData`. Every block in the range must carry the same proposal id as the
  request item. This is the primary proposal-id/block-range binding and must be
  enforced inside TDX, not only by the raiko2 host.
- Reject caller-supplied L2 hash, state root, or parent hash as authority. The
  request may contain L1-derived proposal fields, but L2-derived fields used in
  `ProofCarryData` must come from the TDX-local L2 node.
- Fetch L2 headers for every requested block number with header-only RPC
  (`eth_getBlockByNumber(..., false)` or equivalent). Do not fetch full
  transactions for this path.
- Verify header continuity inside each proposal range:
  `headers[i].parent_hash == headers[i - 1].hash`.
- For multi-proposal direct aggregation, require proposal ids to be consecutive
  and ranges to be ordered. The final carry commitment also enforces
  cross-proposal continuity with `prev.checkpoint.blockHash ==
  next.parent_block_hash`.
- Build the proposal carry data with:
  `parent_block_hash = first_header.parent_hash` and
  `checkpoint = last_header.number/hash/state_root`.
- Build the aggregate commitment from the full carry vector using the same
  Shasta commitment builder as the current aggregate path. Reject if the
  builder reports invalid continuity.
- Compute `shasta_aggregation_output(commitment, chain_id, verifier,
  tdx_instance)` and sign that value once.
- Return the exact `proof_carry_data_vec` that was signed.

The old proof-based aggregate endpoint, if retained, should keep verifying each
sub-proof signature before producing the final aggregate signature.

## Host-Side Checks

raiko2 must also treat the TDX response as untrusted until it is checked.

Request construction:

- Only build a direct aggregate request from canonicalized Shasta proposal
  requests after the existing API validation has run.
- Preserve the exact `l2_block_numbers` from the caller's request.
- Reuse the existing `validate_l2_block_numbers` rules: non-empty, strictly
  increasing, contiguous.
- Do not load or depend on cached proposal proof artifacts for the direct TDX
  aggregate path.
- Include the route/schema in the aggregate task fingerprint so old cached
  aggregate results cannot be reused across protocol versions.

Response validation:

- Require `schema == "reth-tdx-proof-v1"`.
- Require proof bytes to be exactly the expected TDX proof length.
- Decode the instance address from `proof` and require it to match
  `instance_address` when that field is present.
- Require `proof_carry_data_vec.len() == request.proposals.len()`.
- For each returned carry entry, require these fields to match the corresponding
  request proposal exactly:
  `chain_id`, `verifier`, `proposal_id`, `proposal_hash`,
  `parent_proposal_hash`, `actual_prover`, and `transition`.
- Require `carry.transition_input.checkpoint.blockNumber` to equal the last
  value in that proposal's `l2_block_numbers`.
- Recompute the commitment and input hash from the returned carry vector and
  decoded instance address. Reject the proof if it does not match response
  `input`.
- Store the returned aggregate artifact with its `proof_carry_data_vec`; do not
  store synthetic proposal artifacts for this path.

raiko2 should not require its own L2 RPC headers to match the TDX-local L2
headers by default. The point of the TDX lane is to let the attested remote
environment source L2 state. A debug-only strict comparison mode is useful for
operator diagnostics, but it should not be the normal proof validity condition.

## Security Boundary

The direct aggregate proof relies on these boundaries:

- The TDX quote and verifier registration bind the bootstrap key to an accepted
  TDX measurement and trusted runtime parameters.
- The measured TDX image must include the remote prover and the local L2 node or
  otherwise constrain the local L2 RPC endpoint to the intended node.
- raiko2 may choose proposal ids and L2 block ranges, but it cannot supply the
  L2 hash/state root values that become part of `ProofCarryData`.
- the remote TDX provider must not sign a proof for a different chain id or
  verifier than its static configuration.
- On-chain verification remains the final acceptance gate for the signature,
  instance registration, and commitment hash.

This design does not solve TDX image measurement and rollout policy by itself.
The image must still be deployed in a mode where SSH/debug access and runtime
mutation are excluded for production, and any measurement-changing component
update must require a new trusted-parameter update and instance registration.

## Equivalence Requirements

For the same ordered set of Shasta proposals and the same local L2 headers:

- old flow: proposal proof per item, then aggregate those sub-proofs;
- new flow: direct aggregate over the proposal list;

must produce the same `proof_carry_data_vec` and the same final `input` hash.
The signature bytes and quote may differ because they are generated at different
times, but they must verify against the same message.

For a one-proposal request, direct aggregate is still an aggregate proof with a
single carry entry. It does not need to be byte-identical to the proposal proof,
but its carry vector and input must be consistent with the single-proposal
commitment.

## Implementation Plan

### Task 1: Add Direct Aggregate Protocol Types

Files:

- Modify: `crates/prover/src/reth_tdx/protocol.rs`
- Modify: remote provider protocol module, either `reth-tdx/src/protocol.rs`
  or gaiko2's TDX-mode protocol module

Steps:

1. Add `RETH_TDX_SHASTA_DIRECT_AGGREGATE_REQUEST_SCHEMA`.
2. Add `ShastaDirectAggregateRequest`.
3. Add `ShastaDirectAggregatePayload { proposals: Vec<ShastaDirectAggregateProposal> }`.
4. Add `ShastaDirectAggregateProposal` with the existing `ShastaProvePayload`
   fields plus `l2_block_numbers: Vec<u64>`.
5. Update comments to make clear that `proposal_id` is not an L2 block selector.

Tests:

- Serialize and deserialize a direct aggregate request fixture.
- Reject unknown schemas in the remote server.

### Task 2: Implement Remote TDX Direct Signing

Files:

- Modify: remote provider proposal/carry builder module
- Modify: remote provider aggregation module
- Modify: remote provider L2 client module
- Modify: remote provider server/router module

Steps:

1. Refactor carry construction so it accepts an explicit proposal payload and a
   validated header range.
2. Add a header-only fetch helper for multiple block numbers.
3. Validate each `l2_block_numbers` list inside the TDX process.
4. Fetch all requested headers and verify per-range continuity.
5. Decode each header's Shasta proposal id from `extraData` and require it to
   match the request proposal id.
6. Verify multi-proposal ordering: consecutive proposal ids and ordered L2
   ranges.
7. Build one `ProofCarryData` per proposal.
8. Build the aggregate commitment from the carry vector.
9. Sign the aggregate hash once and return `proof_carry_data_vec`.
10. Keep the old proof-based aggregate path for compatibility.

Tests:

- Direct aggregate with one proposal returns one carry entry.
- Direct aggregate with multiple contiguous ranges returns the expected carry
  vector.
- Non-contiguous block numbers fail before signing.
- A block whose header `extraData` encodes a different proposal id fails before
  signing.
- Multi-proposal direct aggregation rejects skipped proposal ids.
- Multi-proposal direct aggregation rejects out-of-order or overlapping L2
  ranges.
- Wrong chain id fails before any L2 fetch.
- Wrong verifier fails before any L2 fetch.
- Header continuity break fails before signing.
- Old proposal-proof aggregate and direct aggregate produce the same carry
  vector and input on a fake L2 client.

### Task 3: Wire raiko2 to Prefer Direct TDX Aggregate

Files:

- Modify: `crates/prover/src/reth_tdx/mod.rs`
- Modify: `bin/raiko2/src/server/handlers/proof_api.rs`
- Modify: engine/runtime task input types as needed

Steps:

1. Add a builder for `ShastaDirectAggregateRequest` from canonical Shasta
   proposal submissions.
2. Add response validation equivalent to the current aggregate validation, but
   match against direct request proposals instead of sub-proof payloads.
3. Point the direct aggregate client at `/prove/shasta-direct-aggregate`.
4. In `build_submission_plan`, special-case
   `aggregate == true && proof_type == tdx_dcap` to create one direct aggregate
   task instead of proposal subtasks plus an aggregate task.
5. Do not read proposal proof artifacts for direct TDX aggregate.
6. Ensure the aggregate task cache/fingerprint includes the direct schema and
   exact proposal request data.
7. Keep `/v3/proof/aggregate` working for explicit proof-artifact aggregation.

Tests:

- `tdx_dcap + aggregate=true` enqueues one direct aggregate task and no proposal
  tasks.
- Non-TDX aggregate paths are unchanged.
- Host rejects a response whose carry vector has the wrong length.
- Host rejects a response whose carry L1 fields differ from the request.
- Host rejects a response whose checkpoint block number differs from the last
  requested L2 block number.
- Host rejects a response whose `input` does not match the returned carry vector
  and instance address.

### Task 4: Update Docs And Fixtures

Files:

- Modify: `docs/API.md`
- Modify: TDX integration/runbook docs once the remote endpoint lands
- Add: direct aggregate JSON fixture under the existing remote prover fixture
  area, if the fixture set is available on the active branch

Steps:

1. Document that external raiko2 API remains stable.
2. Document the remote TDX direct aggregate schema and response expectations.
3. Include one captured devnet direct aggregate request/response pair after the
   endpoint is available.

### Task 5: Devnet Verification

Prerequisites:

- remote TDX provider deployed with direct aggregate schema support.
- TDX verifier and Automata DCAP verifier deployed on the current devnet.
- `chain_spec_list_default.json` points `taiko_dev` `TDX_DCAP` to the live
  verifier address.

Commands:

1. Check remote health:

   ```bash
   curl -fsS http://130.211.228.253:8080/health
   ```

2. Check bootstrap:

   ```bash
   curl -fsS http://130.211.228.253:8080/bootstrap
   ```

3. Discover a current devnet Shasta proposal.
4. Submit `tdx_dcap + aggregate=true` to raiko2.
5. Confirm only one direct aggregate proof task is created.
6. Confirm the proof artifact has `proof_carry_data_vec` and the expected TDX
   metadata.
7. Compute the commitment hash from the returned carry vector.
8. Call the deployed verifier's `verifyProof` against the returned proof.

Acceptance:

- raiko2 produces a direct aggregate artifact without intermediate proposal
  artifacts.
- `input` recomputation passes locally.
- The deployed verifier accepts the proof on devnet.
- Existing SGX/ZK aggregate tests still pass.

## Rollout

1. Add direct aggregate support to the selected remote TDX provider at
   `/prove/shasta-direct-aggregate` behind the new schema.
2. Add raiko2 support while leaving the old proof-artifact aggregate path in
   place.
3. Run black-box remote prover regression against the direct schema.
4. Switch `tdx_dcap + aggregate=true` to direct mode by default.
5. Keep the old TDX proposal endpoint for diagnostics until there is a clear
   reason to remove it.

## Open Questions

- Maximum proposals per direct aggregate request should be configured. Start
  with the same practical limit as current aggregate batch size.
- The direct schema can use full `l2_block_numbers` or compact
  `{ "start": N, "end": M }`. Full `l2_block_numbers` is safer for v1 because
  it preserves the existing raiko2 request semantics exactly.
- The production TDX image policy must separately define how the local L2 node
  endpoint is constrained by the measured image.
