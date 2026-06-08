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
cargo test -p raiko2-primitives -p raiko2-primitives-shasta -p raiko2-protocol -p raiko2-protocol-shasta
cargo test -p raiko2-provider -p raiko2-pipeline -p preflight
cargo test -p raiko2-queue -p raiko2-runtime
```

## Alethia Reth Integration Workflow

Raiko2 consumes [taikoxyz/alethia-reth](https://github.com/taikoxyz/alethia-reth) from upstream
`main`. The Cargo manifests pin reviewed `main` commits with explicit `rev` values, and the lockfiles
record the exact resolved source used by workspace and guest builds.

Development rules:

- Land every alethia-reth change required by raiko2 on alethia-reth `main` before updating raiko2.
- Point raiko2 alethia-reth Cargo dependencies at explicit `rev` pins from `origin/main`.
- Update raiko2 manifests and lockfiles together when adopting a new alethia-reth `main` commit.
- Rebuild guest ELFs with the normal `xtask`/`just` entrypoints when guest-facing dependencies change.

Do not keep raiko2-required alethia-reth fixes only in stale local branches, one-off PR branches, or
raiko2 workaround layers. Do not route guest or no-std fixes through reth `test-utils` features or
dev-only APIs.

RISC0 guest `getrandom` configuration belongs in the guest build path managed by `xtask`. Do not add a
second Cargo config source for `getrandom_backend`.

## Fixture-Backed API Testing

For manual HTTP testing without external RPC dependencies, run the fixture server:

```bash
cargo run -p raiko2 --features fixture-server -- fixture-server --host 127.0.0.1 --port 8087
```

This fixture-backed server is intended for:

- API upgrade smoke tests that only need stable request/response behavior
- local validation of `/v3/proof/batch/shasta` and `/v3/proof/report` wiring
- development without live L1/L2 RPC or a real prover backend

Do not use it as evidence for:

- preflight correctness
- remote prover integration
- end-to-end proposal regression on a real network window

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
  --l2-rpc-url http://<l2-rpc-host>:8545 \
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
The host loads those files from that fixed path at process startup; they are not embedded into
the `raiko2` binary. Set `RAIKO2_GUEST_ELF_DIR` when running a packaged binary from a layout that
differs from the source tree. `build-guest` does not register verifier trust-list entries or
update any external program registry.

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
cargo run -r -p xtask -- register-image --profile mainnet-shasta --backend all
PRIVATE_KEY=0x... cargo run -r -p xtask -- register-image --profile mainnet-shasta --backend all --apply
```

This `register-image` flow only covers zk guest digests (`risc0` image IDs and `sp1` verifier
digests). SGX registration is separate: read `mr_enclave` from the baked
`/opt/raiko2-sgx/etc/attestation.raiko2.json` file in the built `raiko2-sgx` image and use your
external SGX verifier tooling, such as the `taiko-mono` SGX verifier scripts, to register it.

## Guest Benchmarking

`bench-guest` measures execution metadata, SP1 prover gas, cycles, and wall time for guest runs.
The checked-in sample input lives at
`tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json` and was
generated from a real Shasta preflight.

Run one cached `GuestInput` and write an aggregate JSON report:

```bash
cargo run -r -p xtask -- bench-guest sp1 \
  --input ./tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json \
  --repeat 3 \
  --json-out /tmp/sp1-prover-gas.json
```

Reuse prebuilt ELFs after a prior `build-guest sp1 --bench`:

```bash
cargo run -r -p xtask -- bench-guest sp1 \
  --skip-build-guest \
  --input ./tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json \
  --repeat 3 \
  --json-out /tmp/sp1-prover-gas.json
```

For live Shasta proposals, first use
`scripts/regression/stress_shasta_proposal.py --discover-only --proposal-out` to capture the full
proposal tuple, then run `preflight` once and reuse the generated `GuestInput` here. Keep repeated
`bench-guest` runs on cached inputs so prover-gas research does not depend on live RPC.

`bench-guest` can also run `preflight` for one input when the full tuple is known:

```bash
cargo run -r -p xtask -- bench-guest sp1 \
  --network taiko_dev \
  --l1-network taiko_dev_l1 \
  --proposal-id 730 \
  --l1-inclusion-block-number 20802 \
  --last-anchor-block-number 20734 \
  --l2-start 239062 \
  --l2-end 239445 \
  --skip-build-guest \
  --repeat 1 \
  --json-out /tmp/proposal-730.sp1-prover-gas.json
```

Run a suite of cached `GuestInput` files:

```json
{
  "cases": [
    {
      "name": "mainnet-proposal-2222",
      "input": "./tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json",
      "proof_type": "sp1"
    }
  ]
}
```

