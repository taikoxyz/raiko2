# TDX-Gaiko2 Remote Provider And Image Identity Design

## Status

Draft for discussion.

This document defines the preferred TDX integration model for raiko2 and the
provider-side project currently referred to as `tdx-gaiko2`.

The design goal is to make the TDX proof statement, trust boundary, image identity,
and signing-key lifecycle explicit before accepting a TDX prover implementation.

## Summary

TDX should be modeled as a third-party remote TEE provider, not as a local raiko2
guest backend.

The useful TDX statement is:

```text
A registered TDX instance, running an accepted measured VM image, used the
taiko-geth node inside that same measured VM to accept the same Shasta block
range or checkpoint and signed the same Shasta commitment.
```

The ZK proof remains responsible for execution correctness. The TDX proof provides
independent measured-VM acceptance attestation for the same commitment.

This is a distinct lane from current SGX/Gaiko2 guest-input replay:

- `sgxgeth`: SGX service replays the host-provided raiko2 guest input.
- `tdxgeth`: TDX service checks the commitment against a local taiko-geth node
  running inside the same measured VM.

`tdx-gaiko2` is the provider-side project or image name. `tdxgeth` is the raiko2
proof type and lane name.

## Current Problem

The previous TDX implementation shape was ambiguous.

It put raiko2 inside a TDX VM, but the TDX path did not behave like a raiko2 guest
backend:

- the TDX path split Shasta preflight and fetched only blocks
- it did not fetch witnesses
- it skipped guest-input validation
- `TdxProver` consumed `proof_carry_data`, built a Shasta commitment, and signed it
- the proof was a registered TDX key signature, not evidence that TDX executed the
  raiko2 guest

This can be reasonable only if the intended statement is "a node inside TDX accepted
the same blocks". However, that statement must be explicit and enforced by code. It
cannot live only in a deployment template or a loose `l2_rpc` setting.

The key lifecycle also must be SGX-equivalent. A plain persisted `priv.key` is not
enough. Registration proves that a key was quoted once, but steady-state proofs only
carry `instance_id`, address, and signature. If the old key can still be used after
a measured image change, the verifier cannot distinguish old accepted code from new
unaccepted code.

## Goals

- Add an explicit raiko2 remote lane for TDX-local taiko-geth acceptance.
- Keep `tdx-gaiko2` behind the standard remote prover API.
- Make the TDX proof statement explicit and testable.
- Force the TDX L2 source to be the taiko-geth node inside the same measured VM.
- Bind the TDX signing key to the accepted measured image identity.
- Require re-registration when the measured image changes.
- Keep raiko2 as the owner of the remote prover protocol.
- Preserve the 89-byte TEE proof body shape when possible:
  `instance_id(4) || address(20) || signature(65)`.

## Non-Goals

- Do not make TDX a RISC0/SP1-style guest backend.
- Do not put network I/O into a raiko2 guest program.
- Do not use TDX as a replacement for ZK execution correctness.
- Do not rely on an arbitrary external L2 RPC for TDX proof generation.
- Do not require per-proof on-chain quote verification once registration binds the
  key to an accepted image.
- Do not allow mutable post-boot binary updates to affect the proof statement without
  changing the measured image identity.

## Naming And Lane Boundaries

Provider-side name:

```text
tdx-gaiko2
```

Raiko2 lane name:

```text
proof_type = "tdxgeth"
route      = "tdx/remote" or equivalent explicit remote route
config     = prover.remote_tdxgeth.base_url
```

`tdxgeth` should not reuse `sgxgeth` at the proof-type, pipeline-key, config, or
task-record level. The two lanes may share the same HTTP client and neutral remote
protocol, but they must have separate identities so task records, retries, metrics,
verifier-address selection, and future policy can diverge safely.

If a future TDX lane replays the host-provided guest input without a local node, it
should use a different name, such as `tdxreplay`. That would be a different proof
statement.

## Architecture

`raiko2 host`

