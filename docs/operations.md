# Operations Guide

This guide covers runtime configuration, Docker, SGX operation, and Boundless operation.
API contracts live in [API.md](API.md), and the canonical config shape lives in
[`config.example.toml`](../config.example.toml).

See also:

- [Docs index](README.md)
- [README](../README.md) for the project overview
- [Development guide](development.md) for local workflows and guest tooling

## Run the Server

`config.example.toml` is a combined production sample. Before running, edit the copy to keep only
the desired per-proof-type tables enabled and fill every setting, credential, and endpoint required
by those backends.

Run the server with the edited config file:

```bash
cp config.example.toml config.toml
$EDITOR config.toml
./target/release/raiko2 --config config.toml
```

Or select the config path through the environment:

```bash
RAIKO2_CONFIG=./config.toml ./target/release/raiko2
```

CLI flags and explicitly supported environment variables override values loaded from the file.
`--l1-rpc` and `--l2-rpc` remain available as overrides, but they only apply when the config
defines exactly one `rpc.pairs` entry.

When `--l2-rpc` overrides a single configured pair, it updates only `rpc.pairs[*].l2_rpc`.
Configured `rpc.pairs[*].l2_witness_rpc` values are preserved; unset witness RPCs continue to
fall back to the effective `l2_rpc`.

## Docker Compose

The Docker path uses:

- [`Dockerfile`](../Dockerfile) for the runtime image
- [`docker/docker-compose.yml`](../docker/docker-compose.yml) for orchestration
- [`docker/config.compose.toml`](../docker/config.compose.toml) for the mounted base config
- [`docker/.env.sample`](../docker/.env.sample) as the operator-facing environment template

Quickstart:

```bash
cp docker/.env.sample docker/.env
$EDITOR docker/.env
docker compose --env-file docker/.env -f docker/docker-compose.yml up --build
```

The default compose stack runs a single `raiko2` container on port `8080` with the in-process
memory queue. Default binaries include RISC Zero local/network proving and SP1 proving. The
operator env sample selects `risc0/local` for this one-container quickstart.

To select proving routes, set `RAIKO2_PROVER_ROUTES` in `docker/.env` to a comma-separated list.
It atomically replaces all proof-type enablement and updates the selected RISC0/SP1 execution
selectors; it does not append to the mounted config.
For a single local route:

```dotenv
RAIKO2_PROVER_ROUTES=risc0/local
```

Do not apply the four-route production value to `docker/config.compose.toml` as-is. That file is
the SGX compose shape: its SP1 settings select the local prover, and its Boundless signer is only a
placeholder.

For a production host serving both network ZK systems and both SGX lanes, start from the complete
production-oriented example instead:

```bash
cp config.example.toml config.toml
$EDITOR config.toml
```

In that edit, replace placeholder credentials, configure production RPC and runtime storage, and
verify that each enabled backend section matches its route. In particular,
`[prover.sp1]` must select the network prover, `[prover.risc0.boundless]` must contain real
deployment credentials, and `[prover.sgx]` / `[prover.sgxgeth]` must point to their production
services. The atomic override may then be passed to that completed config:

```bash
RAIKO2_PROVER_ROUTES=risc0/network,sp1/network,sgx/remote,sgxgeth/remote \
  ./target/release/raiko2 --config ./config.toml
```

Supported pairs are `risc0/local`, `risc0/network`, `sp1/local`, `sp1/network`, `native/local`,
`sgx/remote`, and `sgxgeth/remote`. Without an override, each proof-type table owns its own
`enabled` state and execution selector. One host may explicitly enable any supported combination.

The queue is always in-process. Durable task state and remote-provider checkpoints use the
configured namespaced GCS runtime store; Boundless does not need an extra feature flag.

## Hosted Aggregate Route

The hosted API accepts external proposal proofs through:

```http
POST /v3/proof/aggregate
POST /proof/aggregate
```

This route expects already-produced proposal proofs and registers an aggregation task directly.
It does not support `proof_type = "zk_any"`.

Operational notes:

- `proof_type = "sp1"` requires proofs that include the SP1 aggregation metadata expected by the
  canonical aggregation path. Hosted SP1 batch proposal proving emits Compressed proofs, and the
  hosted aggregation route emits a Plonk proof.
- `proof_type = "risc0"` uses the same hosted RISC0 network prover path as proposal proving.
- The request body is intentionally aligned with old `raiko`'s global body-limit posture; do not
  widen it ad hoc at the route level.

## SGX Runtime

The `sgx` lane is operated as a separate remote service built from this repository:

- binary: `raiko2-sgx-prover`
- image: [`Dockerfile.sgx`](../Dockerfile.sgx)
- compose: [`docker/docker-compose.sgx.yml`](../docker/docker-compose.sgx.yml)
- env template: [`docker/.env.sgx.sample`](../docker/.env.sgx.sample)

The SGX binary exposes:

- `bootstrap`
- `check`
- `serve`

Serving endpoints are:

- `GET /health`
- `POST /prove/shasta`
- `POST /prove/shasta-aggregate`

`bootstrap` and `check` are CLI lifecycle commands, not HTTP routes.

The binary supports two runtime modes:

- `tee` (default): Gramine-backed SGX mode with quote generation
- `native`: operator/testing mode that reuses the fixed native signer, omits quotes, and keeps the
  same remote HTTP surface

### Local CLI

Build and inspect the binary:

```bash
cargo run -r -p raiko2-sgx-prover -- --help
```

Bootstrap the SGX runtime:

```bash
cargo run -r -p raiko2-sgx-prover -- \
  --mode tee \
  --config-dir ~/.config/raiko2/sgx/config \
  --secret-dir ~/.config/raiko2/sgx/secrets \
  bootstrap
```

Check the lifecycle state:

```bash
cargo run -r -p raiko2-sgx-prover -- \
  --mode tee \
  --config-dir ~/.config/raiko2/sgx/config \
  --secret-dir ~/.config/raiko2/sgx/secrets \
  check
```

Run the SGX sign server:

```bash
cargo run -r -p raiko2-sgx-prover -- \
  --mode tee \
  --config-dir ~/.config/raiko2/sgx/config \
  --secret-dir ~/.config/raiko2/sgx/secrets \
  serve --listen-addr 0.0.0.0:8080 --instance-id 3131899904
```

If `--instance-id` is omitted, `serve` resolves it from `registered.json` using `--fork`.

For operator/link testing without SGX, start the same remote surface in native mode:

```bash
cargo run -r -p raiko2-sgx-prover -- \
  --mode native \
  serve --listen-addr 0.0.0.0:8080
```

Native mode treats `bootstrap` as a no-op, `check` as a lightweight no-op, and falls back to the
mock instance id `0xDEAD_C0DE` when `--instance-id` is omitted.

Use `GET /health` for simple liveness checks. `POST /prove/shasta` is not a lightweight signing
smoke endpoint: `raiko2-sgx-prover` requires a complete `GuestInput` request envelope and runs the
same Shasta guest validation path as the zk guests before signing the resulting public input.
Use the main `raiko2` service or the regression scripts to build that request.

### Docker Compose

Quickstart:

```bash
cp docker/.env.sgx.sample docker/.env.sgx
$EDITOR docker/.env.sgx
docker compose --env-file docker/.env.sgx -f docker/docker-compose.sgx.yml --profile init up raiko2-sgx-init
docker compose --env-file docker/.env.sgx -f docker/docker-compose.sgx.yml up raiko2-sgx
```

To build the EDMM variant without replacing or confusing it with the default local image, use a
distinct local tag and force the service build:

```bash
SGX_EDMM_ENABLE=true \
RAIKO2_SGX_IMAGE=raiko2-sgx:local-edmm \
docker compose --env-file docker/.env.sgx -f docker/docker-compose.sgx.yml build raiko2-sgx

RAIKO2_SGX_IMAGE=raiko2-sgx:local-edmm \
docker compose --env-file docker/.env.sgx -f docker/docker-compose.sgx.yml \
  --profile init up --no-build raiko2-sgx-init

RAIKO2_SGX_IMAGE=raiko2-sgx:local-edmm \
docker compose --env-file docker/.env.sgx -f docker/docker-compose.sgx.yml \
  up --no-build raiko2-sgx
```

Operator notes:

