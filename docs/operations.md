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

CLI flags and environment variables override values loaded from the file. `--l1-rpc` and
`--l2-rpc` remain available as overrides, but they only apply when the config defines exactly one
`rpc.pairs` entry.

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

## Release Images

Use the `xtask` release entrypoint for runtime images. It rebuilds guest ELFs, builds the runtime
image, pushes it, and prints the rollout command.

```bash
just release-image risc0 tolba-20260310-1013
```

Direct `xtask` entrypoint:

```bash
cargo run -r -p xtask -- release-image risc0 \
  --tag tolba-20260310-1013 \
  --repository us-docker.pkg.dev/evmchain/images/raiko2 \
  --namespace tolba-raiko2-host \
  --deployment raiko2 \
  --container raiko2
```

Avoid ad-hoc `docker build` for releases. The runtime image packages the existing
`crates/guests/elf` artifacts and does not rebuild guest sources by itself.

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
- Proposal requests use `prover.boundless.batch_quoted_mcycles` when it is set. Otherwise,
  `batch_quote_strategy = "raiko_agent"` rounds evaluated user cycles up to the next `1000`
  mcycles with a `2000` mcycle floor.
- Aggregation requests quote `200` mcycles.
- The local dry-run validates guest execution and prepares the request journal.

Optional `zk_any` request sampling is configured at the server level:

```toml
[prover.zk_any.sp1]
probability = 0.20
per_day = 100

[prover.zk_any.risc0]
probability = 0.30
per_day = 0
```

When a client submits `proof_type = "zk_any"` to `/v3/proof/batch/shasta`, the server draws once
at admission time and either routes the request to `sp1` / `risc0` or returns
`data.status = "zk_any_not_drawn"` without registering a task.

## RPC and Witness Expectations

`rpc.pairs[*].l2_rpc` should ideally point to a witness-capable endpoint that supports
`debug_executionWitness` for the best latency envelope.

For supported Taiko chain specs, `raiko2` can fall back to on-the-spot witness construction when
the endpoint does not expose `debug_executionWitness`, but that path is materially slower.

If the upstream L2 does not expose that method and you need predictable proving latency, place
[`zeth-rpc-proxy`](../bin/rpc-proxy) in front of it and point `rpc.pairs[*].l2_rpc` at the
proxy instead.

## Health and Readiness

- `GET /health`: basic process health
- `GET /ready`: configured L1/L2 RPC chain-ID readiness, queue readiness, and prerequisite checks
  for the configured default prover route
