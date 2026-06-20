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

Redis-backed queueing requires rebuilding with `BIN_FEATURES=--features redis-queue`; Boundless
does not need an extra feature flag in default builds.

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

## TDX GuestInput Runtime

The `raiko2-tdx-prover` binary exposes the same remote HTTP surface and GuestInput validation path
as `raiko2-sgx-prover`, but binds runtime identity to TDX:

- binary: `raiko2-tdx-prover`
- default config dir: `~/.config/raiko2/tdx/config`
- default secret dir: `~/.config/raiko2/tdx/secrets`
- mode env var: `RAIKO2_TDX_MODE`
- quote socket env var: `RAIKO2_TDXS_SOCKET` (default `/var/tdxs.sock`)

TEE mode expects the in-VM `tdxs` daemon to listen on the configured Unix socket. `bootstrap`
requests a quote bound to the prover instance address, while proposal and aggregate proofs request
per-proof quotes bound to the signed input hash. This keeps the TDX quote tied to the proof payload
instead of only to the long-lived instance identity.

TEE-mode example:

```bash
cargo run -r -p raiko2-tdx-prover -- \
  --mode tee \
  --tdxs-socket /var/tdxs.sock \
  --config-dir ~/.config/raiko2/tdx/config \
  --secret-dir ~/.config/raiko2/tdx/secrets \
  serve --listen-addr 0.0.0.0:8080
```

Local native-mode smoke:

```bash
cargo run -r -p raiko2-tdx-prover -- \
  --mode native \
  serve --listen-addr 0.0.0.0:8080
```

Native mode is for protocol and GuestInput replay regression. Production TDX still needs a real TDX
VM image/release/deploy flow that pins `raiko2-tdx-prover`, `tdxs`, systemd units, and measured
configuration; do not treat native mode as a trusted TDX proof.

### Docker Compose

Quickstart:

```bash
cp docker/.env.sgx.sample docker/.env.sgx
$EDITOR docker/.env.sgx
docker compose --env-file docker/.env.sgx -f docker/docker-compose.sgx.yml --profile init up raiko2-sgx-init
docker compose --env-file docker/.env.sgx -f docker/docker-compose.sgx.yml up raiko2-sgx
```

Operator notes:

- The compose stack mounts SGX devices and passes the enclave signing key as a build secret.
- The default signing key is the checked-in [`docker/enclave-key.pem`](../docker/enclave-key.pem),
  inherited from the historical `raiko` SGX release flow. Override `RAIKO2_SGX_ENCLAVE_KEY_HOST`
  only when you intentionally need a different signer.
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

TDX is a separate first-party remote route. A local or deployed `raiko2` host uses
`RAIKO2_PROVER=tdx/remote` plus `RAIKO2_REMOTE_TDX_BASE_URL` when targeting
`raiko2-tdx-prover`.

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

`raiko2` keeps the TEE paths as remote routes. The dedicated `raiko2-sgx-prover` binary is the
runtime for `sgx`, and `raiko2-tdx-prover` is the first-party runtime for `tdx`. Historical
`sgxgeth` compatibility is expected to come from an external `gaiko2` SGX service.

## Source Releases

Use this flow when cutting a versioned source release such as `v0.1.0`.

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
   cargo run -p xtask-build-guest --bin guest-digests -- \
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

5. Write release notes:

   ```bash
   cat > "${RELEASE_DIR}/release-notes-${TAG}.md" <<'EOF'
   ## Summary

   - release summary here

   ## Runtime Images

   - runtime image: us-docker.pkg.dev/evmchain/images/raiko2@sha256:...
   - includes both `risc0` and `sp1` guest ELFs

   ## ZK Guest Digests

   - risc0 proposal image_id: 0x...
   - risc0 aggregation image_id: 0x...
   - sp1 proposal vk_bn254: 0x...
   - sp1 proposal vk_hash_bytes: 0x...
   - sp1 aggregation vk_bn254: 0x...
   - sp1 aggregation vk_hash_bytes: 0x...

   See attached `release-manifest-vX.Y.Z.json` and `guest-digests-summary.json`.
   EOF
   ```

6. Create the tag and GitHub Release:

   ```bash
   git tag "${TAG}" "${RELEASE_SHA}"
   git push origin "${TAG}"

   gh release create "${TAG}" \
     --target "${RELEASE_SHA}" \
     --title "${TAG}" \
     --notes-file "${RELEASE_DIR}/release-notes-${TAG}.md" \
     "${RELEASE_DIR}/release-manifest-${TAG}.json" \
     "${RELEASE_DIR}/guest-digests-summary.json"
   ```

Expected release outputs:

- git tag: `${TAG}`
- runtime image tag: `${TAG}`
- release notes file: `release-notes-${TAG}.md`
- release manifest file: `release-manifest-${TAG}.json`
- guest digest export file: `guest-digests-summary.json`

