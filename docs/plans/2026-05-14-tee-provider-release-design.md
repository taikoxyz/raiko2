# TEE Provider Release Design

## Goal

Define a repeatable pre-release flow for TEE-backed remote prover images that matches the release
discipline already used for zk guest digests.

The flow must cover:

- building release-candidate TEE provider images from pinned source inputs
- pushing those images and recording immutable repo digests
- extracting `mr_enclave` and `mr_signer` from each pushed image
- emitting one machine-readable attestation manifest that can be handed to the operator who updates
  on-chain verifier allowlists

This flow intentionally stops before bootstrap, quote registration, and on-chain configuration.

## Scope

This design introduces a release orchestrator for TEE providers and the release metadata it emits.

It covers:

- `raiko2-sgx` built from the current `raiko2` checkout
- external TEE providers such as `gaiko2-sgxgeth`, built from an explicitly pinned external commit
- future TEE provider families such as `tdx`

It does not cover:

- instance bootstrap or quote generation
- `setMrEnclave` / `registerSgxInstanceWithQuoteBytes` execution
- rollout or deployment
- replacing the existing zk `guest-digests` or `release-image` flows

## Problem

Today the repository has:

- a zk release flow that records guest digests
- a runtime image release flow that records pushed image digests
- build-time SGX attestation metadata for `raiko2-sgx`

But it does not yet have a single release-time command that says:

1. which TEE provider sources were used
2. which images were pushed
3. which `mr_enclave` / `mr_signer` values correspond to those pushed images

That gap becomes more important now that:

- `raiko2-sgx` and `gaiko2-sgxgeth` are separate provider images
- external providers should be pinned to exact source commits
- future providers such as `tdx` should follow the same release contract

## Decision

Add a new `xtask` command:

- `release-tee-providers`

This command becomes the TEE analogue of the existing zk release/export flow.

It should:

1. verify the local `raiko2` worktree is clean
2. read a checked-in provider lock file
3. build and push the local `raiko2-sgx` image
4. clone each external provider repo to a temporary release workspace
5. check out the exact pinned commit for that provider
6. build and push the external provider image
7. resolve the pushed repo digest for every image
8. read the attestation metadata from every built image
9. emit a unified TEE attestation manifest

This keeps TEE release provenance explicit and auditable without coupling day-to-day development to
provider source trees.

## Recommended Shape

### Checked-In Provider Lock File

Store pinned external provider release inputs in:

- `release/providers.toml`

Each provider entry should include:

- `repo`
- `commit`
- `provider`
- `lane`
- `image_name`
- `repository`
- `dockerfile`
- `context`
- `attestation_path`

Example:

```toml
[providers.gaiko2]
repo = "https://github.com/taikoxyz/gaiko2.git"
commit = "abcdef1234567890abcdef1234567890abcdef12"
provider = "gaiko2-sgxgeth"
lane = "sgxgeth"
image_name = "gaiko2-sgxgeth"
repository = "us-docker.pkg.dev/evmchain/images/gaiko2-sgxgeth"
dockerfile = "docker/Dockerfile.tee"
context = "."
attestation_path = "/opt/gaiko2/etc/attestation.gaiko2.json"
```

This file is the source of truth for which external provider commit is eligible for release.

### Local Provider Defaults

`raiko2-sgx` should not need a lock entry for its source repo because it is built from the current
clean checkout.

Its release config can be hard-coded in the `xtask` command as the local TEE provider entry:

- `provider = "raiko2-sgx"`
- `lane = "sgx"`
- `repository = "us-docker.pkg.dev/evmchain/images/raiko2-sgx"`
- `dockerfile = "Dockerfile.sgx"`
- `attestation_path = "/opt/raiko2-sgx/etc/attestation.raiko2.json"`

### Release Command Surface

The primary command should be:

```bash
cargo run -r -p xtask -- release-tee-providers --tag <tag>
```

Recommended optional flag:

```bash
--no-push
```

This allows a local smoke run that:

- builds provider images
- reads attestation metadata
- verifies manifest shape

without pushing images or claiming release readiness.

### Pinned Provider Updates

Add a companion command for explicit provider pin changes:

```bash
cargo run -r -p xtask -- update-tee-provider-lock <provider> --commit <sha>
```

