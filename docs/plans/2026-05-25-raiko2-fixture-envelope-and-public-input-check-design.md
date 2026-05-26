# Raiko2 Fixture Envelope And Public Input Check Design

## Status

Draft for discussion.

This document defines a fixture contract for provider conformance before the benchmark work. The
main purpose is to separate fixture inputs from provider proof outputs, and to let local tests
open and check the public input commitment without requiring full proof verification on every path.

## Goal

Create a fixture framework where:

- the fixture input is canonical and provider-independent
- the expected public input commitment can be opened and recomputed locally
- local, remote, ZK, and TEE providers can be tested against the same fixture case
- full proof verification is optional and can be enabled when the verifier environment exists

The first implementation should be proposal-only. Aggregate fixtures should be deferred until the
proposal fixture contract and provider public-input conformance path are stable.

For Shasta proposal fixtures, the canonical statement is:

```text
GuestInput -> ProofCarryData -> hash_shasta_subproof_input(ProofCarryData)
```

The fixture framework should make that chain explicit. A provider must not be able to pass
conformance by returning an arbitrary proof blob over an unknown public input.

## Non-Goals

- Do not build the benchmark scoreboard in this workstream.
- Do not require every provider to expose a locally verifiable proof in the first iteration.
- Do not make `ProofEnvelope` the fixture input format.
- Do not replace the public API response shape in the first iteration.
- Do not relax same-instance aggregate checks for TEE providers.
- Do not require old fixture JSON files to stay stable; fixture data may be regenerated.
- Do not make aggregate fixture support part of the first implementation slice.

## Current Problems

### Fixture Inputs And Proof Outputs Are Mixed

Raiko2 currently has several fixture surfaces:

- `test/guest_inputs/shasta/...` stores Shasta `GuestInput` proposal fixtures.
- `tests/fixtures/remote_prover/...` stores remote prover wire request goldens.
- server and prover tests use mock proof blobs for RISC0, SP1, native, and remote paths.

These are all useful, but they do not define a single conformance contract. Some tests exercise
wire compatibility, some replay `GuestInput`, and some only assert that a mock proof field has a
known placeholder value.

### Public Input Checks Are Not The Central Fixture Contract

`xtask replay-guest-input` already does most of the useful local work:

- load a `GuestInput`
- rebuild `ProofCarryData`
- compute `hash_shasta_subproof_input`
- run Shasta guest-input validation

However, that result is a replay report, not the shared fixture contract that remote providers must
match.

The fixture-backed server mock provers also still return placeholder input hashes in some paths.
That is acceptable for pure API tests, but it is not sufficient for provider conformance.

### Full Proof Verification Is Too Heavy As The Minimum Gate

Onchain verification is the final production path, but it is too heavy and environment-dependent as
the only test mode for third-party provider intake. Local verification may require:

- SP1 or RISC0 verifier dependencies
- GPU or proving artifacts
- PCCS or quote policy for TEE paths
- provider-specific metadata
- exact guest image or verifying key artifacts

The fixture framework needs a lower-level conformance gate that proves the provider is proving the
same statement even when full proof verification is unavailable.

## Terminology

### Fixture Input

The provider-independent input to a test case.

For Shasta proposal fixtures, this is currently `GuestInput`.

For aggregate fixtures, this should become a typed aggregate input that references child proof
evidence and child openings.

### Public Input Commitment

The canonical value that the guest or trusted runtime commits to.

For Shasta proposal fixtures:

```text
hash_shasta_subproof_input(ProofCarryData)
```

For Shasta aggregation:

```text
hash_public_input(hash_commitment(Commitment), chain_id, verifier, prover_or_instance)
```

For ZK aggregation over child proofs:

```text
hash_two_values(sub_image_id, sub_input_hash)
```

The exact function depends on the fixture stage and provider class, so it must be named in the
fixture metadata rather than inferred from a generic `proof.input` field.

### Opening

The data needed to locally recompute the public input commitment.

For Shasta proposal fixtures, the opening is `ProofCarryData`, derived from `GuestInput` and checked
against the guest-input invariants.

For Shasta aggregate fixtures, the opening is the ordered `ProofCarryData` vector plus the
aggregation identity fields.

### Provider Evidence

The provider output that claims to prove or attest to the public input commitment.

Examples for proposal fixtures:

- a ZK proof plus verifier artifacts
- a TEE quote plus signature
- a native mock signature
- a proof envelope with provider metadata
- a legacy `Proof` response with `input`, `proof`, `quote`, and `extra_data`

