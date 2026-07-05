# Raiko2 External Prover Intake Criteria

## Status

Draft for discussion.

## Goal

This note defines the review bar for deciding whether a new prover, especially a TEE or TDX-style
prover, fits `raiko2`'s architecture and should be accepted into the repository.

This is intentionally stricter than "can it produce some proof-shaped output". The question is not
only whether a prover runs, but whether it fits the current `raiko2` design, trust boundaries,
operator model, and verification story.

## Why This Note Exists

`raiko2` has changed substantially since PR #2 (`feat: pipeline`, merged on December 22, 2025).
At that point:

- the repository did not yet have the current `GuestInput`-centric architecture
- there was no mature hosted verifier story
- there was no repository-owned SGX remote runtime
- there was no provider-neutral remote prover harness

Review feedback based on that older snapshot is now incomplete.

## Current Architecture Baseline

The current architecture should be reviewed from the repository as it exists on `main`, not from
the early pipeline PR.

### Canonical flow

The current top-level proving model is:

1. `Preflight` builds canonical Shasta inputs from RPC
2. `Validation` checks request invariants and witness-derived data
3. `Prover` consumes canonical inputs
4. `Aggregate` combines proposal proofs when required

This is the source-of-truth flow documented in:

- `README.md`
- `docs/API.md`
- `config.example.toml`

### Canonical proving input

The core architectural move after the original pipeline PR is that provers are expected to consume
canonical `GuestInput`, not re-invent their own preflight model.

That means:

- chain/RPC fetching belongs to `raiko2` preflight
- witness and block reconstruction belong to canonical pipeline code
- prover backends should consume the canonical proving payload, not fetch or derive their own
  alternate execution inputs behind the framework's back

If a new prover duplicates preflight, duplicates witness derivation, or rebuilds its own execution
input outside the canonical `GuestInput` path, it does not fit `raiko2` well.

### Proposal and aggregation are both first-class

`raiko2` is not proposal-only.

A prover integration is expected to explain:

- proposal proof generation
- aggregation proof generation
- proof artifact reuse between the two
- whether aggregation requires same-instance, same-provider, or same-key constraints

Proposal-only integrations are incomplete by default.

## What Changed After The Early Pipeline Work

The easiest way to avoid misjudging a new prover is to anchor on the major capabilities that landed
after the early pipeline PR.

### 1. Guest-driven architecture

The repository later introduced:

- explicit guest programs for proposal and aggregation
- `GuestInput` serialization across prover implementations
- `preflight` and `guest-launcher` tooling
- backend selection through stage-specific guest artifacts

Implication:

- a modern prover should be explainable as a consumer of canonical guest inputs and stage-specific
  proving artifacts

### 2. Hosted zk verifier story

The repository later added a stronger hosted verification flow for zk provers, especially SP1:

- pair-level verifier RPC and verifier address config
- remote verifier-contract checks for hosted SP1 proofs
- image or verifier registration helpers in `xtask register-image`
- aggregation-side checks that proof artifacts actually match expected image IDs

Implication:

- zk integrations are no longer judged only by proof generation
- they are judged by proof generation plus verification and registration semantics

### 3. Remote SGX runtime

The repository now has a dedicated remote SGX model:

- `raiko2` main service remains orchestration and preflight
- a separate `raiko2-sgx-prover` runtime serves the remote proving API
- `proof_type=sgx` and `proof_type=sgxgeth` map onto `sgx/remote`
- `sgx` is repo-local runtime ownership
- `sgxgeth` remains external provider ownership

Implication:

- TEE-style integrations are now expected to justify whether they belong in-process or as a remote
  runtime
- "just add another prover backend" is no longer obviously the right design for TEE

### 4. Provider-neutral remote prover harness

Work now exists to move from a `gaiko2`-specific remote contract to a provider-neutral contract
owned by `raiko2`.

That work introduces:

- provider-neutral schema names
- canonical remote request fixtures
- black-box conformance tests
- live aggregate conformance derived from a real proposal proof

Implication:

- a new remote prover should preferably target the generic remote prover contract, not invent a
  one-off protocol

## Intake Categories

A proposer should state clearly which of the following categories their prover belongs to.

### Category A: Canonical zk backend

Examples:

- `risc0/local`
- `risc0/network`
- `sp1/local`
- `sp1/network`

Expected properties:

- consumes canonical `GuestInput`
- uses canonical stage split: proposal and aggregation
- has a verifier story
- has a registration story
- integrates into existing route and artifact semantics

### Category B: Remote TEE provider