- The compose stack mounts SGX devices and passes the enclave signing key as a build secret.
- `Dockerfile.sgx` and the local Compose stacks default to a non-EDMM enclave for compatibility
  with hosts that do not support EDMM. Set `SGX_EDMM_ENABLE=true` in the Compose env file to build
  an EDMM-enabled local image explicitly.
- Set `RAIKO2_SGX_ENCLAVE_KEY_HOST` to a local Gramine enclave signing key. Local builds must not
  claim the Taiko release `mr_signer`; official Taiko-signed provider images are built by the
  protected GitHub Actions release workflow, which fetches the signing key through Workload Identity
  Federation and GCP Secret Manager. Do not commit signing keys.
- `raiko2-sgx-init` is a one-shot bootstrap job.
- `raiko2-sgx` is the long-running sign server.
- The SGX image is signed during `Dockerfile.sgx` build, and tee startup reuses the baked
  manifest/signature artifacts.
- Set `RAIKO2_SGX_MODE=native` to bypass Gramine for operator/link testing while keeping the same
  `/prove/shasta*` server behavior.
- This compose file is still SGX-oriented and mounts SGX devices by default. For native-mode runs
  on a host without SGX devices, run the binary directly or trim the device mounts locally.
- Either set `RAIKO2_SGX_INSTANCE_ID` directly or provide a `registered.json` mapping under the
  mounted config directory and select it with `RAIKO2_SGX_FORK`.
- This compose file only covers the `sgx` lane. `sgxgeth` is served by external geth-backed
  remote SGX infrastructure and is not built in this repository.

Migration warning: before SGX image variants were introduced, `Dockerfile.sgx` hardcoded
`sgx.edmm_enable = true`. The unsuffixed release image and the local Compose stacks now default to
non-EDMM.
Operators retaining the previous EDMM behavior must select the `<release>-edmm` image or set
`SGX_EDMM_ENABLE=true` for local builds. Changing variants changes `MRENCLAVE`; verifier
registration and image selection must use the measurement for the selected variant.

Read the baked SGX measurement from:

```bash
docker run --rm --entrypoint cat \
  raiko2-sgx:local \
  /opt/raiko2-sgx/etc/attestation.raiko2.json
```

`attestation.raiko2.json` is the operator-facing source for `mr_enclave` on the `raiko2-sgx`
image. Use that value with your external SGX verifier registration flow; this repository does not
apply SGX attester registration onchain.

## SGX Regression Stack

For SGX regression work, use the unified compose stack:

- [`docker/docker-compose.sgx.regression.yml`](../docker/docker-compose.sgx.regression.yml)
- [`docker/.env.sgx.regression.sample`](../docker/.env.sgx.regression.sample)

This stack is intentionally SGX-focused:

- it starts `raiko2-sgx-prover` for the `sgx` lane
- it starts an external geth-backed remote SGX tee image for the `sgxgeth` lane
- it can optionally start the `raiko2` main service under the `raiko2` profile
- it does not build `../gaiko2` automatically; operators should pre-build or pull the
  `GAIKO2_SGXGETH_IMAGE`

Bootstrap both tee services:

```bash
cp docker/.env.sgx.regression.sample docker/.env.sgx.regression
docker compose --env-file docker/.env.sgx.regression -f docker/docker-compose.sgx.regression.yml --profile init up raiko2-sgx-init gaiko2-sgxgeth-init
```

Start the two remote SGX services:

```bash
docker compose --env-file docker/.env.sgx.regression -f docker/docker-compose.sgx.regression.yml up -d
```

Add the optional dockerized `raiko2` main service:

```bash
docker compose --env-file docker/.env.sgx.regression -f docker/docker-compose.sgx.regression.yml --profile raiko2 up -d raiko2
```

The optional dockerized `raiko2` service is prewired with both remote SGX URLs:

- `proof_type=sgx` uses `RAIKO2_REMOTE_SGX_BASE_URL`
- `proof_type=sgxgeth` uses `RAIKO2_REMOTE_SGX_SGXGETH_BASE_URL`

The regression env sample pins fixed `RAIKO2_SGX_INSTANCE_ID` and `GAIKO2_INSTANCE_ID` values so
both SGX lanes can boot and prove without an onchain registration step. Replace them with the real
registered instance ids, or mount the corresponding registration metadata, when you need
chain-verifiable SGX proofs.

For a local `raiko2` CLI against the compose-managed SGX servers:

```bash
RAIKO2_CONFIG=docker/config.compose.toml \
RAIKO2_PROVER_ROUTES=sgx/remote,sgxgeth/remote \
RAIKO2_L1_RPC=http://127.0.0.1:8545 \
RAIKO2_L2_RPC=http://127.0.0.1:9545 \
RAIKO2_REMOTE_SGX_BASE_URL=http://127.0.0.1:9090 \
RAIKO2_REMOTE_SGX_SGXGETH_BASE_URL=http://127.0.0.1:8090 \
cargo run -r -p raiko2 -- --config docker/config.compose.toml
```

Then choose the lane per request:

- `proof_type=sgx` targets `raiko2-sgx-prover`
- `proof_type=sgxgeth` targets the external geth-backed remote SGX server

### Main-Service Wiring

`raiko2` keeps the SGX path as a remote route. The dedicated `raiko2-sgx-prover` binary is the
runtime for `sgx`. Historical `sgxgeth` compatibility is expected to come from an external
`gaiko2` SGX service.

## Source Releases

Use this flow as the ZK/runtime portion of a versioned release such as `vX.Y.Z`. The commands for
TEE provider metadata remain separate, but the current default full release runs both flows and
combines their manifests and release-note sections. A ZK-only release must be explicitly scoped as
such and must not claim to contain TEE provider metadata.

Release prerequisites:

- start from a clean checkout of `main`
- choose a concrete release commit SHA
- publish one runtime image that includes both guest backends:
  - `risc0`
  - `sp1`
- export guest digests from the checked-in ELFs
- create both:
  - a human-readable release notes file
  - a machine-readable release manifest
- include human-readable ZK guest digests in the release notes:
  - `risc0` proposal and aggregation `image_id`
  - `sp1` proposal and aggregation `vk_bn254`
  - `sp1` proposal and aggregation `vk_hash_bytes`

Suggested local variables:

```bash
export VERSION=0.1.0
export TAG=v0.1.0
export RELEASE_SHA=$(git rev-parse HEAD)
export RELEASE_DIR=/tmp/raiko2-release-${TAG}
mkdir -p "${RELEASE_DIR}"
```

Recommended sequence:

1. Confirm the release commit:

   ```bash
   git checkout main
   git pull --ff-only origin main
   git status --short
   git rev-parse HEAD
   ```

2. Start the official Taiko-signed TEE provider image workflow and the local runtime image
   publication from the same release commit. These two publication paths can run in parallel after
   `RELEASE_SHA` is frozen, but both must finish successfully before release notes, the git tag, or
   the GitHub Release are created.

   Start the TEE provider workflow:

   ```bash
   gh workflow run release-tee-providers.yml \
     --repo taikoxyz/raiko2 \
     --ref main \
     -f tag="${TAG}"
   ```

   In a separate clean checkout at `${RELEASE_SHA}`, publish the runtime image:

   ```bash
   git rev-parse HEAD
   test "$(git rev-parse HEAD)" = "${RELEASE_SHA}"
   git status --short

   just release-image all ${TAG}
   ```

   Record the immutable runtime image digest reference printed by `release-image`:

   - `us-docker.pkg.dev/evmchain/images/raiko2@sha256:...`

   After the TEE workflow is approved and finishes, verify it used the same release commit and
   download the manifest:

   ```bash
   # Record TEE_RUN_ID from the Actions URL printed by the command above, wait for
   # `sgx-release-signing` approval, then verify the workflow used the frozen release commit.
   export TEE_RUN_ID=<run-id-from-actions-url>
   gh run watch "${TEE_RUN_ID}" --repo taikoxyz/raiko2 --exit-status
   test "$(gh run view "${TEE_RUN_ID}" --repo taikoxyz/raiko2 --json headSha --jq .headSha)" \
     = "${RELEASE_SHA}"

   gh run download "${TEE_RUN_ID}" \
     --repo taikoxyz/raiko2 \
     --name "tee-attestation-manifest-${TAG}" \
     --dir "${RELEASE_DIR}"

   export TEE_MANIFEST="${RELEASE_DIR}/tee-attestation-manifest-${TAG}/tee-attestation-manifest-${TAG}.json"
   test -s "${TEE_MANIFEST}"
   ```

   This must produce a workflow artifact at `${TEE_MANIFEST}`. Record the immutable image digests
   and attestation values from that manifest. The protected workflow validates both local SGX
   variants before publishing their final tags. A local `release-tee-providers` run can validate
   `mr_enclave` reproducibility with a disposable key, but it cannot produce the official Taiko
   `mr_signer`.