Provider evidence is output. It should not be the fixture input.

## Design Direction

Add a new `FixtureEnvelope` contract for test cases.

`ProofEnvelope` remains the provider output direction. It can be consumed by fixture checks later,
but it should not own the fixture case definition.

The fixture runner should execute in layers:

1. load fixture input
2. open the expected public input commitment locally
3. optionally call a local or remote provider
4. check that provider evidence claims the same public input commitment
5. optionally run full proof verification against that commitment

This makes public input conformance the minimum provider gate, and full proof verification an
additional gate.

For the first implementation slice:

- support only `stage = proposal`
- support only `input.kind = shasta_guest_input`
- support only the Shasta subproof public input commitment
- defer aggregate fixtures and aggregate openings to a later phase

## Fixture Envelope Schema

Logical JSON shape:

```json
{
  "schema": "raiko2-fixture-v1",
  "case_id": "shasta/taiko_masaya/proposal_22758",
  "fork": "shasta",
  "stage": "proposal",
  "description": "Masaya Shasta proposal fixture",
  "statement": {
    "proof_type": "native",
    "public_input_kind": "shasta_subproof_input"
  },
  "input": {
    "kind": "shasta_guest_input",
    "path": "test/guest_inputs/shasta/taiko_masaya/proposals/proposal_22758.json",
    "sha256": "0x..."
  },
  "expected": {
    "commitment": "0x...",
    "opening": {
      "kind": "shasta_proof_carry_data",
      "proof_carry_data": {}
    }
  },
  "checks": {
    "open_commitment": true,
    "check_provider_public_input": false,
    "verify_proof": false
  },
  "providers": {}
}
```

Important rules:

- `schema` is required and versioned.
- `case_id` is stable and human-readable.
- `input.path` is repo-relative.
- `input.sha256` pins the raw fixture input file.
- `statement.proof_type` is required because the Shasta verifier address and commitment depend on
  the proof profile.
- `statement.public_input_kind` names the semantic public input being opened.
- `expected.commitment` is the canonical public input commitment.
- `expected.opening` stores enough data to recompute the commitment locally.
- `checks.verify_proof` is not required for all providers.
- provider-specific expectations are optional sidecars, not part of the canonical input.

## Input Payloads

### Shasta Proposal Input

Use existing `GuestInput` as the input payload:

```json
{
  "kind": "shasta_guest_input",
  "path": "test/guest_inputs/shasta/<network>/proposals/proposal_<id>.json",
  "sha256": "0x..."
}
```

The opener should:

1. parse `GuestInput`
2. normalize any known historical fixture gaps, if the current replay path still requires it
3. rebuild `ProofCarryData` for the selected proof type
4. validate existing non-default `proof_carry_data`, when present
5. run Shasta guest input validation
6. compute `hash_shasta_subproof_input`

The result is a `FixtureOpening` and a public input commitment.

### Shasta Aggregate Input

This should be deferred until proposal fixtures and provider public-input conformance are stable.

Aggregate fixtures are still expected later, but they should not be part of the first delivery
because they add:

- child evidence references
- ordered carry-data vectors
- aggregation identity fields
- same-instance TEE rules
- continuity checks across child proofs

Those are real requirements, but they should be handled in a dedicated follow-up phase rather than
inflating the first fixture contract.

## Expected Output Model

The fixture expected output should not be "a proof". It should be a locally openable public input
commitment.

Rust model sketch:

```rust
pub struct FixtureExpected {
    pub commitment: B256,
    pub opening: FixtureOpening,
}

pub struct FixtureStatement {
    pub proof_type: ProofType,
    pub public_input_kind: PublicInputKind,
}

pub enum PublicInputKind {
    ShastaSubproofInput,
}

pub enum FixtureOpening {
    ShastaProofCarryData(ProofCarryData),
}
```

The runner must derive the opening from `GuestInput`, compare it with the stored opening, then
recompute `commitment` from that opening. It should reject fixture envelopes where the stored
opening is stale, where the stored commitment does not match the opening, or where the proof type
selects a different verifier profile.

## Provider Evidence Model

Provider evidence should be checked after the fixture opening is validated.

Logical model:

```rust
pub struct ProviderEvidence {
    pub provider_id: Option<String>,
    pub format: ProviderEvidenceFormat,
    pub public_inputs: ProviderPublicInputs,
    pub proof: Option<serde_json::Value>,
    pub verifier_artifacts: Vec<VerifierArtifact>,
    pub carry_data: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
}
```

