# Operations Guide

This guide covers runtime configuration, Docker, SGX operation, and Boundless operation.
API contracts live in [API.md](API.md), and the canonical config shape lives in
[`config.example.toml`](../config.example.toml).

See also:

- [Docs index](README.md)
- [README](../README.md) for the project overview
- [Development guide](development.md) for local workflows and guest tooling

## Run the Server

Run the server with an explicit config file:

```bash
cp config.example.toml config.toml
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
memory queue. Default binaries include RISC Zero local/network proving and SP1 proving.

To switch proving routes, change `RAIKO2_PROVER` in `docker/.env`:

- `native/local`
- `risc0/local`
- `risc0/network`
- `sp1/local`
- `sp1/network`

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
- Set `RAIKO2_SGX_ENCLAVE_KEY_HOST` to a local Gramine enclave signing key. Release builds fetch the
  signing key from GCP Secret Manager through `release-tee-providers`; do not commit signing keys.
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
RAIKO2_PROVER=sgx/remote \
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

2. Publish the runtime image:

   ```bash
   just release-image all ${TAG}
   ```

   Record the immutable digest references printed by `release-image`:

   - `us-docker.pkg.dev/evmchain/images/raiko2@sha256:...`

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

5. For the default full profile, build and validate the TEE provider images before creating the
   release notes or GitHub Release:

   ```bash
   GCP_ENCLAVE_KEY_SECRET=<secret-name> \
   GCP_ENCLAVE_KEY_VERSION=latest \
   GCP_ENCLAVE_KEY_PROJECT=<gcp-project> \
   cargo run -r -p xtask -- release-tee-providers --tag "${TAG}"
   ```

   This must produce `target/releases/${TAG}/tee-attestation-manifest-${TAG}.json`. Record the
   immutable image digests and attestation values from that manifest. The command validates both
   local SGX variants before publishing their final tags.

6. Write release notes from the ZK source release template, then append the TEE Provider Release
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

7. Create the tag and GitHub Release:

   ```bash
   git tag "${TAG}" "${RELEASE_SHA}"
   git push origin "${TAG}"

   gh release create "${TAG}" \
     --target "${RELEASE_SHA}" \
     --title "${TAG}" \
     --notes-file "${RELEASE_DIR}/release-notes-${TAG}.md" \
     "${RELEASE_DIR}/release-manifest-${TAG}.json" \
     "${RELEASE_DIR}/guest-digests-summary.json" \
     "target/releases/${TAG}/tee-attestation-manifest-${TAG}.json" \
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
alias, and existing S3/Pinata/File settings continue to work.

## Release TEE Provider Metadata

TEE-backed remote prover images have a separate pre-release metadata flow.

Use:

```bash
GCP_ENCLAVE_KEY_SECRET=<secret-name> \
GCP_ENCLAVE_KEY_VERSION=latest \
GCP_ENCLAVE_KEY_PROJECT=<gcp-project> \
cargo run -r -p xtask -- release-tee-providers --tag release-20260514-tee-smoke --no-push
```

for local smoke verification without registry publication. `--no-push` still builds both local SGX
images, clones and builds each external provider, replaces local Docker tags, and writes local output
state. Each manifest `image.digest` field contains a mutable `repository:tag` reference rather than
an immutable registry digest. The resulting manifest must not be used as release handoff metadata;
run the command without `--no-push` to push the images and resolve immutable digests first.

For a formal pre-release export, use:

```bash
GCP_ENCLAVE_KEY_SECRET=<secret-name> \
GCP_ENCLAVE_KEY_VERSION=latest \
GCP_ENCLAVE_KEY_PROJECT=<gcp-project> \
cargo run -r -p xtask -- release-tee-providers --tag vX.Y.Z-rc1
```

This flow:

- reads exact external provider pins from `release/providers.toml`
- fetches the local `raiko2-sgx` Gramine enclave signing key from GCP Secret Manager when
  `GCP_ENCLAVE_KEY_SECRET` is set
- builds two local `raiko2-sgx` provider images from the same source revision and signing key, with
  the key passed as a Docker BuildKit secret:
  - `<tag>` is the non-EDMM compatibility/default image
  - `<tag>-edmm` is the explicitly EDMM-enabled image