3. Export guest digests:

   ```bash
   cargo run -r -p xtask-build-guest --bin guest-digests --features digests -- \
     --output "${RELEASE_DIR}/guest-digests-summary.json"
   ```

4. Build the release manifest:

   ```bash
   python3 scripts/release/write_release_manifest.py \
     --version "${VERSION}" \
     --tag "${TAG}" \
     --git-sha "${RELEASE_SHA}" \
     --image-tag "${TAG}" \
     --image-digest-ref "us-docker.pkg.dev/evmchain/images/raiko2@sha256:..." \
     --guest-digests "${RELEASE_DIR}/guest-digests-summary.json" \
     --output "${RELEASE_DIR}/release-manifest-${TAG}.json"
   ```

5. Write release notes from the ZK source release template, then append the TEE Provider Release
   Notes Template below for the default full profile. The final notes must include both reproduce
   sections.

   ```bash
   cat > "${RELEASE_DIR}/release-notes-${TAG}.md" <<'EOF'
## Summary

- ZK/runtime source release summary.

## Runtime Image

- runtime image: us-docker.pkg.dev/evmchain/images/raiko2@sha256:...
- commit: <release commit SHA>
- includes `risc0` guest ELFs and `sp1` guest ELF/VK artifacts

## ZK Guest Digests

   - risc0 proposal image_id: 0x...
   - risc0 aggregation image_id: 0x...
   - sp1 proposal vk_bn254: 0x...
   - sp1 proposal vk_hash_bytes: 0x...
- sp1 aggregation vk_bn254: 0x...
- sp1 aggregation vk_hash_bytes: 0x...

## Reproduce

See `docs/operations.md#reproduce-zk-guest-digests`.

## Release Assets

- `release-manifest-vX.Y.Z.json`
- `guest-digests-summary.json`
- `tee-attestation-manifest-vX.Y.Z.json` (full profile)
- `risc0_shasta_*.elf`
- `sp1_shasta_*.elf`
- `sp1_shasta_*.vk.bin`
EOF
```

6. Create the tag and GitHub Release:

   ```bash
   git tag "${TAG}" "${RELEASE_SHA}"
   git push origin "${TAG}"

   # Add `--prerelease` for release candidates such as `vX.Y.Z-rcN`.
   gh release create "${TAG}" \
     --target "${RELEASE_SHA}" \
     --title "${TAG}" \
     --notes-file "${RELEASE_DIR}/release-notes-${TAG}.md" \
     "${RELEASE_DIR}/release-manifest-${TAG}.json" \
     "${RELEASE_DIR}/guest-digests-summary.json" \
     "${TEE_MANIFEST}" \
     crates/guests/elf/risc0_shasta_*.elf \
     crates/guests/elf/sp1_shasta_*.elf \
     crates/guests/elf/sp1_shasta_*.vk.bin
   ```

Expected release outputs:

- git tag: `${TAG}`
- runtime image tag: `${TAG}`
- release notes file: `release-notes-${TAG}.md`
- release manifest file: `release-manifest-${TAG}.json`
- guest digest export file: `guest-digests-summary.json`
- TEE attestation manifest file: `tee-attestation-manifest-${TAG}.json` (full profile)
- Shasta guest artifact assets:
  - `risc0_shasta_*.elf`
  - `sp1_shasta_*.elf`
  - `sp1_shasta_*.vk.bin`

Do not:

- apply `register-image` automatically as part of the release cut
- mix rollout or deployment steps into the release flow
- mix TEE provider metadata into a ZK/runtime-only release
- write release-only metadata back into the source tree

### Reproduce ZK Guest Digests

Use this when GitHub release notes need a stable reference for how ZK digest
values were regenerated. This compares the published release digest asset with
digests recomputed from the tag checkout.

```bash
export TAG=vX.Y.Z
export REPRO_DIR=target/releases/${TAG}/zk-digest-repro

git fetch --tags origin "${TAG}"
git checkout "${TAG}"
mkdir -p "${REPRO_DIR}"

just build-guest all --force

cargo run -r -p xtask-build-guest --bin guest-digests --features digests -- \
  --output "${REPRO_DIR}/from-source.json"

gh release download "${TAG}" --repo taikoxyz/raiko2 \
  --pattern guest-digests-summary.json \
  --dir "${REPRO_DIR}" \
  --clobber

jq -S '.digests | sort_by(.proof_system, .object_name, .stage, .digest_source)' \
  "${REPRO_DIR}/guest-digests-summary.json" > "${REPRO_DIR}/release-digests.sorted.json"
jq -S '.digests | sort_by(.proof_system, .object_name, .stage, .digest_source)' \
  "${REPRO_DIR}/from-source.json" > "${REPRO_DIR}/source-digests.sorted.json"
diff -u "${REPRO_DIR}/release-digests.sorted.json" "${REPRO_DIR}/source-digests.sorted.json"
```

Print the values for the release notes:

```bash
jq -r '.digests[]
  | [.proof_system, .object_name, .stage, .digest_source, .digest]
  | @tsv' "${REPRO_DIR}/from-source.json"
```

## Release Images

Use the `xtask` release entrypoint for runtime images. It refreshes selected guest ELFs before
building non-host runtime images unless `--skip-guest-refresh` is explicitly set.

```bash
just release-image risc0 release-20260507-1013
```

Direct `xtask` entrypoint:

```bash
cargo run -r -p xtask -- release-image risc0 \
  --tag release-20260507-1013 \
  --repository us-docker.pkg.dev/evmchain/images/raiko2
```

`release-image host` automatically builds the host-only feature set. Runtime image builds use
Cargo's default release profile unless the caller explicitly overrides Cargo profile environment
variables.
The root runtime `Dockerfile` uses BuildKit cache mounts, `sccache`, and `mold` by default, and
`release-image` prints the Docker build elapsed time before pushing so release logs can compare
cold and warm cache runs.

Avoid ad-hoc `docker build` for releases. The runtime image packages the existing
`crates/guests/elf` artifacts at `/app/crates/guests/elf`; `raiko2` loads those files when the
process starts and does not rebuild guest sources by itself. The image sets
`RAIKO2_GUEST_ELF_DIR=/app/crates/guests/elf` so ELF lookup does not depend on the container
working directory.

For unreleased testing, build local ELFs with `just build-guest all` before building the image. For
released artifacts, download guest ELF/VK assets from GitHub Releases with
`cargo run -r -p xtask -- download-guest-elves --tag <tag> --backend all --dir crates/guests/elf`.
`release-image` refreshes guest ELFs for the selected non-host backend by default.
Guest builds and refreshes skip unchanged backends by fingerprint unless `--force` or
`--force-rebuild-guests` is used. Logs include backend elapsed time, and the repo-managed Docker
toolchain image path uses persistent Cargo and `sccache` volumes by default. Custom toolchain images
must opt in with `DOCKER_SCCACHE_CACHE=volume` or `DOCKER_SCCACHE_CACHE_VOLUME=<volume>` so images
without `sccache` keep working. RISC0 and SP1 rebuild logs print `sccache --show-stats`, so release
logs expose cache hit/miss counts as well as wall time. Use `DOCKER_CARGO_CACHE=none` or
`DOCKER_SCCACHE_CACHE=none` only for diagnostics; disabling either cache should not be needed for
normal releases.
If refresh leaves tracked guest ELF artifacts dirty, it stops before publishing; review
and commit the updated `crates/guests/elf` artifacts, then rerun the release command so image
provenance still matches the committed repo state.

### Compose A Host Image With Released Guest Artifacts

Use this SOP when a selected host revision must package the complete RISC0 and SP1 artifact set from
an existing raiko2 release. This creates a source branch whose commit records both sides of the
composition. It publishes an image only; it does not register verifier digests, deploy the image, or
perform a Kubernetes rollout.

#### 1. Freeze The Inputs

Run from a raiko2 checkout. Replace the example tag before continuing:

```bash
set -Eeuo pipefail