Do not:

- apply `register-image` automatically as part of the release cut
- mix rollout or deployment steps into the release flow
- write release-only metadata back into the source tree

## Release Images

Use the `xtask` release entrypoint for runtime images. It ensures the checked-in guest ELFs are
current, then builds and pushes the runtime image.

```bash
just release-image risc0 release-20260507-1013
```

Direct `xtask` entrypoint:

```bash
cargo run -r -p xtask -- release-image risc0 \
  --tag release-20260507-1013 \
  --repository us-docker.pkg.dev/evmchain/images/raiko2
```

Avoid ad-hoc `docker build` for releases. The runtime image packages the existing
`crates/guests/elf` artifacts at `/app/crates/guests/elf`; `raiko2` loads those files when the
process starts and does not rebuild guest sources by itself. The image sets
`RAIKO2_GUEST_ELF_DIR=/app/crates/guests/elf` so ELF lookup does not depend on the container
working directory.

If `release-image` refreshes tracked guest ELF artifacts and leaves the worktree dirty, it stops
before publishing. Review and commit the updated `crates/guests/elf` artifacts, then rerun the
release command so the image provenance still matches committed repo state.

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
- Boundless program upload is a separate runtime concern and still happens automatically when
  `risc0/network` submits a request.

## Release TEE Provider Metadata

TEE-backed remote prover images have a separate pre-release metadata flow.

Use:

```bash
cargo run -r -p xtask -- release-tee-providers --tag release-20260514-tee-smoke --no-push
```

for local smoke verification, and:

```bash
cargo run -r -p xtask -- release-tee-providers --tag vX.Y.Z-rc1
```

for a formal pre-release export.

This flow:

- reads exact external provider pins from `release/providers.toml`
- builds the local `raiko2-sgx` provider image
- clones and builds each pinned external TEE provider image
- pushes provider images unless `--no-push` is set
- records immutable image digests
- reads baked attestation metadata from each image
- emits one handoff artifact:
  - `target/releases/<tag>/tee-attestation-manifest-<tag>.json`

Use this manifest to hand off:

- `mr_enclave`
- `mr_signer`
- source commit
- pushed image digest

to whoever configures the on-chain verifier allowlists.

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

[prover.boundless.deployment]
deployment_type = "base"
```

Full deployment and offer parameter examples live in
[`config.example.toml`](../config.example.toml).

Operator notes:

- `raiko2` uploads guest ELFs and submits Boundless requests directly.
- Runtime state and task workdirs are stored under `./data/runtime` by default.
- `runtime.inactive_ttl_secs` controls automatic cleanup for terminal root tasks
  (`completed`, `failed`, `cancelled`). `0` disables cleanup; the default is `7200` seconds.
- Proposal requests use `prover.boundless.batch_quoted_mcycles` when it is set. Otherwise,
  `batch_quote_strategy = "raiko_agent"` rounds evaluated user cycles up to the next `1000`
  mcycles with a `2000` mcycle floor.
- Aggregation requests use `prover.boundless.aggregation_quoted_mcycles`.
- `prover.boundless.offer_params.{batch,aggregation}.pricing_mode` defaults to `manual`.
  `manual` requires `max_price_per_mcycle` and optionally accepts `min_price_per_mcycle`;
  `market` omits both price fields and lets the Boundless SDK price provider set the offer price.
- `prover.boundless.deployment.deployment_type` selects the Boundless market deployment. Supported
  values are `base`, `sepolia`, and `taiko`; use `taiko` for Taiko mainnet market submissions.
- `rpc.pairs[*].boundless` can override `batch_quoted_mcycles`,
  `aggregation_quoted_mcycles`, and either offer param block for a specific
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

Operators can adjust the in-memory `zk_any` ballot without restarting the server when
`server.admin_api_key` is configured:

```bash
curl -H "x-api-key: $RAIKO2_ADMIN_API_KEY" http://localhost:8080/admin/ballot
curl -X POST -H "x-api-key: $RAIKO2_ADMIN_API_KEY" -H "content-type: application/json" \
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
- `GET /ready`: configured L1/L2 RPC chain-ID readiness, queue readiness, and prerequisite checks
  for the hosted proving capabilities exposed by the endpoint

The hosted server exports a minimal Prometheus surface focused on request intake and proving-stage
health:

- `raiko2_request_registrations_total`
- `raiko2_stage_tasks_inflight`
- `raiko2_stage_task_started_total`
- `raiko2_stage_task_terminal_total`
- `raiko2_stage_task_duration_seconds`
- `raiko2_external_submission_total`

Import [raiko2-hosted-stage-latency.json](./grafana/raiko2-hosted-stage-latency.json) into
Grafana for a baseline hosted-api dashboard with preflight, prove, aggregate, inflight, and
external-submission panels.