```bash
cargo run -r -p xtask -- bench-guest sp1 \
  --skip-build-guest \
  --suite ./bench/sp1-suite.json \
  --repeat 3 \
  --json-out /tmp/sp1-suite-report.json
```

If the checked-in SP1 ELF is stale, rebuild it with the benchmark feature:

```bash
cargo run -r -p xtask -- build-guest sp1 --bench
```

This command rewrites the checked-in SP1 ELF artifacts under `crates/guests/elf`. Treat those diffs
as generated artifact refreshes, not source edits. If a `--skip-build-guest` run fails with a guest
panic such as `unexpected hard_forks`, rebuild the SP1 bench ELF so the embedded chain specs match
the current checkout and input.

The JSON report keeps each `guest-launcher` run under `cases[].runs[]` and summarizes each case
under `cases[].summary`. The `gas` field is SP1 prover gas from `ExecutionReport::gas()`, not EVM
gas. The summary also preserves wall time, instruction count, syscall count, touched memory
addresses, total cycle-tracker cycles, and per-label cycle medians.

## Opcode Prover-Gas Experiments

The experiment scaffold under `experiments/opcode-gas/` is for SP1 opcode and precompile
prover-gas research. It generates synthetic opcode-lab and precompile-lab inputs, runs cached local execute cases
through `target/release/guest-launcher`, and fits marginal coefficients from JSONL reports.

Use the repository Python venv because it provides Python 3.11 `tomllib`:

```bash
cargo run -r -p xtask -- build-guest sp1 --bench
cargo build -r -p guest-launcher --features sp1-sdk/profiling

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate \
  --manifest experiments/opcode-gas/manifests/sp1-smoke.toml \
  --out /tmp/raiko2-opcode-gas/fixtures

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run \
  --fixtures /tmp/raiko2-opcode-gas/fixtures \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_opcode_lab.elf \
  --precompile-elf crates/guests/elf/sp1_precompile_lab.elf \
  --out /tmp/raiko2-opcode-gas/raw-runs.jsonl

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit \
  --runs /tmp/raiko2-opcode-gas/raw-runs.jsonl \
  --out /tmp/raiko2-opcode-gas/report
```

V1 keeps the output compatible with alethia-reth's one-dimensional Uzen zk gas table. Warm/cold
storage access, argument-dependent precompile costs, and memory-size sweeps are tracked as future
dimensions, not V1 coefficients.

The current `sp1-opcode-lab` guest is an experiment-only mini bytecode interpreter. The
`sp1-precompile-lab` guest supports direct `identity(0x04)` and `sha256(0x02)` body measurements.
Both deliberately remove Shasta proposal/block noise and avoid live RPC, but they are not yet full
alethia-reth/revm execution.

The run command groups generated `guest-input.json` files by lab stage and batches each group into
one launcher process via `--input-list` and `--jsonl-out`. This is important for research loops
because SP1 profiling startup is much more expensive than the tiny lab execute bodies.

## GuestInput Replay

Checked-in Shasta GuestInput fixtures live under `test/guest_inputs/shasta/<network>/`.
Use `preflight` to capture a native fixture after live RPC preflight succeeds. The checked-in
`taiko_hoodi/smoke` suite currently contains proposals `17460` and `17462`; proposal `17461`
was skipped because public RPC witness fetching was unstable during capture.

```bash
cargo run -r -p preflight -- \
  --rpc-url http://<l2-rpc-host>:8545 \
  --l1-rpc-url https://ethereum-hoodi-rpc.publicnode.com \
  --l2-chain-id 167013 \
  --l1-chain-id 560048 \
  --proposal-id 17460 \
  --l1-inclusion-block-number 2668326 \
  --last-anchor-block-number 0 \
  --l2-start 7165709 \
  --l2-end 7165900 \
  --proof-type native \
  --save-guest-input \
  --network taiko_hoodi \
  --rpc-retry-max-attempts 8 \
  --rpc-timeout-ms 120000 \
  --pretty
```

Replay one proposal, a suite, or every fixture for a network without RPC:

```bash
cargo run -r -p xtask -- replay-guest-input --network taiko_hoodi --suite smoke
cargo run -r -p xtask -- replay-guest-input --network taiko_hoodi --proposal 17460
cargo run -r -p xtask -- replay-guest-input --network taiko_hoodi --all
```

Suites are tracked as `test/guest_inputs/shasta/<network>/suites/<name>.json`:

```json
{
  "network": "taiko_hoodi",
  "name": "smoke",
  "proposals": [17460, 17462]
}
```

Masaya also carries a checked-in `shasta_unzen_transition` suite with proposals `25125`,
`25126`, and `25127`. Use it as the fixed fork-transition regression case around the
`SHASTA -> UNZEN` boundary: `25125` and `25126` are pre-fork controls, while `25127`
spans the transition window itself.

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