- clones and builds each pinned external TEE provider image
- pushes provider images unless `--no-push` is set
- records immutable image digests for pushed runs
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

`GCP_ENCLAVE_KEY_VERSION` defaults to `latest`. Omit `GCP_ENCLAVE_KEY_PROJECT` to use the active
`gcloud` project. Release builds must set `GCP_ENCLAVE_KEY_SECRET`. For local non-release builds
only, `RAIKO2_SGX_ENCLAVE_KEY_HOST` can point to a local key file.

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

Use this to regenerate TEE provider attestation metadata from the tag checkout.
Official rebuilds need the release enclave signing key from GCP Secret Manager
to reproduce `mr_signer`; a disposable local key can reproduce `mr_enclave` but
will produce a different signer.

```bash
export TAG=vX.Y.Z
export REPRO_DIR=target/releases/${TAG}/tee-provider-repro

git fetch --tags origin "${TAG}"
git checkout "${TAG}"

GCP_ENCLAVE_KEY_SECRET=<secret-name> \
GCP_ENCLAVE_KEY_VERSION=latest \
GCP_ENCLAVE_KEY_PROJECT=<gcp-project> \
cargo run -r -p xtask -- release-tee-providers --tag "${TAG}" --no-push

mkdir -p "${REPRO_DIR}"
cp "target/releases/${TAG}/tee-attestation-manifest-${TAG}.json" \
  "${REPRO_DIR}/from-source.json"

gh release download "${TAG}" --repo taikoxyz/raiko2 \
  --pattern "tee-attestation-manifest-${TAG}.json" \
  --dir "${REPRO_DIR}" \
  --clobber

jq -S '[.providers[]
  | {lane, provider, source, attestation}]
  | sort_by(.provider, .lane)' \
  "${REPRO_DIR}/tee-attestation-manifest-${TAG}.json" > "${REPRO_DIR}/release-tee.sorted.json"
jq -S '[.providers[]
  | {lane, provider, source, attestation}]
  | sort_by(.provider, .lane)' \
  "${REPRO_DIR}/from-source.json" > "${REPRO_DIR}/source-tee.sorted.json"
diff -u "${REPRO_DIR}/release-tee.sorted.json" "${REPRO_DIR}/source-tee.sorted.json"
```

For a disposable local signing key, run the same rebuild with `RAIKO2_SGX_ENCLAVE_KEY_HOST` instead
of `GCP_ENCLAVE_KEY_*`, then compare the same projection with `attestation.mr_signer` removed from
both manifests:

