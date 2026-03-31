# Development Guide

This guide covers local development, fixture-backed API testing, guest builds, and benchmarking.
API request and response contracts live in [API.md](API.md), and the canonical config shape lives in
[`config.example.toml`](../config.example.toml).

See also:

- [Docs index](README.md)
- [README](../README.md) for the project overview
- [Operations guide](operations.md) for runtime and deployment workflows

## Local Workflow

```bash
cp config.example.toml config.toml
cargo run -r -p raiko2 -- --config config.toml
```

Recommended checks before opening a PR:

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
```

## Fixture-Backed API Testing

For manual HTTP testing without external RPC dependencies, run the fixture server:

```bash
cargo run -p raiko2 -- fixture-server --host 127.0.0.1 --port 8087
```

Submit an asynchronous v3 request:

```bash
curl -X POST http://127.0.0.1:8087/v3/proof/batch/shasta \
  -H 'content-type: application/json' \
  -d '{
    "proposals": [{
      "proposal_id": 3,
      "l1_inclusion_block_number": 1,
      "l2_block_numbers": [3],
      "last_anchor_block_number": 0
    }],
    "aggregate": false,
    "proof_type": "sp1",
    "network": "taiko_dev",
    "l1_network": "ethereum",
    "sp1": {
      "mode": "execute",
      "prover": "local"
    }
  }'
```

Query the resulting task:

```bash
curl http://127.0.0.1:8087/v3/tasks/<task_id>
```

`sp1.mode=execute` completes without a zk proof and stores the execution report under
`proposals[].extra_data.sp1`.

## Build Guest ELFs

Guest programs live under `guests/` as standalone crates. Use `just` by default:

```bash
just build-guest all
just build-guest risc0
just build-guest sp1
```

Direct `xtask` entrypoint:

```bash
cargo run -r -p xtask -- build-guest all
```

Prerequisites:

- `docker`
- `just`

Repo-managed local toolchain images are used by default:

- `RISC0_TOOLCHAIN_IMAGE=raiko2-risc0-toolchain:local`
- `SP1_TOOLCHAIN_IMAGE=raiko2-sp1-toolchain:local`

To disable toolchain images and use local toolchains instead:

```bash
RISC0_TOOLCHAIN_IMAGE=none SP1_TOOLCHAIN_IMAGE=none \
  cargo run -r -p xtask -- build-guest all
```

## Guest Benchmarking

`bench-guest` measures execution metadata, cycles, and wall time for guest runs.

Typical workflow:

```bash
cargo run -r -p xtask -- bench-guest sp1 --input ./test.json --repeat 3
```

Reuse prebuilt ELFs:

```bash
cargo run -r -p xtask -- bench-guest sp1 --skip-build-guest --input ./test.json --repeat 3
```

If the checked-in SP1 ELF is stale, rebuild it with the benchmark feature:

```bash
cargo run -r -p xtask -- build-guest sp1 --bench
```

## Regression Harness

The file-based Shasta regression flow lives in
[script/regression/README.md](../script/regression/README.md).

Setup and run:

```bash
script/regression/prepare_regression.sh
python script/regression/shasta_regression.py \
  --config script/regression/config/shasta_regression_devnet.json \
  --count 3
```

Artifacts are written under `test/regression/shasta/`.