export HOST_REF=origin/main
export ELF_TAG=vX.Y.Z
export IMAGE_REPOSITORY=us-docker.pkg.dev/evmchain/images/raiko2
export AR_PROJECT=evmchain
export AR_LOCATION=us
export AR_REPOSITORY=images

test "${ELF_TAG}" != "vX.Y.Z"
test "${IMAGE_REPOSITORY}" = \
  "${AR_LOCATION}-docker.pkg.dev/${AR_PROJECT}/${AR_REPOSITORY}/raiko2"

export ELF_RELEASE_REF HOST_SHA ELF_SHA HOST_SHORT COMPOSE_BRANCH COMPOSE_WORKTREE IMAGE_TAG
ELF_RELEASE_REF="refs/raiko2-release-tags/${ELF_TAG}"

git fetch --no-tags origin main
git fetch --no-tags origin \
  "+refs/tags/${ELF_TAG}:${ELF_RELEASE_REF}"

HOST_SHA=$(git rev-parse "${HOST_REF}^{commit}")
ELF_SHA=$(git rev-parse "${ELF_RELEASE_REF}^{commit}")
HOST_SHORT=${HOST_SHA:0:12}
COMPOSE_BRANCH="main-${HOST_SHORT}-elf-${ELF_TAG}"
COMPOSE_WORKTREE="../raiko2-${COMPOSE_BRANCH}"
IMAGE_TAG="${COMPOSE_BRANCH}"

test "$(gh release view "${ELF_TAG}" --repo taikoxyz/raiko2 \
  --json isDraft --jq '.isDraft')" = "false"
```

The selected remote tag is fetched into a dedicated local ref so unrelated local tag conflicts cannot
abort the operation. Stop if the host revision, selected release tag, or non-draft GitHub Release
cannot be resolved. Use a new branch and image tag; do not overwrite an existing publication.

#### 2. Create The Composition Branch

```bash
git worktree add -b "${COMPOSE_BRANCH}" "${COMPOSE_WORKTREE}" "${HOST_SHA}"
cd "${COMPOSE_WORKTREE}"

git restore --source="${ELF_SHA}" --staged --worktree -- crates/guests/elf

# The complete directory, including provenance, must equal the release tag.
git diff --exit-code "${ELF_SHA}" -- crates/guests/elf

# Nothing outside the artifact directory may be staged.
git diff --cached --quiet -- . ':(exclude,glob)crates/guests/elf/**'
git diff --cached --check
```

Restore the whole directory. Do not combine one release's RISC0 ELF with another release's SP1
ELF/VK, omit lab artifacts or provenance manifests, or regenerate provenance against old binaries.
Provenance records the source fingerprint and artifact hashes that produced that release; raiko2
does not consume it as a runtime trust anchor.

#### 3. Verify Release Artifact Identity

Derive the expected Shasta asset names from the release tag, require the GitHub Release to publish
that exact set, then download and compare every byte with the tag checkout:

```bash
mkdir -p target/release-guest-verification
export VERIFY_ROOT VERIFY_DIR EXPECTED_RELEASE_ASSETS ACTUAL_RELEASE_ASSETS
VERIFY_ROOT=$(mktemp -d "target/release-guest-verification/${ELF_TAG}.XXXXXXXX")
VERIFY_DIR="${VERIFY_ROOT}/downloads"
EXPECTED_RELEASE_ASSETS="${VERIFY_ROOT}/release-assets.expected"
ACTUAL_RELEASE_ASSETS="${VERIFY_ROOT}/release-assets.actual"
mkdir -p "${VERIFY_DIR}"

git ls-tree -r --name-only "${ELF_SHA}" -- crates/guests/elf \
  | grep -E \
    '^crates/guests/elf/(risc0_shasta_.*\.elf|sp1_shasta_.*\.(elf|vk\.bin))$' \
  | sed 's#^crates/guests/elf/##' \
  | sort > "${EXPECTED_RELEASE_ASSETS}"
test -s "${EXPECTED_RELEASE_ASSETS}"

gh release view "${ELF_TAG}" --repo taikoxyz/raiko2 \
  --json assets --jq '.assets[].name' \
  | grep -E '^(risc0_shasta_.*\.elf|sp1_shasta_.*\.(elf|vk\.bin))$' \
  | sort > "${ACTUAL_RELEASE_ASSETS}"

diff -u "${EXPECTED_RELEASE_ASSETS}" "${ACTUAL_RELEASE_ASSETS}"

cargo run --locked -r -p xtask -- download-guest-elves \
  --tag "${ELF_TAG}" \
  --repo taikoxyz/raiko2 \
  --backend all \
  --dir "${VERIFY_DIR}"

while IFS= read -r artifact; do
  test -f "${VERIFY_DIR}/${artifact}"
  cmp -s "${VERIFY_DIR}/${artifact}" "crates/guests/elf/${artifact}"
done < "${EXPECTED_RELEASE_ASSETS}"
```

Any missing asset or byte mismatch is a hard stop. The GitHub Release assets, release tag, and
composition directory must identify the same programs.

#### 4. Gate Host/Guest Compatibility

First validate both provenance manifests, each backend's exact inventory, every recorded artifact,
and the Shasta SP1 ELF/VK pairs without comparing source fingerprints to the current host. This
prevents a source mismatch from hiding an artifact, manifest, or SP1 failure:

```bash
for backend in risc0 sp1; do
  manifest="crates/guests/elf/${backend}.provenance.json"
  provenance_artifacts="${VERIFY_ROOT}/${backend}.provenance-artifacts"
  disk_artifacts="${VERIFY_ROOT}/${backend}.disk-artifacts"

  jq -e --arg backend "${backend}" '
    .schema_version == 1
    and .backend == $backend
    and .bench == false
    and (.source_fingerprint | test("^[0-9a-f]{64}$"))
    and ((.artifacts | type) == "object")
    and ((.artifacts | length) > 0)
    and ([.artifacts[] | test("^[0-9a-f]{64}$")] | all)
  ' "${manifest}" >/dev/null

  jq -r '.artifacts | to_entries[] | "\(.value)  \(.key)"' "${manifest}" \
    | sha256sum --check --strict -

  jq -r '.artifacts | keys[]' "${manifest}" \
    | sort > "${provenance_artifacts}"

  find crates/guests/elf -maxdepth 1 -type f \
    \( -name "${backend}_*.elf" -o -name "${backend}_*.vk.bin" \) \
    -print \
    | sort > "${disk_artifacts}"

  diff -u "${provenance_artifacts}" "${disk_artifacts}"
done

cargo run --locked -r -p xtask-build-guest --bin guest-digests --features digests -- \
  --output "${VERIFY_ROOT}/guest-digests-summary.json"
```

Only after the artifact-only pass succeeds, run the source-closure check:

```bash
cargo run --locked -p xtask-build-guest --bin xtask-build-guest -- all --check
```

A pass means the current host revision has the same tracked guest build inputs and artifact hashes
as the selected release. It does not cover host-side input construction or encoding in the pipeline
and prover crates.

Every mixed host/released guest composition must record:

- proposal regression on the exact old release artifacts;
- aggregation regression on the exact old release artifacts;
- the regression inputs and request IDs in the composition PR.

A `source fingerprint mismatch` is not proof that the host and old guest are incompatible, but it
requires the composition PR to additionally record all of:

- a reviewed diff of guest-facing input types, serialization, public-input construction, manifest
  and carry-data hashing, and proposal and aggregation behavior;
- confirmation that the old guest contains every required soundness check and that its RISC0 image
  IDs and SP1 verification keys remain trusted on the target network.

Artifact SHA-256 equality and SP1 ELF/VK consistency establish artifact identity; they do not
establish protocol compatibility or soundness. The proof executes the old guest constraints,
regardless of newer host-side validation.

The source-drift exception is available only after the artifact-only pass above succeeds for both
backends. Any other provenance failure is a hard stop.

#### 5. Commit The Auditable Pairing

```bash
git commit --allow-empty \
  -m "chore(release): compose host ${HOST_SHORT} with ${ELF_TAG} guests" \
  -m "Host-Commit: ${HOST_SHA}" \
  -m "Guest-Release: ${ELF_TAG}" \
  -m "Guest-Commit: ${ELF_SHA}"