Examples:

- `sgx/remote`
- an external prover that follows the remote prover harness

Expected properties:

- `raiko2` still owns preflight and canonical input construction
- the remote service only proves canonical replay or aggregation payloads
- the request and response contract is explicit and stable
- operator lifecycle is separate from main-service lifecycle
- the attestation and trust model is documented

### Category C: Experimental in-process trusted backend

This category should be treated as exceptional.

A proposer must justify:

- why it should not be modeled as a remote TEE provider
- why it belongs in the main `raiko2` process
- how it avoids creating a second architecture for trusted proving

Without a strong justification, a TEE or TDX design should not default to this category.

## Required Submission Package

Before a new prover is considered for inclusion, the proposer should provide the following.

### 1. Architecture image

This is mandatory.

The image must show:

- `raiko2` main service
- preflight and validation stages
- canonical `GuestInput`
- proposal and aggregation stages
- where proving happens
- whether the prover is in-process or remote
- where attestation happens
- where verification happens
- which artifacts exist at each boundary

The image should not collapse important trust boundaries into a single box labeled "prover".

### 2. Interface contract table

The proposer should include a table with at least these rows:

- canonical input type
- request schema
- response schema
- proposal support
- aggregation support
- artifact output fields
- retry model
- idempotency behavior

The table should answer whether the design reuses:

- existing API routes
- existing runtime task model
- existing artifact model
- existing remote prover harness

### 3. Trust model

This section must answer:

- what is actually being proven
- who is expected to trust the result
- whether trust comes from zk verification, remote attestation, or both
- who verifies the quote or attestation document
- whether there is an on-chain verifier
- if not, what the chain-facing trust story is

This is the section most likely to expose architectural mismatch.

### 4. Verification and registration story

The proposer must specify which of these applies:

- verifier contract call
- image ID registration
- vk digest registration
- enclave or measurement registration
- external verifier scripts or tooling

It must also be clear whether this registration flow is:

- performed by `raiko2`
- performed by `xtask`
- performed by external tooling
- out of scope for the repository

If the answer is "external tooling", that is acceptable only if the trust model remains coherent.
It is not acceptable to leave this undefined.

### 5. Operator flow

The proposer should document:

- how the prover is built
- how guest artifacts are built, if relevant
- how the runtime image is produced
- how configuration is applied
- how keys, quotes, measurements, or instance IDs are provisioned
- how the service is started
- how `raiko2` points at it

If the operator flow introduces a second, unrelated lifecycle from the rest of `raiko2`, that is a
design smell and should be justified.

### 6. Failure and recovery model

The proposer should explain:

- retry semantics
- artifact caching semantics
- stale task recovery
- whether proposal and aggregation proofs are resumable
- whether aggregation requires proof material from the same provider instance
- whether key rotation is allowed between proposal and aggregation

If these behaviors are unspecified, the integration is not ready.

### 7. Test and conformance plan

The proposer should list:

- unit tests
- integration tests
- fixture or golden tests
- remote black-box conformance tests, if remote
- proof verification tests
- aggregation compatibility tests

If the integration is remote, it should explain whether it conforms to the repository-owned remote
prover harness and how that conformance will be enforced.

## Acceptance Bar For A New Prover

The following are the minimum acceptance rules.

### Rule 1: It must not bypass canonical preflight

Rejected if:

- it fetches chain data independently when `raiko2` already canonicalizes that input
- it reconstructs witness or replay state through a private side channel
- it treats `GuestInput` as optional instead of canonical

### Rule 2: It must explain proposal and aggregation, not just proposal

Rejected if:

- proposal is supported but aggregation is "future work"
- aggregation semantics are incompatible with current proof artifact flow
- aggregation requires undocumented hidden state

### Rule 3: It must fit one trust model cleanly

Accepted trust models include:

- zk proof plus verifier contract
- remote attestation plus explicit external trust or registration flow

Rejected if:

- it produces a proof-like payload with no clear verifier or attestation consumer
- it mixes zk and TEE claims without defining which one is authoritative

### Rule 4: It must fit the current route and operator model

Preferred outcomes are:

- reuse existing local or network backend pattern for zk
- reuse existing remote runtime pattern for TEE

Rejected if:

- it requires an entirely separate bespoke API shape
- it requires hidden per-prover workflow that cannot be reasoned about from `raiko2`
- it introduces a one-off config surface that does not generalize

### Rule 5: It must have an artifact and registration story

Accepted if the design clearly identifies:

