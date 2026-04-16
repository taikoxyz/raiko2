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

## Generate A Latest Proposal Request

Use the new `xtask` helper to discover the latest onchain Shasta proposal and emit a ready-to-post
`/v3/proof/batch/shasta` JSON body.

Print a mainnet request to stdout:

```bash
cargo run -r -p xtask -- latest-proposal-request --profile taiko-mainnet
```

Write a Hoodi request to a file with explicit RPC overrides:

```bash
cargo run -r -p xtask -- latest-proposal-request \
  --profile taiko-hoodi \
  --l1-rpc-url https://ethereum-hoodi-rpc.publicnode.com \
  --l2-rpc-url http://34.172.70.130:8545 \
  -o target/latest-proposal/hoodi.json
```

The helper scans recent L1 `Proposed` logs to find the newest proposal, then scans recent L2 block
headers to recover the contiguous `l2_block_numbers` range for that proposal.

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

`build-guest` only rebuilds checked-in ELF artifacts under `crates/guests/elf`.
It does not register verifier trust-list entries or update any external program registry.

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

If a guest ELF changes and the target environment relies on onchain verifier trust lists,
register the new digests explicitly:

```bash
cargo run -r -p xtask -- register-image --profile hoodi-shasta --backend all
PRIVATE_KEY=0x... cargo run -r -p xtask -- register-image --profile hoodi-shasta --backend all --apply
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
[scripts/regression/README.md](../scripts/regression/README.md).

Setup and run:

```bash
scripts/regression/prepare_regression.sh
python scripts/regression/shasta_regression.py \
  --config scripts/regression/config/shasta_regression_devnet.json \
  --count 3

python scripts/regression/shasta_regression.py \
  --config scripts/regression/config/shasta_regression_devnet.json \
  --count 3 \
  --discover-only
```

Artifacts are written under `test/regression/shasta/`.
Use `--discover-only` when you need the latest completed proposal spans for live proof tests without re-deriving them from the latest tip by hand.