- Runs outside the TDX VM.
- Owns the public API, queueing, task lifecycle, and remote prover protocol.
- Selects the explicit `tdxgeth` lane when `proof_type = "tdxgeth"`.
- Sends standard remote prover requests to the configured `tdx-gaiko2` endpoint.
- Does not treat `tdxgeth` as `sgxgeth` with different metadata.

`tdx-gaiko2`

- Runs inside the TDX VM.
- Implements the standard remote prover protocol:
  `POST /prove/shasta`, `POST /prove/shasta-aggregate`, and `GET /healthz`.
- Owns the TDX signing key lifecycle.
- Reads L2 data only from local taiko-geth inside the same measured VM.
- Verifies that the requested Shasta commitment is consistent with the local node.
- Signs the canonical input hash only after local-node checks pass.

`taiko-geth`

- Runs inside the same measured TDX VM.
- Is driven by the local taiko-client driver.
- Is the only L2 source accepted by `tdx-gaiko2`.
- Exposes HTTP, WS, and AuthRPC only on loopback or a private VM-local socket.

`taiko-client`

- Runs inside the same measured TDX VM.
- Watches L1 and drives local taiko-geth through the engine API.
- Uses pinned binary and baked startup config for trusted builds.

`tdxs / vTPM`

- Runs inside or is exposed to the same measured TDX VM.
- Provides attestation quotes.
- Provides or backs a sealing mechanism for the TDX signing key.

`TdxVerifier`

- Runs on-chain.
- Stores trusted image params.
- Registers TDX instance public keys only when attestation verifies against trusted
  image params.
- Verifies steady-state proofs by recovering the registered signer from the
  commitment signature.

## High-Level Flow

```text
raiko2 host
  -> POST /prove/shasta
     tdx-gaiko2 inside measured TDX VM
       -> local taiko-geth only
       -> verify local node agrees with the requested block range/checkpoint
       -> build or validate the Shasta commitment
       -> sign commitment hash with image-bound TDX key
       -> return raiko2-proof-v1

on-chain
  -> verify ZK proof for execution correctness
  -> verify TDX proof for accepted-image signer over the same commitment
```

## Remote Protocol

TDX should use the same neutral remote provider protocol as other TEE providers.

Proposal endpoint:

```text
POST /prove/shasta
request schema:  raiko2-shasta-request-v1
response schema: raiko2-proof-v1
```

Aggregate endpoint:

```text
POST /prove/shasta-aggregate
request schema:  raiko2-shasta-aggregate-request-v1
response schema: raiko2-proof-v1
```

Health endpoint:

```text
GET /healthz
```

Bootstrap endpoint:

```text
GET /tdx/bootstrap
```

The bootstrap endpoint may be provider-specific. The proof endpoints should remain
provider-neutral.

## Proposal Proof Statement

Input from raiko2:

- Shasta proposal context encoded in the standard remote request.
- Network and fork identity.
- Block range and `ProofCarryData` needed to bind the same Shasta commitment.

Provider-side work:

- parse the standard `raiko2-shasta-request-v1`
- reject mismatched network, fork, chain id, or unsupported proof-carry shape
- fetch the relevant block headers or blocks from local taiko-geth only
- verify block continuity, block hash, state root, receipts root, and final checkpoint
- verify the local taiko-geth facts agree with the requested commitment opening
- compute the canonical verifier input hash for the TDX instance
- sign that hash with the registered TDX key

Output:

- `schema = "raiko2-proof-v1"`
- `proof = 0x<instance_id || address || signature>`
- `input = <signed input hash>`
- `extra_data = <encoded ProofCarryData or commitment opening>`
- `quote` is optional for diagnostics and should not be required for steady-state
  on-chain proof verification

The statement is not:

```text
TDX executed the raiko2 guest.
```

The statement is:

```text
The accepted TDX VM image, through its local taiko-geth node, accepted the same
blocks or checkpoint and signed the same Shasta commitment.
```

## Aggregate Proof Statement

For aggregate TDX proofs, `tdx-gaiko2` should:

- accept only child TDX proofs from compatible registered TDX identities, unless a
  future design explicitly supports mixed-image aggregation