- what artifact is authoritative
- how that artifact is registered or trusted
- how aggregation verifies compatibility with prior proposal proofs

Rejected if artifact compatibility is hand-waved.

## Recommended Review Questions For A TDX Prover

If the proposer is bringing a TDX prover, ask these questions directly.

### Architecture

- Is this a remote provider or an in-process backend?
- Why is that the correct category for `raiko2`?
- Does it consume canonical `GuestInput`?

### Trust

- Is the primary trust anchor zk verification, TDX attestation, or both?
- Who verifies the TDX quote?
- Is there an on-chain verifier contract?
- If there is no on-chain verifier, how is the proof trusted by downstream consumers?

### Aggregation

- Can proposal proofs from this prover be aggregated?
- Does aggregation require the same instance key or same provider instance?
- What happens if proposal and aggregation run on different machines or keys?

### Operations

- How is the runtime built and deployed?
- What key material is provisioned?
- What measurement or instance identity is registered?
- What exact config does `raiko2` need to point at it?

### Harness compatibility

- If remote, does it implement the provider-neutral remote prover contract?
- Can it pass black-box conformance tests?
- Does it support both proposal and live-derived aggregate requests?

## Immediate Red Flags

The following should trigger a redesign request before merge discussion.

- The prover introduces its own preflight pipeline instead of using canonical `GuestInput`
- The prover only supports proposal proving
- The prover has no clear verifier or attestation consumer
- The prover is remote but does not target the repository-owned remote prover contract
- The prover is TEE-based but is forced in-process without a strong reason
- The prover introduces a bespoke operator lifecycle unrelated to existing repo patterns
- The proposer cannot explain how aggregation reuses or validates proposal proof artifacts

## How To Judge The Current TDX Direction

If a TDX design currently looks like:

- a feature-gated in-process prover
- backed by a local attestation socket
- producing quote-bearing proof envelopes
- without a repository-owned verifier or registration flow

then it should not automatically be treated as production-ready just because it can emit proof
bytes and a quote.

The correct default judgment is:

- promising direction
- insufficient design closure for automatic inclusion
- requires an explicit decision on whether it should become:
  - a remote TEE provider compatible with the remote prover harness, or
  - a fully justified in-process trusted backend with a complete trust story

Without that decision, it is an experiment, not a finished `raiko2` integration.

## Decision Framework

At the end of review, the outcome should be one of these.

### Acceptable for `raiko2` mainline

Use this only if the prover:

- consumes canonical `GuestInput`
- fits proposal and aggregation flow
- has a coherent trust model
- has a verification or registration story
- fits the route and operator model already used by the repository

### Needs redesign before inclusion

Use this if the prover:

- is technically interesting
- may work in isolation
- but does not yet fit the current `raiko2` design cleanly

This should be the default result for designs that are halfway between remote provider and local
backend, or halfway between zk verification and external attestation trust.

### Do not include in `raiko2`

Use this if the prover:

- fundamentally bypasses canonical inputs
- cannot support aggregation coherently
- has no defensible trust story
- requires special-case infrastructure with no path to generalization

## Short Review Message Template

The following message can be sent to a proposer directly.

> Please do not evaluate this prover against the early `raiko2` pipeline snapshot. The current bar
> is the modern `raiko2` architecture: canonical `Preflight -> Validation -> GuestInput -> Prover ->
> Aggregate`, plus the current verifier, artifact, and remote-runtime model.
>
> To evaluate whether this prover fits `raiko2`, please provide:
> 1. an architecture image showing `raiko2`, canonical `GuestInput`, proposal and aggregation,
>    proving location, attestation location, verification location, and artifact boundaries
> 2. an interface table covering request/response contracts, proposal support, aggregation support,
>    artifact outputs, and retry/idempotency behavior
> 3. a trust-model section explaining whether the trust anchor is zk verification, TEE attestation,
>    or both, who verifies it, and whether there is an on-chain verifier
> 4. a verification and registration section explaining image/vk/measurement registration and which
>    tooling owns it
> 5. an operator flow covering build, deploy, configuration, key provisioning, and how `raiko2`
>    points at the prover
> 6. a failure and recovery section covering artifact reuse, retry, stale recovery, and aggregation
>    constraints
> 7. a test and conformance plan, including remote prover harness compatibility if this is a remote
>    provider
>
> A prover is not considered a fit for `raiko2` if it bypasses canonical `GuestInput`, only solves
> proposal proving, has no clear verifier or attestation consumer, or introduces a one-off lifecycle
> outside the repository's current proving model.