Supported formats:

- `legacy_proof`
- `proof_envelope_v1`
- `remote_prover_response_v1`
- `native_mock`

The first required check is:

```text
provider_evidence.public_input == fixture_expected.commitment
```

If a provider returns multiple public inputs, each value must be named by semantic meaning. The
checker should not guess that a backend-specific field is the Shasta commitment.

## Check Modes

### Mode 1: Open Commitment

This mode does not call a provider.

It validates that the fixture input can open the expected commitment:

```text
GuestInput -> ProofCarryData -> commitment
```

This should become the default fast fixture check.

### Mode 2: Public Input Conformance

This mode calls a provider, including remote providers.

It checks:

- the fixture opens locally
- the provider returns a public input commitment
- the provider commitment equals the local expected commitment
- required provider metadata is present, if configured

This mode does not require local ZK proof verification.

### Mode 3: Full Proof Verification

This mode checks the provider proof or attestation against the expected commitment.

Examples:

- SP1 proof verifies against the expected public values and verifying key
- RISC0 receipt verifies against the expected journal and image id
- TEE signature verifies over the expected commitment and matches the accepted signer or enclave
- native mock signature verifies for local tests

This mode should be optional per provider and per test environment.

## Remote Prover Conformance

Remote provers should not be forced directly into full proof verification.

The minimum remote prover flow should be:

```text
load fixture input
open expected commitment locally
send canonical request to remote prover
parse remote response as provider evidence
check claimed public input equals expected commitment
optionally verify proof or attestation
```

This gives third-party providers a clear intake path:

- they can first prove that they use the same public input statement
- then they can enable proof verification when their verifier artifacts are ready
- failures can be diagnosed as input mismatch, metadata mismatch, or proof verification failure

This is safer than accepting opaque proof blobs, and more practical than requiring every verifier
dependency at the first integration step.

The first workstream should focus on proposal proof conformance. Remote aggregate conformance can
be added after the proposal fixture contract is stable.

## Relationship To Existing Proof Types

### Legacy `Proof`

The legacy `Proof` response can be adapted into `ProviderEvidence`.

Mapping:

- `Proof.input` becomes the claimed public input commitment
- `Proof.proof` becomes the proof payload
- `Proof.quote` becomes TEE quote evidence
- `Proof.extra_data.proof_carry_data` can be checked against the fixture opening
- backend-specific metadata remains backend-specific evidence

Legacy `Proof` should remain supported while fixture conformance is introduced.

### `ProofEnvelope`

`ProofEnvelope` should be the preferred provider output contract over time.

It should carry:

- provider id
- named public inputs
- proof payload
- verifier artifacts
- carry data
- metadata

The fixture runner should be able to consume either legacy `Proof` or `ProofEnvelope`, but
`FixtureEnvelope` remains the case definition.

## Fixture Layout

Recommended layout:

```text
test/fixtures/v1/
  shasta/
    taiko_masaya/
      proposals/
        proposal_22758.fixture.json
      suites/
        cornercases.json
```

The large `GuestInput` JSON files can stay in the current `test/guest_inputs/shasta/...` layout.
The fixture envelope references them by repo-relative path.

This avoids duplicating large inputs in each fixture envelope and makes regeneration explicit.

## Suite Manifest

Suite manifests should select fixture envelopes, not raw proposal ids.

Logical shape:

```json
{
  "schema": "raiko2-fixture-suite-v1",
  "suite_id": "shasta/taiko_masaya/cornercases",
  "cases": [
    "shasta/taiko_masaya/proposal_22758"
  ],
  "default_checks": {
    "open_commitment": true,
    "check_provider_public_input": false,
    "verify_proof": false
  }
}
```

The old proposal-id-only suites can be migrated by generating one fixture envelope per proposal.

## CLI Shape

Add a new command rather than overloading benchmark commands:

Generate one fixture envelope from an existing Shasta `GuestInput`:

```bash
cargo run -p xtask -- fixture generate \
  --network taiko_masaya \
  --proposal 22758 \
  --proof-type native
```

Local open-commitment check:

```bash
cargo run -p xtask -- fixture check \
  --case test/fixtures/v1/shasta/taiko_masaya/proposals/proposal_22758.fixture.json \
  --mode open-commitment
```