- verify child signatures locally before producing the aggregate proof
- build the aggregate Shasta commitment using the standard remote aggregate request
- sign the aggregate input hash with the registered TDX key

The aggregate endpoint should follow the standard remote provider aggregate protocol,
not a TDX-specific shape.

## Local Taiko-Geth Enforcement

`tdx-gaiko2` must enforce that its L2 source is taiko-geth inside the same measured
VM.

Required constraints:

- production mode refuses non-local L2 RPC endpoints
- allowed endpoints are loopback or private Unix sockets inside the VM
- readiness checks taiko-geth chain id, sync/head status, fork config, and block
  availability
- readiness checks that taiko-client is driving local taiko-geth
- proof generation fails if local taiko-geth is unhealthy, stale, or on the wrong
  network
- public JSON-RPC, WS, AuthRPC, and debug APIs are not exposed outside the VM

This constraint is part of the TDX proof statement. It should be enforced in
`tdx-gaiko2`, not only in deployment documentation.

## TDX Image Shape

`simple-taiko-node` is a useful source for service commands and network parameters,
but its compose shape is not acceptable as the trusted production image boundary.

The TDX image should bake in:

- `tdx-gaiko2`
- taiko-geth
- taiko-client driver
- tdxs or the selected attestation helper
- systemd units and startup ordering
- production config templates that affect the proof statement

The TDX image should not depend on:

- floating Docker tags or `pull_policy: always`
- host-mounted startup scripts
- host-provided replacement binaries
- unrestricted `GETH_ADDITIONAL_ARGS`
- public geth debug endpoints

Persistent storage may contain:

- taiko-geth chain data
- sealed TDX signing-key material
- registration state
- logs and metrics state

Persistent storage must not override measured binaries, systemd units, or
statement-affecting config.

## Image Identity

The accepted TDX image identity should cover every component that can affect the
proof statement.

At minimum, this includes:

- `tdx-gaiko2`
- taiko-geth
- taiko-client
- tdxs or the selected attestation helper
- kernel, initramfs, OS base, and system libraries
- systemd units and startup ordering
- statement-affecting config files
- boot chain and measured runtime configuration

The build must produce an immutable manifest:

- git URL and commit SHA for every source repo
- package or binary digests
- build profile and feature flags
- image digest or boot artifact digest
- measured identity values used by the verifier, such as `mrTd`, selected RTMR/PCR
  values, and TCB fields
- verifier trusted-params fingerprint

Floating refs such as `master`, `main`, feature branch names, or mutable image tags
are not acceptable for a trusted release artifact. They may be acceptable for
development builds only.

## Key Lifecycle

First boot of a new accepted image:

1. `tdx-gaiko2` asks the TDX attestation/sealing layer to create a signing key.
2. The private key is generated inside the measured VM.
3. The private key is sealed to the current image identity.
4. The public key is embedded into attestation `userData`.
5. The provider exposes bootstrap data for registration.

Restart of the same image:

1. `tdx-gaiko2` unseals the existing key.
2. The current image identity is compared against the bootstrap or registration
   record.
3. If the identity matches, the provider can continue using the same registered
   instance key.

Image change:

1. the image identity changes
2. the old sealed key must fail to unseal
3. the provider must generate a new signing key
4. a new quote must be produced
5. the new key must be registered on-chain
6. registration succeeds only if the new image identity has been configured as
   trusted

The provider must not silently reuse an old key after an image change.

## Registration State

`tdx-gaiko2` should refuse proof generation until it has a non-zero registered
instance id.

Registration should write local state containing:

- verifier address
- chain id
- trusted params index
- registered instance id
- registered instance address
- bootstrap public key
- image identity fingerprint
- transaction hash and block number

If the current image identity no longer matches the stored registration record,
`tdx-gaiko2` should be unhealthy and refuse proofs.

## On-Chain Requirements

The verifier should enforce:

- quote verification through the configured attestation path
- public key binding through quote `userData`
- trusted image params equality for the selected trusted params index
- one-time address registration, or an equivalent anti-replay rule
- instance expiry and revocation, matching the SGX security model
- steady-state proof verification by recovering the registered signer over the
  canonical commitment input