export COMPOSE_SHA
COMPOSE_SHA=$(git rev-parse HEAD)

test -z "$(git status --porcelain)"
git push -u origin "${COMPOSE_BRANCH}"
test "$(git ls-remote origin "refs/heads/${COMPOSE_BRANCH}" | cut -f1)" \
  = "${COMPOSE_SHA}"
```

The empty-commit case is intentional: even when the host branch already contains identical guest
bytes, the composition commit and branch still record the selected guest release explicitly. Do not
hide changes with `assume-unchanged` or `skip-worktree`, and do not publish from an unreferenced
detached commit.

#### 6. Build And Publish Without Guest Refresh

```bash
git fetch --no-tags origin main
test "$(git rev-parse 'origin/main^{commit}')" = "${HOST_SHA}"

mkdir -p target/release-image-logs
export IMAGE_REF RELEASE_LOG TAG_INSPECT_LOG
IMAGE_REF="${IMAGE_REPOSITORY}:${IMAGE_TAG}"
RELEASE_LOG="target/release-image-logs/${IMAGE_TAG}.log"
TAG_INSPECT_LOG=$(mktemp \
  "target/release-image-logs/tag-inspect.${IMAGE_TAG}.XXXXXXXX")

# The absence preflight is fail-closed against operator mistakes. The pushed digest, not the
# mutable tag, is the release handoff. The post-push digest check below verifies the registry tag
# resolves to the recorded digest at publication time.
if docker manifest inspect "${IMAGE_REF}" >"${TAG_INSPECT_LOG}" 2>&1; then
  echo "image tag already exists: ${IMAGE_REF}" >&2
  exit 1
fi

# Accept only a registry response that conclusively means the tag is absent. Authentication,
# authorization, connectivity, rate-limit, and server failures must stop the release.
BLOCKING_TAG_ERROR_PATTERN='authenticat|authoriz|credential|denied|forbidden|permission|reauthentication|login|service unavailable|too many requests|timeout|timed out|deadline|request canceled|connection|temporary failure|network is unreachable|no route to host|dial tcp|i/o timeout|tls handshake|certificate|unexpected eof|500 internal server error|502 bad gateway|504 gateway timeout|(^|[[:space:]:])(429|503)([[:space:]:]|$)'
if grep -Eqi "${BLOCKING_TAG_ERROR_PATTERN}" "${TAG_INSPECT_LOG}"; then
  cat "${TAG_INSPECT_LOG}" >&2
  exit 1
fi

if ! grep -Eqi \
  'manifest unknown|no such manifest|name unknown|requested entity was not found|manifest .* not found' \
  "${TAG_INSPECT_LOG}"; then
  cat "${TAG_INSPECT_LOG}" >&2
  exit 1
fi

just release-image host "${IMAGE_TAG}" "${IMAGE_REPOSITORY}" \
  --skip-guest-refresh 2>&1 | tee "${RELEASE_LOG}"

export DIGEST_REF
DIGEST_REF=$(sed -n 's/^\[INFO\] Image pushed: //p' "${RELEASE_LOG}")
test -n "${DIGEST_REF}"

# Resolve the mutable tag's manifest directly from the registry and compare exactly.
export TAG_MANIFEST_DIGEST
TAG_MANIFEST_DIGEST=$(docker buildx imagetools inspect "${IMAGE_REF}" \
  --format '{{json .Manifest}}' | jq -er '.digest')
test "${IMAGE_REPOSITORY}@${TAG_MANIFEST_DIGEST}" = "${DIGEST_REF}"

docker buildx imagetools inspect "${DIGEST_REF}"
```

`host` already skips guest refresh by default; the explicit flag documents the composition
decision. This SOP supports the default Google Artifact Registry repository shown above. A different
registry requires an equivalent fail-closed absence preflight and post-push tag digest
reconciliation. Do not use `--refresh-guest-elves` or an ad-hoc `docker build`.

#### 7. Verify The Published Image

```bash
docker pull "${DIGEST_REF}"

test "$(docker image inspect "${DIGEST_REF}" \
  --format '{{ index .Config.Labels "org.opencontainers.image.revision" }}')" \
  = "${COMPOSE_SHA}"

export IMAGE_ARTIFACT_DIR
IMAGE_ARTIFACT_DIR=$(mktemp -d "target/image-guest-artifacts.${IMAGE_TAG}.XXXXXXXX")
CID=$(docker create "${DIGEST_REF}")
docker cp "${CID}:/app/crates/guests/elf/." "${IMAGE_ARTIFACT_DIR}"
docker rm "${CID}"

diff -qr crates/guests/elf "${IMAGE_ARTIFACT_DIR}"
```

Report and retain:

```text
Host commit:        <HOST_SHA>
Guest release:      <ELF_TAG>
Guest commit:       <ELF_SHA>
Composition branch: <COMPOSE_BRANCH>
Composition commit: <COMPOSE_SHA>
Image tag:          <IMAGE_REPOSITORY>:<IMAGE_TAG>
Image digest:       <DIGEST_REF>
Compatibility:      proposal and aggregation regressions, plus any source-drift review
```

If tag-to-digest, revision-label, or packaged-artifact verification fails after push, mark the
publication invalid and do not hand off its tag or digest.

## Register Guest Digests

Guest builds and image releases do not update verifier trust lists automatically.
When a checked-in guest ELF changes, register the new digests explicitly with `xtask`:

```bash
# Built-in profiles are environment-specific. Pick the profile or explicit RPC/verifier overrides
# that match the target verifier network.
cargo run -r -p xtask -- register-image --profile hoodi-shasta --backend all
PRIVATE_KEY=0x... cargo run -r -p xtask -- register-image --profile hoodi-shasta --backend all --apply
cargo run -r -p xtask -- register-image --profile mainnet-shasta --backend all
PRIVATE_KEY=0x... cargo run -r -p xtask -- register-image --profile mainnet-shasta --backend all --apply
```

Current behavior:

- `risc0` registrations compute the digest from the current ELF and call
  `setImageIdTrusted(bytes32,bool)`.
- `sp1` registrations derive the current proving key digests from `setup(elf)` and call
  `setProgramTrusted(bytes32,bool)`.
- `sp1` `*.vk.bin` artifacts are checked against the ELF-derived key before dry-run or apply.
  A mismatch means the release artifacts are inconsistent and registration stops before any
  transaction is sent.
- Boundless program upload is a separate runtime concern and still happens automatically when
  `risc0/network` submits a request.

## Boundless Storage Upload

Boundless storage uploader selection is environment-driven. Use your private GCS
bucket:

```bash
BOUNDLESS_STORAGE_UPLOADER=gcs
GCS_BUCKET=<your-gcs-bucket>
GCS_PUBLIC_URL=false
```

The GCP project is selected by the current gcloud/ADC context, for example
`<your-gcp-project>`; raiko2 does not carry a separate project id setting. The
bucket should enforce public access prevention, so `GCS_PUBLIC_URL=false` returns
private `gs://` URLs and Boundless downloaders must have GCS credentials. Set
`GCS_PUBLIC_URL=true` only for a publicly readable bucket that should return HTTPS
URLs.

Authentication uses Google ADC from the active environment
(`GOOGLE_APPLICATION_CREDENTIALS`, workload identity, metadata server, or local
application-default credentials). Inline service account JSON through
`GCS_CREDENTIALS_JSON` is only needed when ADC is not available. Optional `GCS_URL`
supports custom endpoints. `STORAGE_UPLOADER` remains accepted as a compatibility
alias. Pinata and File remain available in default builds. S3 requires a host built
with the non-default `boundless-s3` feature; a default build rejects
`BOUNDLESS_STORAGE_UPLOADER=s3` with a configuration error.

## Release TEE Provider Metadata

TEE-backed remote prover images have a separate pre-release metadata flow.

Use a disposable local signing key for smoke verification without registry publication:

```bash
openssl genrsa -3 -out /tmp/raiko2-local-gramine-signing-key.pem 3072
RAIKO2_SGX_ENCLAVE_KEY_HOST=/tmp/raiko2-local-gramine-signing-key.pem \
cargo run -r -p xtask --no-default-features --features tee-provider-release -- \
  release-tee-providers --tag release-20260514-tee-smoke --no-push
```