```bash
RAIKO2_SGX_ENCLAVE_KEY_HOST=/path/to/local/gramine-signing-key.pem \
cargo run -r -p xtask -- release-tee-providers --tag "${TAG}" --no-push

mkdir -p "${REPRO_DIR}"
cp "target/releases/${TAG}/tee-attestation-manifest-${TAG}.json" \
  "${REPRO_DIR}/from-source.json"

gh release download "${TAG}" --repo taikoxyz/raiko2 \
  --pattern "tee-attestation-manifest-${TAG}.json" \
  --dir "${REPRO_DIR}" \
  --clobber

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
[prover]
guest_system = "risc0"
runner = "network"

[prover.boundless]
offchain = false
rpc_url = "https://base-rpc.publicnode.com"
signer_key = "0xYOUR_PRIVATE_KEY"
poll_interval_ms = 10000
timeout_ms = 3600000
rebid_timeout_ms = 300000
rebid_price_step_bps = 5000
rebid_max_attempts = 4

[prover.boundless.deployment]
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
- Treat runtime lifecycle as one global `NamespaceFence`, not a per-task lock or a lock held across a
  complete lifecycle operation. A process-local lifecycle transition gate serializes only an
  active-root decision with its one in-memory queue attach or detach. `Draining` rejects new task mutations, provider submissions,
  publication steps, invalidation, reconciliation, and cleanup writes. It waits only for short
  repository commits already admitted and request-ID checkpoints covered by permits acquired while
  active. `Inactive` rejects every write. There is deliberately no owner lease, owner epoch, or
  ownership heartbeat.
- Treat `incarnation_id`, scheduler lease tokens, and GCS generations as separate stale-operation
  domains. A `TaskLifetime` rejects callbacks for a removed and recreated runtime record; a queue
  lease token identifies one execution attempt; a manifest generation performs exact artifact CAS.
  None is runtime authority, and runtime-state generations remain repository-internal.
- Submission, cancellation, terminal failure, cleanup, and invalidation commit runtime state first,
  then apply owner-aware execution-projection and exact proof-object effects. A partial effect is
  recovered by reconciliation; operators must not attempt to repair it by reverting the
  authoritative root.
- This release requires an atomic configuration cutover. Before starting the new binary, remove
  legacy `[queue]` keys `backend`, `namespace`, and `redis_url`, remove legacy `[runtime]` keys
  `root` and `inactive_ttl_secs`, and add explicit `runtime.environment`, `runtime.namespace`, and
  `[runtime.store]` settings. Apply the new ConfigMap while the old instance is drained; old and new
  schemas are not dual-read. Keep the prior ConfigMap and GCS namespace together for rollback.
- The runtime snapshot schema is also a hard cut: task `incarnation_id`, first-class artifact
  identity fields, and publication intent owner/hash fields are required and are not reconstructed
  from older snapshots. Deploy with a new empty namespace (or explicitly delete the old runtime
  snapshot after the old instance exits);
  there is no compatibility migration or fail-open recovery for legacy checkpoint state.
- Terminal root tasks (`completed`, `failed`, `cancelled`) are retained for seven days. Active
  proof manifests must not have an age-based GCS lifecycle rule, and immutable proof content must
  remain available until every manifest that references it is gone. Generation-scoped invalidation
  markers and unreferenced content use a minimum 30-day retention window.
- Proposal requests are sized by `prover.boundless.batch_quote`. The default
  `strategy = "raiko_agent"` rounds evaluated user cycles up to the next `1000` mcycles with a
  `2000` mcycle floor; `"evaluated"` uses the raw dry-run count, and `"fixed"` pins a `mcycles`
  value.
- Aggregation requests are sized by `prover.boundless.aggregation_quote` (same strategies).
- `prover.boundless.rebid_timeout_ms` controls how long an unlocked market request can remain
  unclaimed before `raiko2` resubmits at a higher max price. The default is `300000` ms, and the
  minimum is `1000` ms.
- `prover.boundless.rebid_price_step_bps` controls the per-rebid max-price escalation, in basis
  points, compounded over the base max price. The default is `5000` (+50% per rung). `0` is a valid
  flat ladder; values in `1..100` are rejected as a likely basis-points/multiplier confusion.
- `prover.boundless.rebid_max_attempts` caps replacement submissions across every retry path —
  no-lock, expired, and poll-timeout requests all draw from the same budget. The default is `4`, the
  maximum is `31`, and the default allows a final max price of about `5x` the base at the default
  step, unless `absolute_max_price_per_mcycle` clamps it sooner.
- `prover.boundless.offer_params.{batch,aggregation}.pricing_mode` defaults to `manual`.
  `manual` requires `max_price_per_mcycle` and optionally accepts `min_price_per_mcycle`;
  `market` omits both price fields and lets the Boundless SDK price provider set the offer price.
- `prover.boundless.offer_params.{batch,aggregation}.absolute_max_price_per_mcycle` is the
  absolute per-mcycle bid ceiling: no attempt in either pricing mode ever bids above it. In
  `manual` mode it bounds the bps rebid escalation and must be at least `max_price_per_mcycle`; in
  `market` mode it is the canonical spelling of the safety cap (`max_price_per_mcycle` remains
  accepted, but setting both is rejected).
- When a Boundless request expires unfulfilled, `raiko2` resubmits it. Each resubmission escalates
  the offer's max price by `prover.boundless.rebid_price_step_bps` (compounded) up to
  `prover.boundless.rebid_max_attempts`, clamped to `absolute_max_price_per_mcycle` when it is set;
  the min price is unchanged. `market` resubmissions are re-priced by the SDK price provider and
  then escalated by the same step.
- `prover.boundless.deployment.deployment_type` selects the Boundless market deployment. Supported
  values are `base`, `sepolia`, and `taiko`; use `taiko` for Taiko mainnet market submissions.
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