The registration path is where image identity is verified. The steady-state proof
path can remain a small signature check if key sealing guarantees that only the
accepted image can keep using the key.

## Why Not `tdx/local` Inside Raiko2

`tdx/local` makes the trust boundary harder to reason about.

If the host-side raiko2 pipeline builds a partial `GuestInput` and TDX only signs
`proof_carry_data`, then the TDX proof is mostly signing host-derived data. That
does not capture the taiko-geth-in-TDX trust statement.

If the TDX VM is the real trust boundary, then local-node validation and commitment
checks belong inside `tdx-gaiko2`. The host should send a standard remote request
and receive proof evidence. It should not special-case Shasta preflight just to feed
`TdxProver` enough data to sign.

## Raiko2 Integration Plan

1. Add a new `ProofType::TdxGeth` with string form `tdxgeth`.
2. Add a distinct pipeline key for Shasta TDX-Geth.
3. Add route selection for `proof_type = "tdxgeth"` to the remote TDX lane.
4. Add config and CLI/env wiring for `prover.remote_tdxgeth.base_url`.
5. Reuse the neutral remote prover HTTP client and protocol types.
6. Keep task records, fingerprints, metrics, and startup summaries distinct from
   `sgxgeth`.
7. Add verifier address mapping support for `tdxgeth`.
8. Extend API and config docs with the explicit lane.
9. Add tests proving `sgxgeth` and `tdxgeth` do not collide in routing, fingerprinting,
   or stored task identity.

## TDX-Gaiko2 Provider Plan

1. Implement the same remote prover endpoints as existing Gaiko2.
2. Add a TDX attestation and sealed-key provider.
3. Add local taiko-geth health and checkpoint validation.
4. Refuse proof generation before registration.
5. Refuse proof generation when local taiko-geth is unavailable, stale, or external.
6. Build and publish an immutable image manifest with measured identity values.
7. Pass remote proposal conformance.
8. Pass remote aggregate conformance.
9. Pass real Shasta proposal regression through raiko2 using `proof_type = "tdxgeth"`.

## Acceptance Checklist

Do not accept the TDX provider until these are true:

- raiko2 exposes an explicit `tdxgeth` proof type and lane
- `tdx-gaiko2` accepts `raiko2-shasta-request-v1`
- `tdx-gaiko2` accepts `raiko2-shasta-aggregate-request-v1`
- `tdx-gaiko2` returns `raiko2-proof-v1`
- the image build uses immutable source refs
- the image build emits a manifest and measured identity values
- changing `tdx-gaiko2`, taiko-geth, taiko-client, tdxs, or statement-affecting
  config changes the accepted image identity
- the signing key is sealed to the image identity
- rebooting the same image reuses the key
- booting a changed image cannot unseal the old key
- a changed image generates a new key and requires new registration
- proof generation is refused before successful registration
- the registered instance id is stored and used in proof bytes
- the provider enforces local taiko-geth as the L2 source
- public geth RPC/debug endpoints are not exposed outside the VM
- proposal conformance passes
- aggregate conformance passes
- a real Shasta proposal regression succeeds with ZK plus TDX verification
- documentation states exactly what TDX proves and what it does not prove

## Open Questions

- Which exact TDX measurement fields should be treated as the canonical image
  identity: `mrTd`, RTMRs, PCRs, or a specific combination?
- Which config files are statement-affecting and must be measured?
- Should `tdxgeth` derive more `ProofCarryData` inside the provider, or is initial
  local taiko-geth checkpoint verification against the standard request sufficient?
- Should the provider keep returning per-proof quotes for diagnostics, or should quotes
  be limited to bootstrap and registration?
- How should instance id state be written after registration: local provider state,
  generated config, or a small registry file?
- What is the expected reorg behavior if local taiko-geth accepted a block range that
  later changes?
- Should old registered instances remain valid until expiry when the old image is still
  running, or should rollout procedures revoke old image instances immediately?