for local smoke verification without registry publication. `--no-push` still builds both local SGX
images, clones and builds each external provider, replaces local Docker tags, and writes local output
state. Each manifest `image.digest` field contains a mutable `repository:tag` reference rather than
an immutable registry digest, and `mr_signer` will be the disposable local signer. The resulting
manifest must not be used as release handoff metadata.

For a formal pre-release export, use the `Release - TEE provider images` GitHub Actions workflow:

```bash
gh workflow run release-tee-providers.yml \
  --repo taikoxyz/raiko2 \
  --ref main \
  -f tag=vX.Y.Z-rc1
```

Publishing mode checks that each destination tag is currently unpublished before push. Registry-side
tag immutability is recommended as an operator control, but the release tooling does not require it.
After each push, the command resolves the remote tag through the registry and verifies it matches the
recorded digest. The generated manifest records digest references after push; use those immutable
digests as the release handoff, not mutable tag aliases. Use `--no-push` for local smoke verification
instead.

The pushed release workflow runs from the protected `sgx-release-signing` environment with the
release tag. It authenticates through Workload Identity Federation, fetches the enclave signing key
from GCP Secret Manager, pushes provider images to Artifact Registry, and uploads the generated
`tee-attestation-manifest-<tag>.json` as a workflow artifact. It may reuse the existing enclave
signing Google service account when that account already has the required Secret Manager and
Artifact Registry permissions, but its Workload Identity Provider or binding must be scoped to
`taikoxyz/raiko2`, `refs/heads/main`, and this workflow file. Configure these repository variables:

- `GCP_WORKLOAD_IDENTITY_PROVIDER`
- `GCP_ENCLAVE_SIGNER_SA`
- `GCP_ENCLAVE_KEY_SECRET`
- `GCP_ENCLAVE_KEY_PROJECT`
- `GCP_ENCLAVE_KEY_VERSION` (optional; defaults to `latest` when unset)

Official Taiko `mr_signer` values are only produced by this protected workflow. Do not document or
publish a locally built image as Taiko-signed, even if the source commit and `mr_enclave` match.

This flow:

- reads exact external provider pins from `release/providers.toml`
- fetches the `raiko2-sgx` Gramine enclave signing key from GCP Secret Manager inside the protected
  workflow
- verifies destination image tags are not already published for pushed runs
- builds two local `raiko2-sgx` provider images from the same source revision and signing key, with
  the key passed as a Docker BuildKit secret:
  - `<tag>` is the non-EDMM compatibility/default image
  - `<tag>-edmm` is the explicitly EDMM-enabled image
- clones and builds each pinned external TEE provider image
- pushes provider images unless `--no-push` is set
- records immutable image digests for pushed runs after remote tag digest reconciliation
- reads baked attestation metadata from each image
- emits one handoff artifact at
  `target/releases/<tag>/tee-attestation-manifest-<tag>.json`

In a pushed run, the two local images have distinct image digests and `mr_enclave` values, while
their `mr_signer` values match because they use the same signing key. The manifest records them as
separate local provider entries:

- `raiko2-sgx` with `image.sgx_edmm = false`
- `raiko2-sgx-edmm` with `image.sgx_edmm = true`

The command fails closed if the local `mr_enclave` values are equal, the local `mr_signer` values
differ, or the pushed image digests are equal. An invariant failure prevents the command from
writing a new manifest. It does not remove a manifest already present at the same tagged output
path, so operators must not treat that pre-existing file as output from the failed attempt.
External TEE providers keep their existing single-image build behavior and do not emit
`image.sgx_edmm`.

Use this manifest to hand off:

- `mr_enclave`
- `mr_signer`
- source commit
- pushed image digest

to whoever configures the on-chain verifier allowlists.

`GCP_ENCLAVE_KEY_VERSION` defaults to `latest` in the workflow. Local non-release builds should use
`RAIKO2_SGX_ENCLAVE_KEY_HOST` with a local or disposable key file; local output must not be treated
as official Taiko-signed release output.

### TEE Provider Release Notes Template

Use this only for TEE provider metadata releases. Do not include this section in
ZK/runtime-only release notes.

```markdown
## Summary

- TEE provider metadata release summary.

## TEE Provider Images

- `raiko2-sgx` (non-EDMM, `<release>`): us-docker.pkg.dev/evmchain/images/raiko2-sgx@sha256:...
- `raiko2-sgx-edmm` (EDMM, `<release>-edmm`): us-docker.pkg.dev/evmchain/images/raiko2-sgx@sha256:...
- `<provider>`: <provider image digest ref>

## TEE Attestation Metadata

- `raiko2-sgx` `mr_enclave`: ...
- `raiko2-sgx` `mr_signer`: ...
- `raiko2-sgx-edmm` `mr_enclave`: ...
- `raiko2-sgx-edmm` `mr_signer`: ...
- `<provider>` `mr_enclave`: ...
- `<provider>` `mr_signer`: ...

## Reproduce

See `docs/operations.md#reproduce-tee-provider-metadata`.

## Release Assets

- `tee-attestation-manifest-vX.Y.Z.json`
```

### Reproduce TEE Provider Metadata

Use this to regenerate local TEE provider attestation metadata from the tag checkout. Local
reproduction can check source pins and `mr_enclave`, but it cannot reproduce the official Taiko
`mr_signer`; that signer is produced only by the protected GitHub Actions workflow.

```bash
export TAG=vX.Y.Z
export REPRO_DIR=target/releases/${TAG}/tee-provider-repro

git fetch --tags origin "${TAG}"
git checkout "${TAG}"

mkdir -p "${REPRO_DIR}"

gh release download "${TAG}" --repo taikoxyz/raiko2 \
  --pattern "tee-attestation-manifest-${TAG}.json" \
  --dir "${REPRO_DIR}" \
  --clobber

openssl genrsa -3 -out "${REPRO_DIR}/local-gramine-signing-key.pem" 3072
RAIKO2_SGX_ENCLAVE_KEY_HOST="${REPRO_DIR}/local-gramine-signing-key.pem" \
cargo run -r -p xtask --no-default-features --features tee-provider-release -- \
  release-tee-providers --tag "${TAG}" --no-push

cp "target/releases/${TAG}/tee-attestation-manifest-${TAG}.json" \
  "${REPRO_DIR}/from-source.json"

jq -S '[.providers[]
  | {lane, provider, source, attestation: (.attestation | del(.mr_signer))}]
  | sort_by(.provider, .lane)' \
  "${REPRO_DIR}/tee-attestation-manifest-${TAG}.json" > "${REPRO_DIR}/release-tee.no-signer.sorted.json"
jq -S '[.providers[]
  | {lane, provider, source, attestation: (.attestation | del(.mr_signer))}]
  | sort_by(.provider, .lane)' \
  "${REPRO_DIR}/from-source.json" > "${REPRO_DIR}/source-tee.no-signer.sorted.json"
diff -u "${REPRO_DIR}/release-tee.no-signer.sorted.json" \
  "${REPRO_DIR}/source-tee.no-signer.sorted.json"
```

This command does not:

- run bootstrap/init
- register instance quotes
- apply on-chain verifier changes

Those steps remain part of later operator workflows.

## RISC0 Network Route

To use the network-backed RISC0 route, configure:

```toml
[prover.risc0]
enabled = true
runner = "network"

[prover.risc0.boundless]
offchain = false
rpc_url = "https://base-rpc.publicnode.com"
signer_key = "0xYOUR_PRIVATE_KEY"
poll_interval_ms = 10000
timeout_ms = 3600000
rebid_timeout_ms = 300000
rebid_price_step_bps = 5000
rebid_max_attempts = 4

