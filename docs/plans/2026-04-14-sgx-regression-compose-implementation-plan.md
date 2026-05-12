# SGX Regression Compose Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a single SGX-focused regression compose stack that starts the `raiko2-sgx-prover`
and external `gaiko2` SGX server by default, with an optional `raiko2` service profile for
full-docker regression runs.

**Architecture:** Keep the runtime topology close to historical `raiko`: two remote SGX servers
are long-lived infrastructure, while the `raiko2` main service is optional. To make this usable
for both local CLI and dockerized `raiko2`, add CLI/env overrides for the remote prover base URL
instead of hard-coding a dedicated regression config file per backend.

**Tech Stack:** Docker Compose, Rust clap/config loading, existing `raiko2-sgx-prover` image,
external `gaiko2` tee image, Markdown operator docs.

---

### Task 1: Add configurable remote prover overrides to `raiko2`

**Files:**
- Modify: `bin/raiko2/src/cli.rs`
- Modify: `bin/raiko2/src/config/mod.rs`
- Test: `bin/raiko2/src/config/mod.rs`

**Step 1: Write the failing config test**

Add a config-loading test that:

- writes a minimal config file with `prover.guest_system = "sgx"` and `runner = "remote"`
- leaves `[prover.gaiko2]` empty or with a dummy URL in the file
- sets `RAIKO2_GAIKO2_BASE_URL=http://127.0.0.1:19090`
- sets `RAIKO2_GAIKO2_TIMEOUT_MS=12345`
- loads the config through `Cli::parse_from(...)`
- asserts `config.prover.gaiko2.base_url == "http://127.0.0.1:19090"`
- asserts `config.prover.gaiko2.timeout_ms == 12345`

**Step 2: Run the focused test to verify it fails**

Run: `cargo test -p raiko2 test_sgx_remote_route_env_overrides_gaiko2_config`
Expected: FAIL because the CLI/env overrides do not exist yet.

**Step 3: Add CLI/env wiring**

In `bin/raiko2/src/cli.rs`, add:

- `--gaiko2-base-url` with env `RAIKO2_GAIKO2_BASE_URL`
- `--gaiko2-timeout-ms` with env `RAIKO2_GAIKO2_TIMEOUT_MS`

In `Config::load`, apply those overrides after file load and before validation.

**Step 4: Run the focused test to verify it passes**

Run: `cargo test -p raiko2 test_sgx_remote_route_env_overrides_gaiko2_config`
Expected: PASS

### Task 2: Add the unified SGX regression compose assets

**Files:**
- Create: `docker/docker-compose.sgx.regression.yml`
- Create: `docker/.env.sgx.regression.sample`
- Possibly modify: `.gitignore`

**Step 1: Model the remote-server topology**

Create a single compose file with:

- `raiko2-sgx-init` profile `init`
- `raiko2-sgx` default service
- `gaiko2-sgxgeth-init` profile `init`
- `gaiko2-sgxgeth` default service
- `raiko2` profile `raiko2`

Default startup should bring up only the two remote SGX servers.

**Step 2: Follow the historical port conventions**

Use defaults that mirror old `raiko` operator expectations:

- host `9090 -> raiko2-sgx:8080`
- host `8090 -> gaiko2-sgxgeth:8080`
- host `8080 -> raiko2:8080` when the optional profile is enabled

**Step 3: Keep `gaiko2` external**

Do not build `../gaiko2` from this compose by default.

Use:

- `image: ${GAIKO2_SGXGETH_IMAGE:-gaiko2-tee:latest}`

and wire:

- `GAIKO2_PROVING_MODE=tee`
- `GAIKO2_CONFIG_DIR=/var/lib/gaiko2/config`
- `GAIKO2_SECRET_DIR=/var/lib/gaiko2/secrets`
- `GAIKO2_PORT=8080`
- `GAIKO2_FORK=shasta`
- `GAIKO2_INSTANCE_ID` pass-through