Future support may add tag-based resolution, but release execution should always consume the exact
checked-in commit from `release/providers.toml`.

## Why Not Use A Submodule

Submodules and checked-in lock files both pin exact commits.

The main reason to prefer the lock file shape here is not review diff size. A submodule pointer and
an explicit commit field are both small diffs.

The real advantage is workflow isolation:

- ordinary `raiko2` development does not need provider source trees present
- release-only dependency orchestration stays inside `xtask`
- future providers can join the release flow without becoming a permanent part of the repo working
  tree

This keeps provider pinning explicit while avoiding tighter day-to-day repository coupling.

## Build Ownership

The `raiko2` release orchestrator owns the release build of all TEE provider images.

That means:

- `raiko2-sgx` is built from the current `raiko2` checkout
- external providers are cloned into a temporary release workspace and built there

Recommended temporary workspace shape:

- `target/tee-release/<tag>/sources/<provider>/`

This mirrors the old `raiko` release flow in spirit:

- one release entrypoint
- multiple provider builds
- one final metadata artifact

without requiring provider repos to be submodules.

## Manifest Shape

The release output should be a single JSON document:

- `target/releases/<tag>/tee-attestation-manifest-<tag>.json`

Recommended shape:

```json
{
  "release": "vX.Y.Z-rc1",
  "generated_at": "2026-05-14T12:34:56Z",
  "providers": [
    {
      "lane": "sgx",
      "provider": "raiko2-sgx",
      "source": {
        "repo": "local",
        "commit": "4e4378c..."
      },
      "image": {
        "repository": "us-docker.pkg.dev/evmchain/images/raiko2-sgx",
        "tag": "vX.Y.Z-rc1",
        "digest": "us-docker.pkg.dev/evmchain/images/raiko2-sgx@sha256:..."
      },
      "attestation": {
        "mr_enclave": "...",
        "mr_signer": "...",
        "isv_prod_id": 0,
        "isv_svn": 0,
        "debug_enclave": false
      }
    },
    {
      "lane": "sgxgeth",
      "provider": "gaiko2-sgxgeth",
      "source": {
        "repo": "https://github.com/taikoxyz/gaiko2.git",
        "commit": "abcdef123..."
      },
      "image": {
        "repository": "us-docker.pkg.dev/evmchain/images/gaiko2-sgxgeth",
        "tag": "vX.Y.Z-rc1",
        "digest": "us-docker.pkg.dev/evmchain/images/gaiko2-sgxgeth@sha256:..."
      },
      "attestation": {
        "mr_enclave": "...",
        "mr_signer": "..."
      }
    }
  ]
}
```

Key properties:

- one release artifact covers all TEE provider images
- each provider records both source provenance and pushed image provenance
- the operator configuring the verifier can rely on immutable digests, not just tags

## Failure Policy

`release-tee-providers` should fail fast and avoid half-valid outputs.

The command must abort if any provider fails to:

- confirm target Artifact Registry Docker repositories enforce immutable tags for pushed runs
- clone or check out the pinned commit
- build
- push
- resolve a pushed digest
- expose a readable attestation metadata file
- provide non-empty `mr_enclave` / `mr_signer`

The final manifest should only be written once every provider entry is complete and validated.

## Validation Strategy

Implementation validation should cover two levels:

### Local Smoke

Use:

```bash
cargo run -r -p xtask -- release-tee-providers --tag <tag> --no-push
```

This should verify:

- provider builds succeed
- attestation metadata can be read for every provider
- output manifest parses and has the expected shape

### Release Path

Use:

```bash
cargo run -r -p xtask -- release-tee-providers --tag <tag>
```

This should verify:

- images are pushed successfully
- immutable repo digests are captured
- the final manifest records both digest and attestation data

## Documentation Changes

The implementation should update:

- `docs/operations.md`

to document the TEE pre-release flow as a peer to:

- runtime image release
- zk guest digest export

The existing release skill surface should also be updated so agents understand that TEE provider
release metadata is a separate, explicit release artifact.

## Non-Goals

- replacing the existing `release-image` command
- moving on-chain verifier configuration into `xtask`
- running bootstrap or quote registration automatically
- inferring provider versions from branch tips at release time
- collapsing multiple provider images into a single enclave measurement