[prover.risc0.boundless.deployment]
deployment_type = "base"
```

Full deployment and offer parameter examples live in
[`config.example.toml`](../config.example.toml).

Operator notes:

- `raiko2` uploads guest ELFs and submits Boundless requests directly.
- Production runtime state, provider checkpoints, publication intents, pending proof blobs, and proof
  manifests are stored in the configured GCS namespace. Proof bytes are immutable objects, not fields
  in the runtime snapshot. The state and object repositories have separate semantics even when both
  use that namespace. There are no local task workdirs.
- Run exactly one live instance per namespace. Active/active replicas and rolling overlap are not
  supported, even temporarily. Use a `Recreate`-equivalent deployment strategy: close admission and
  readiness, stop new dispatch and provider submission, wait only for short repository commits and
  pre-admitted provider checkpoint permits, then stop or abort and join every worker and maintenance
  task. Start the replacement only after the old process exits. Do not wait for every proof task or
  publication saga to finish; restart reconciliation resumes durable work. Namespace changes are hard
  cuts with no cross-namespace data migration, and the in-process execution projection is rebuilt
  from GCS rather than Redis.
- `runtime.startup_cleanup = ["proof"]` is the normal cutover for SGX/ZK guest, image, verifier, or
  proving-key upgrades. It removes authoritative runtime task state before active proof manifests,
  so stale completed tasks cannot resolve removed proofs. Add `"preflight"` only when derivation,
  fork, or witness-generation rules changed; that scope deletes active canonical preflight
  manifests and does not touch proof state. Cleanup runs after the old process exits and before
  recovery, workers, or HTTP admission. GCS requires `storage.objects.list` and
  `storage.objects.delete`, uses generation-protected manifest deletion with bounded concurrency,
  and aborts startup on failure. Immutable proof/preflight content and invalidation records remain
  for lifecycle TTL. Treat this as a one-shot cutover setting and remove `startup_cleanup` after the
  replacement starts successfully; otherwise every routine restart repeats the cleanup and can
  discard fresh task state and proof manifests. Keep prefixes non-overlapping so one deployment
  scope cannot contain another.
- For a preflight-cache incident, set `runtime.preflight_cache = "off"` and restart using the same
  environment and namespace. This bypasses both GCS preflight reads/writes and process-local
  singleflight without deleting runtime state or proof manifests. Restore `"shared"` after the
  incident is understood. This switch is not a replacement for `startup_cleanup = ["preflight"]`
  when cached preflight data is known to be semantically stale.
- Correlate `registered shasta proof task` and `completed shasta proof task` logs by `task_id`.
  `proof_type` is the resolved proof lane, while `requested_proof_type` on registration preserves
  the raw request such as `zk_any`. Completion logging is at-least-once because idempotent proof
  publication may be observed again; deduplicate accounting and SLO inputs by `task_id` plus
  `content_hash`.
- Before deploying this config schema, remove the former `runtime.reset_namespace_on_start` key
  from every environment and namespace. The service intentionally rejects that removed field.
  Configure the exact one-shot `startup_cleanup` scopes needed for the cutover, start the
  replacement only after the previous process exits, verify recovery and admission, then remove
  `startup_cleanup` so routine restarts do not repeat deletion.
- Treat runtime lifecycle as one global `NamespaceFence`, not a per-task lock or a lock held across a
  complete lifecycle operation. A process-local lifecycle transition gate serializes one short
  active-root decision across its runtime-state CAS and in-memory queue attach or detach. `Draining` rejects new task mutations, provider submissions,
  publication steps, invalidation, reconciliation, and cleanup writes. It waits only for short
  repository commits already admitted and request-ID checkpoints covered by permits acquired while
  active. `Inactive` rejects every write. There is deliberately no owner lease, owner epoch, or
  ownership heartbeat.
- Treat `incarnation_id`, scheduler lease tokens, and GCS generations as separate stale-operation
  domains. A `TaskLifetime` rejects callbacks for a removed and recreated runtime record; a queue
  lease token identifies one execution attempt; a manifest generation performs exact artifact CAS.
  Runtime-state generation, not serialized JSON byte order, is the snapshot CAS identity. None is
  runtime authority, and runtime-state generations remain repository-internal.
- Submission, cancellation, terminal failure, cleanup, and invalidation commit runtime state first,
  then apply owner-aware execution-projection and exact proof-object effects. A partial effect is
  recovered by reconciliation; operators must not attempt to repair it by reverting the
  authoritative root. If terminal-failure persistence is unavailable, the queue task remains
  retryable rather than becoming terminal ahead of its runtime root. Recovery, destructive cleanup,
  and replacement are conditional on the complete observed task snapshot, so stale requests do not
  detach a reopened root or install a second successor. Unowned pending-publication records retain
  their artifact identity until deletion succeeds and are swept during startup reconciliation. A
  successor at the same artifact key does not inherit the predecessor incarnation's publication
  intent.
- Boundless finalizes a non-zero market request ID and checkpoints it before either offchain or
  onchain dispatch. Treat that durable checkpoint as the dispatch admission boundary: cancellation
  that commits first prevents the provider call, while a later cancellation never causes a fresh
  request ID. An uncertain offchain response is polled under the checkpointed ID and is not sent a
  second time. The checkpoint is also bound to the exact guest image, Boundless market deployment,
  and submission transport. Before changing any of them, settle or explicitly abandon every
  outstanding remote request, then start the new configuration in a new namespace. An existing
  checkpoint fails closed rather than crossing that provider boundary.
- SP1 checkpoints the provider-assigned request ID together with its original submission time. A
  restart or late-joining root reprojects that exact timestamp and deadline; it never extends the
  paid request's timeout by treating recovery as a new submission.
- Proposal execution nodes are position-independent: batch order never creates dependencies between
  proposals, and only aggregation depends on the proposal artifact tasks it consumes. Proof
  activation refreshes current owners under the short local lifecycle gate; newly registered distinct
  roots may share the result, but a replacement incarnation for a checkpointed task ID is excluded.
  Execution owners are resolved from canonical task membership, not the artifact-reference index;
  external aggregate inputs remain storage consumers without receiving proposal-stage callbacks.
  Cached proposal artifacts are execution short-circuits, not graph-shape inputs, so restart and
  failed-aggregate recovery rebuild the identical proposal and dependency graph.
- This release requires an atomic configuration cutover. Before starting the new binary, remove
  legacy `[queue]` keys `backend`, `namespace`, and `redis_url`, remove legacy `[runtime]` keys
  `root` and `inactive_ttl_secs`, and add explicit `runtime.environment`, `runtime.namespace`, and
  `[runtime.store]` settings. Apply the new ConfigMap while the old instance is drained; old and new
  schemas are not dual-read. Keep the prior ConfigMap and GCS namespace together for rollback.
- The runtime snapshot schema is also a hard cut: task `incarnation_id`, first-class artifact
  identity fields, canonical proposal and aggregate requests, and publication intent owner/hash
  fields are required. Unknown fields, missing requests, and derived identity drift fail startup and
  are not reconstructed from older snapshots. Deploy with a new empty namespace (or explicitly
  delete the old runtime snapshot after the old instance exits);
  there is no compatibility migration or fail-open recovery for legacy checkpoint state.
- Terminal root tasks (`completed`, `failed`, `cancelled`) are retained for seven days. Active proof
  and canonical preflight manifests must not have an age-based GCS lifecycle rule. Immutable content
  must remain available until every manifest that references it is gone. Generation-scoped
  invalidation markers and unreferenced proof/preflight content use a minimum 30-day retention
  window.
- Proposal requests are sized by `prover.risc0.boundless.batch_quote`. The default
  `strategy = "raiko_agent"` rounds evaluated user cycles up to the next `1000` mcycles with a
  `2000` mcycle floor; `"evaluated"` uses the raw dry-run count, and `"fixed"` pins a `mcycles`
  value.
- Aggregation requests are sized by `prover.risc0.boundless.aggregation_quote` (same strategies).
- `prover.risc0.boundless.rebid_timeout_ms` controls how long an unlocked market request can remain
  unclaimed before `raiko2` resubmits at a higher max price. The default is `300000` ms, and the
  minimum is `1000` ms.
- `prover.risc0.boundless.rebid_price_step_bps` controls the per-rebid max-price escalation, in basis
  points, compounded over the base max price. The default is `5000` (+50% per rung). `0` is a valid
  flat ladder; values in `1..100` are rejected as a likely basis-points/multiplier confusion.
- `prover.risc0.boundless.rebid_max_attempts` caps replacement submissions across every retry path —
  no-lock, expired, and poll-timeout requests all draw from the same budget. The default is `4`, the
  maximum is `31`, and the default allows a final max price of about `5x` the base at the default
  step, unless `absolute_max_price_per_mcycle` clamps it sooner.
- `prover.risc0.boundless.offer_params.{batch,aggregation}.pricing_mode` defaults to `manual`.
  `manual` requires `max_price_per_mcycle` and optionally accepts `min_price_per_mcycle`;
  `market` omits both price fields and lets the Boundless SDK price provider set the offer price.
- `prover.risc0.boundless.offer_params.{batch,aggregation}.absolute_max_price_per_mcycle` is the
  absolute per-mcycle bid ceiling: no attempt in either pricing mode ever bids above it. In
  `manual` mode it bounds the bps rebid escalation and must be at least `max_price_per_mcycle`; in
  `market` mode it is the canonical spelling of the safety cap (`max_price_per_mcycle` remains
  accepted, but setting both is rejected).
- When a Boundless request expires unfulfilled, `raiko2` resubmits it. Each resubmission escalates
  the offer's max price by `prover.risc0.boundless.rebid_price_step_bps` (compounded) up to
  `prover.risc0.boundless.rebid_max_attempts`, clamped to `absolute_max_price_per_mcycle` when it is
  set; the min price is unchanged. `market` resubmissions are re-priced by the SDK price provider
  and then escalated by the same step.
- `prover.risc0.boundless.deployment.deployment_type` selects the Boundless market deployment.
  Supported values are `base`, `sepolia`, and `taiko`; use `taiko` for Taiko mainnet market
  submissions.
- `rpc.pairs[*].boundless` can override `batch_quote`, `aggregation_quote`, runtime timeout/rebid
  fields (including `rebid_price_step_bps`), and either offer param block for a specific
  `(network, l1_network)` pair. This only affects `risc0/network`; SP1 ignores it.
- The local dry-run validates guest execution and prepares the request journal.

Optional `zk_any` request sampling is configured at the server level:

```toml
[prover.zk_any.sp1]
probability = 0.05
per_day = 8

