# Operations Guide

This guide covers runtime configuration, Docker, release images, and Boundless operation.
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
memory queue.

To switch proving routes, change `RAIKO2_PROVER` in `docker/.env`:

- `native/local`
- `risc0/local`
- `risc0/boundless`
- `sp1/local`

Redis-backed queueing requires rebuilding with `BIN_FEATURES=--features redis-queue`.

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
- `proof_type = "risc0"` uses the same hosted Boundless path as proposal proving.
- The request body is intentionally aligned with old `raiko`'s global body-limit posture; do not
  widen it ad hoc at the route level.

## Release Images

Use the `xtask` release entrypoint for runtime images. It ensures the checked-in guest ELFs are
current, then builds and pushes the runtime image.

```bash
just release-image risc0 tolba-20260310-1013
```

Direct `xtask` entrypoint:

```bash
cargo run -r -p xtask -- release-image risc0 \
  --tag tolba-20260310-1013 \
  --repository us-docker.pkg.dev/evmchain/images/raiko2
```

Avoid ad-hoc `docker build` for releases. The runtime image packages the existing
`crates/guests/elf` artifacts and does not rebuild guest sources by itself.

If `release-image` refreshes tracked guest ELF artifacts and leaves the worktree dirty, it stops
before publishing. Review and commit the updated `crates/guests/elf` artifacts, then rerun the
release command so the image provenance still matches committed repo state.

## Register Guest Digests

Guest builds and image releases do not update verifier trust lists automatically.
When a checked-in guest ELF changes, register the new digests explicitly with `xtask`:

```bash
cargo run -r -p xtask -- register-image --profile hoodi-shasta --backend all
PRIVATE_KEY=0x... cargo run -r -p xtask -- register-image --profile hoodi-shasta --backend all --apply
```

Current behavior:

- `risc0` registrations compute the digest from the current ELF and call
  `setImageIdTrusted(bytes32,bool)`.
- `sp1` registrations derive the current proving key digests from `setup(elf)` and call
  `setProgramTrusted(bytes32,bool)`.
- Boundless program upload is a separate runtime concern and still happens automatically when
  `risc0/boundless` submits a request.

## Boundless Route

To use the boundless-backed RISC0 route, configure:

```toml
[prover]
guest_system = "risc0"
runner = "boundless"

[prover.boundless]
offchain = false
rpc_url = "https://base-rpc.publicnode.com"
signer_key = "0xYOUR_PRIVATE_KEY"
poll_interval_ms = 10000
timeout_ms = 3600000
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
- `rpc.pairs[*].boundless` can override `batch_quoted_mcycles`,
  `aggregation_quoted_mcycles`, and either offer param block for a specific
  `(network, l1_network)` pair. This only affects `risc0/boundless`; SP1 ignores it.
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
If the same request also sets `aggregate = true`, existing proposal proof artifacts take precedence
only when every proposal has a cached artifact for one concrete proof type. Mixed or incomplete
cached artifacts are treated as a cache miss, so the draw still happens exactly once and the
resulting backend is reused for both proposal proving and aggregation.

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
verifier contract used after a network proof is fulfilled.

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