**Step 4: Make optional `raiko2` runs configurable**

The optional `raiko2` service should:

- reuse the existing repo `Dockerfile`
- mount `docker/config.compose.toml`
- set `RAIKO2_PROVER=sgx/remote`
- set `RAIKO2_GAIKO2_BASE_URL=${RAIKO2_REMOTE_PROVER_URL:-http://raiko2-sgx:8080}`
- accept standard RPC envs (`RAIKO2_L1_RPC`, `RAIKO2_L2_RPC`, chain IDs, queue knobs)

This lets operators switch the target backend by changing one env var instead of swapping config
files.

**Step 5: Add the env template**

Add `docker/.env.sgx.regression.sample` with:

- image tags for `raiko2`, `raiko2-sgx`, and `gaiko2-sgxgeth`
- RPC URLs and chain IDs
- host config/secret directories for both SGX services
- PCCS host
- default remote target URL for the optional `raiko2` service
- comments for:
  - local CLI against host ports
  - docker `raiko2` against service names
  - switching `RAIKO2_REMOTE_PROVER_URL` between `raiko2-sgx` and `gaiko2-sgxgeth`

### Task 3: Document the regression startup flows

**Files:**
- Modify: `docs/operations.md`
- Modify: `scripts/regression/README.md`
- Modify: `README.md`

**Step 1: Document the compose entrypoint**

Add an SGX regression section that explains:

- default compose startup launches both SGX remote services
- `--profile init` bootstraps both tee services
- `--profile raiko2` adds the optional `raiko2` service

Include exact commands:

```bash
cp docker/.env.sgx.regression.sample docker/.env.sgx.regression
docker compose --env-file docker/.env.sgx.regression -f docker/docker-compose.sgx.regression.yml --profile init up raiko2-sgx-init gaiko2-sgxgeth-init
docker compose --env-file docker/.env.sgx.regression -f docker/docker-compose.sgx.regression.yml up -d
docker compose --env-file docker/.env.sgx.regression -f docker/docker-compose.sgx.regression.yml --profile raiko2 up -d raiko2
```

**Step 2: Document local CLI usage**

Add a local CLI example showing:

```bash
RAIKO2_CONFIG=docker/config.compose.toml \
RAIKO2_PROVER=sgx/remote \
RAIKO2_GAIKO2_BASE_URL=http://127.0.0.1:9090 \
cargo run -r -p raiko2 -- --config docker/config.compose.toml
```

Also note that today this chooses one remote backend at a time even though the compose stack starts
both.

**Step 3: Document regression usage**

Update `scripts/regression/README.md` to explain that:

- the compose stack is the recommended remote-SGX baseline
- the file-based regression harness still only supports `native` and `sp1`
- API-driven SGX regression can run against either the local CLI or dockerized `raiko2`

### Task 4: Verify the compose and config behavior

**Files:**
- Verify only

**Step 1: Run Rust formatting**

Run: `cargo fmt --all`
Expected: PASS

**Step 2: Run the focused `raiko2` config tests**

Run: `cargo test -p raiko2 test_sgx_remote_route_`
Expected: PASS, including the new env override test.

**Step 3: Run SGX compose rendering**

Run: `docker compose --env-file docker/.env.sgx.regression.sample -f docker/docker-compose.sgx.regression.yml config`
Expected: PASS

**Step 4: Verify shell syntax if helper scripts change**

Run: `bash -n docker/sgx-entrypoint.sh docker/sgx-bootstrap.sh`
Expected: PASS

**Step 5: Commit**

```bash
git add bin/raiko2/src/cli.rs bin/raiko2/src/config/mod.rs docker/docker-compose.sgx.regression.yml docker/.env.sgx.regression.sample docs/operations.md scripts/regression/README.md README.md docs/plans/2026-04-14-sgx-regression-compose-implementation-plan.md
git commit -m "feat: add sgx regression compose stack"
```