[prover.zk_any.risc0]
probability = 0.05
per_day = 8
```

When a client submits `proof_type = "zk_any"` to `/v3/proof/batch/shasta`, the server draws once
at admission time and either routes the request to `sp1` / `risc0` or returns
`data.status = "zk_any_not_drawn"` without registering a task.
`zk_any` is only accepted when `aggregate = false`; aggregate requests must specify a concrete
proof type such as `sp1` or `risc0`.

Operators can adjust the in-memory `zk_any` ballot without restarting the server when an ACL key
grants `admin.ballot.read` and `admin.ballot.write`:

```bash
curl -H "x-api-key: $RAIKO2_ACL_API_KEY" http://localhost:8080/admin/ballot
curl -X POST -H "x-api-key: $RAIKO2_ACL_API_KEY" -H "content-type: application/json" \
  --data '{"Risc0":[0.1,10],"Sp1":[0.0,0]}' \
  http://localhost:8080/admin/ballot
```

Only `Risc0` and `Sp1` are accepted. The second tuple value is the per-day frequency gate; `0`
disables the gate for that proof type.

## SP1 Hosted Posture

For hosted proof submission, `sp1.mode = "prove"` now requires `sp1.verify = true`.
`sp1.cycle_limit` is the default Succinct network request cycle limit. Operators can set
`sp1.proposal_cycle_limit` and `sp1.aggregation_cycle_limit` to tune proposal and aggregation
requests independently; if either is unset, it falls back to `sp1.cycle_limit`.

This is enforced in two places:

- config validation for the server default route
- request validation for hosted `sp1` batch and aggregate requests

Hosted `sp1.prover = "network"` verification is enabled per RPC pair, not globally. Set both:

- `rpc.pairs[*].sp1_verifier_rpc_url`
- `rpc.pairs[*].sp1_verifier_address`

to turn on remote verifier-contract checks for that pair. Leave them unset to keep a line closed
without changing the rest of the endpoint posture.

`verify = false` is no longer a supported hosted-API posture for production operation.

## RPC and Witness Expectations

`rpc.pairs[*].l2_rpc` should ideally point to a witness-capable endpoint that supports
`debug_executionWitness` for the best latency envelope.

`rpc.pairs[*].l2_provider` selects the L2 execution-client family. It defaults to `reth`, whose
`debug_executionWitness` response carries RLP-encoded headers. Use `geth` for geth's native
`debug_executionWitness` response, where headers are JSON objects.

Use `geth_local_witness` for a regular geth L2 endpoint when `raiko2` should always build witnesses
locally from RPC state instead of calling `debug_executionWitness`.

For geth witness endpoints, run a version that includes the upstream `debug_executionWitness`
corruption fix from geth v1.17.2 or newer.

`rpc.pairs[*].l2_witness_rpc` is optional. When set, witness/debug traffic uses that endpoint
while canonical chain data still comes from `rpc.pairs[*].l2_rpc`.

`rpc.pairs[*].sp1_verifier_rpc_url` and `rpc.pairs[*].sp1_verifier_address` are optional pair
settings for hosted SP1 network verification. They point to the verifier-chain RPC and deployed
Succinct verifier contract used after a network proof is fulfilled. This is separate from the Taiko
Shasta verifier address used for proof registration and chain-spec data carried in proofs.

For supported Taiko chain specs, `raiko2` can fall back to on-the-spot witness construction when
the endpoint does not expose `debug_executionWitness`, but that path is materially slower.

## Preflight Concurrency

Shasta preflight defaults are aligned with the old raiko hosted deployment shape:

- `queue.workers=6` runs up to six queue tasks in parallel, matching the old hosted proving
  concurrency.
- `PREFLIGHT_CHUNK_SIZE=8` splits proposal blocks into moderate batches.
- `PREFLIGHT_CHUNK_CONCURRENCY=6` limits the number of chunks fetched at the same time.
- `WITNESS_BATCH_SIZE=2` limits locally assembled geth witnesses inside each chunk.

`PREFETCH_CHUNK_SIZE` is still accepted as an alias for old-raiko deployment compatibility.
Increase these values only when the L2 RPC can sustain the additional concurrent state and witness
traffic.

Preflight retries retryable provider/RPC/IO failures inside the stage with exponential backoff.
Invalid request/configuration errors and deterministic validation failures fail fast. The queue task
timeout remains the outer deadline for all preflight attempts.

If the upstream L2 does not expose that method and you need predictable proving latency, place
[`zeth-rpc-proxy`](../bin/rpc-proxy) in front of it and point `rpc.pairs[*].l2_witness_rpc` at
the proxy. If `l2_witness_rpc` is unset, the server falls back to `l2_rpc`.

## Health and Readiness

- `GET /health`: basic process health
- `GET /metrics`: Prometheus text-format key service metrics
- `GET /ready`: configured L1/L2 RPC chain-ID readiness, global runtime lifecycle and store
  access, recent queue-maintenance success, and prerequisite checks for the hosted proving
  capabilities exposed by the endpoint. Queue maintenance is stale after
  `max(3 * queue.maintenance_interval_ms, 1000ms)`.

The response reports separate `reth`, `runtime`, `queue`, and `prover` checks. See
[Architecture](architecture.md#readiness) for the traffic-gating flow and lifecycle behavior.

The hosted server exports a minimal Prometheus surface focused on request intake and proving-stage
health:

- `raiko2_request_registrations_total`
- `raiko2_stage_tasks_inflight`
- `raiko2_stage_task_started_total`
- `raiko2_stage_task_terminal_total`
- `raiko2_stage_task_failures_total`
- `raiko2_stage_task_duration_seconds`
- `raiko2_duplicate_requests_total`
- `raiko2_external_submission_total`

Import [raiko2-hosted-stage-latency.json](./grafana/raiko2-hosted-stage-latency.json) into
Grafana for a baseline hosted-api dashboard with preflight, prove, aggregate, inflight, and
external-submission panels.

For the old log-based alert shape of "too many errors in the last 30 minutes", prefer Prometheus
counters instead of matching the text `error` in logs. The broad equivalent is:

```promql
sum(increase(raiko2_stage_task_terminal_total{status="failed"}[30m])) > 30
```

For diagnosis, alert or dashboard by bounded failure kind:

```promql
sum by (pair, proof_type, stage, error_kind) (
  increase(raiko2_stage_task_failures_total[30m])
)
```

Duplicate requests that return completed cache hits are normally harmless. Duplicates against failed
tasks, non-terminal tasks, or completed tasks whose proof artifact is missing should be watched
separately. Missing completed artifacts are reported as
`runner_status="completed_artifact_missing"`:

```promql
sum by (pair, proof_type, aggregate, runner_status) (
  increase(raiko2_duplicate_requests_total{runner_status!="completed"}[30m])
)
```