Provider conformance:

```bash
cargo run -p xtask -- fixture check \
  --case shasta/taiko_masaya/proposal_22758 \
  --mode public-input \
  --provider-url <provider-url>
```

Full verification:

```bash
cargo run -p xtask -- fixture check \
  --case shasta/taiko_masaya/proposal_22758 \
  --mode full-proof \
  --provider sp1.local
```

The existing `replay-guest-input` command can remain as a lower-level replay tool or become a thin
wrapper around `fixture check --mode open-commitment`.

## Report Shape

The runner should emit structured JSON:

```json
{
  "schema": "raiko2-fixture-report-v1",
  "suite_id": "shasta/taiko_masaya/cornercases",
  "results": [
    {
      "case_id": "shasta/taiko_masaya/proposal_22758",
      "mode": "public-input",
      "passed": true,
      "commitment": "0x...",
      "provider_id": "sgx.gaiko2",
      "proof_verified": false,
      "proof_verification_skipped_reason": "not requested"
    }
  ]
}
```

This keeps benchmark reporting separate from conformance reporting, while still allowing future
benchmark runs to reuse the same fixture cases.

## Mutation Suite Relationship

The fixture envelope is the base unit for future mutation tests.

Mutation tests should start from a valid fixture input and apply a named mutation:

- proposal id mismatch
- proposal hash mismatch
- blob commitment mismatch
- tx order mutation
- anchor mutation
- witness node mutation
- proof carry hash mismatch

The expected result for mutations is rejection before provider evidence is accepted.

This workstream should only define the fixture unit and open-commitment checks. Mutation generation
can be a follow-up.

## Recommended Position In The Larger Program

This document should be treated as the first implementation slice under the broader open prover
platform effort.

It sits between:

- provider registry and proof-envelope work, because it needs a stable way to interpret provider
  outputs
- security invariant and conformance work, because it defines the concrete case unit that later
  mutations and provider gates build on

It should not be treated as a separate fourth top-level platform pillar.

## Migration Plan

### Phase 1: Fixture Envelope Types And Opener

- add typed `FixtureEnvelope`, `FixtureExpected`, and `FixtureOpening`
- implement Shasta proposal opener from `GuestInput`
- recompute and validate `hash_shasta_subproof_input`
- produce JSON reports
- generate one Shasta proposal fixture envelope from an existing `GuestInput`
- do not call providers yet

### Phase 2: Generate Envelopes For Existing Guest Inputs

- bulk-generate fixture envelopes for current Shasta proposal inputs
- preserve existing large `GuestInput` files
- migrate suite manifests from proposal ids to case ids
- keep old `replay-guest-input` behavior available during migration

### Phase 3: Provider Public Input Check

- adapt legacy `Proof` into `ProviderEvidence`
- call local or remote providers
- check provider public input equals local expected commitment
- fail clearly on missing or mismatched public input

### Phase 4: Optional Proof Verification

- add native mock verifier first
- add TEE signature and identity checks where local policy exists
- add SP1 and RISC0 verifier checks behind features or provider capabilities
- keep `verify_proof = skipped` as an explicit result, not a silent pass

### Phase 5: Aggregate Fixtures

This should be a follow-up phase, not part of the first implementation slice.

Later scope:

- add aggregate fixture envelope shape
- derive `Commitment` from child openings
- require same-instance checks for providers that need them
- replace placeholder aggregate proof fixtures with real provider-generated child evidence

## Compatibility Notes

- Existing `GuestInput` fixtures remain valid input payloads.
- Existing remote prover wire goldens remain useful request compatibility tests.
- Existing proof response tests may continue to use mock proof blobs, but conformance tests should
  not use placeholder public input commitments.
- Fixture-backed mock RISC0 and SP1 paths should eventually return the real Shasta subproof input
  commitment instead of a zero placeholder when used by conformance tests.

## Open Questions

- Should fixture envelopes store the full opening inline, or only store the expected commitment and
  recompute the opening from `GuestInput` every time?
- Should `input.sha256` use raw JSON bytes or canonical JSON bytes?
- Should the first implementation extend `replay-guest-input` or add a new `fixture` command group?
- Should remote prover conformance consume HTTP responses directly, or require providers to dump a
  provider evidence JSON file first?
- Which proof verification modes must be mandatory before third-party provider admission?
- Should aggregate fixture evidence always be generated live from the same provider instance, or can
  it be stored as a signed provider evidence artifact?
