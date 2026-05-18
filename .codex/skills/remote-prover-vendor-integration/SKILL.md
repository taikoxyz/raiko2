---
name: remote-prover-vendor-integration
description: Use when integrating a third-party remote proving service such as TDX, SGX, or another TEE-backed provider into raiko2, especially when the provider must implement the raiko2 remote prover protocol, pass black-box conformance, bootstrap or init correctly, and clear API regression on both proposal and aggregate proving.
---

# Remote Prover Vendor Integration

## Overview

Use this skill when a new remote proving vendor is being connected to `raiko2`.

This skill is for `remote` provers only:

- `sgx`
- `sgxgeth`
- future remote TEE lanes such as `tdx`

Do not use this skill for `risc0` or `sp1`. `zk` provers remain internal first-class backends and
still consume `GuestInput` directly.

This skill assumes the checkout already contains the neutral remote prover protocol and conformance
harness. If those files are missing, switch to the branch that contains them before proceeding.

## Stable Contract

Every remote vendor must implement the same protocol surface:

- `POST /prove/shasta`
- `POST /prove/shasta-aggregate`

Request schemas:

- `raiko2-shasta-request-v1`
- `raiko2-shasta-aggregate-request-v1`

Response schema:

- `raiko2-proof-v1`

Protocol ownership stays with `raiko2`. The vendor is only a provider implementation.

## Canonical Artifacts

Use these files as the contract source of truth:

- protocol types: `crates/prover/src/remote_prover/protocol.rs`
- black-box conformance: `crates/prover/tests/remote_prover_conformance.rs`
- proposal fixture:
  `tests/fixtures/remote_prover/shasta_request_v1_taiko_mainnet_proposal_2222_l2_5412225_5412416.json`
- aggregate fixture:
  `tests/fixtures/remote_prover/shasta_aggregate_request_v1_single_fixture_proof.json`
- first provider example: `docs/gaiko2-remote-prover-integration.md`

Request compatibility is strict on the vendored fixtures. Treat them as byte-for-byte goldens on
the request side.

Response compatibility is semantic:

- `schema` must be `raiko2-proof-v1`
- required result fields must be present
- `input` must be self-consistent with the submitted carry data

## Integration Flow

### 1. Align the provider to the protocol

Keep the provider surface narrow:

- do not introduce provider-branded schema names
- do not change the endpoint paths
- do not special-case aggregate into a separate incompatible shape

If the provider already has a local protocol module, rename it to the neutral `raiko2` names
instead of adding a compatibility shim.

### 2. Run black-box conformance first

Start the provider locally, then point the harness at it:

```bash
RAIKO2_REMOTE_PROVER_BASE_URL=http://127.0.0.1:8080 \
cargo test -p raiko2-prover --no-default-features \
  --test remote_prover_conformance -- --ignored --nocapture
```

The acceptance gate is:

- `remote_prover_conformance_proposal`
- `remote_prover_conformance_aggregate`

Important aggregate rule:

- do not post the static aggregate fixture directly as the black-box aggregate request
- the harness intentionally derives the aggregate request from a live proposal proof returned by the
  same running provider instance

This matters for providers that require aggregate subproofs to come from the same prover identity.

### 3. If this is a TEE provider, prove the service lifecycle

For SGX, TDX, or similar attested providers, conformance alone is not enough. Also verify:

- image builds successfully
- bootstrap or `--init` succeeds
- service starts after init
- health endpoint is green

Use the provider's real init path, not a hand-written mock.

### 4. Wire the provider into a concrete remote lane

Before API regression, make sure `raiko2` has an explicit lane for the provider being integrated.

Do not collapse distinct providers into one shared runtime identity and recover the distinction
later from ad-hoc metadata. A concrete remote lane should have:

- an explicit `proof_type`
- an explicit pipeline identity
- a concrete remote base URL in config

### 5. Run full API regression

After conformance and init succeed:

- start a fresh `raiko2` API service
- point it at the new provider endpoint
- run proposal regression on a recent real network window
- cover both proposal and aggregate proving if the lane supports both

Use a fresh runtime root for regression so stale failures do not pollute results.

For Shasta-style proposal regression, prefer the repo's existing regression helpers and recent real
proposal windows rather than synthetic one-block smoke tests only.

## Acceptance Checklist

Do not call the integration complete until all of these are true:

- provider accepts `raiko2-shasta-request-v1`
- provider accepts `raiko2-shasta-aggregate-request-v1`
- provider returns `raiko2-proof-v1`
- proposal conformance passes
- aggregate conformance passes
- TEE init or bootstrap succeeds if applicable
- provider health endpoint is green
- `raiko2` API regression completes successfully on a real proposal window

## Common Mistakes

- Treating `zk` provers as remote vendors. They are not.
- Keeping provider-branded schema names and adding aliases instead of moving to the neutral
  protocol.
- Verifying only proposal proving and skipping aggregate proving.
- Using a static placeholder aggregate proof blob for black-box conformance.
- Validating only source layout or submodule structure instead of a running provider endpoint.
- Reusing stale runtime state during regression and then debugging the wrong failure.

## Boundaries

This skill does not cover:

- verifier registration
- chain rollout or deployment orchestration
- Kubernetes or Tolba rollout
- guest ELF changes for `risc0` or `sp1`
- forcing source vendoring or submodules as an acceptance requirement

Submodules are optional. The acceptance gate is a running provider that passes the `raiko2`
protocol contract and API regression.
