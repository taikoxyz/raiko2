# Raiko V2

Raiko V2 is a multi-backend prover for Taiko, built on top of [alethia-reth](https://github.com/taikoxyz/alethia-reth). It supports zkVM backends (RISC0, SP1) and TEE-based attestation (TDX).

## Quickstart

```bash
cp config.example.toml config.toml
cargo run -r -p raiko2 -- --config config.toml
```

Configuration is loaded from a TOML file via `--config` (or `RAIKO2_CONFIG`). CLI flags and
environment variables override values from the file.

## Features

- Split into focused crates: `primitives`, `protocol`, `pipeline`, `provider`, `engine`, `queue`, `stateless`, `prover`
- Built on Taiko's `alethia-reth`
- Shasta protocol support (Based Contestable Rollup)
- Prover backends: RISC0, SP1, `agent-risc0`, and TDX
- Native mode: runs locally and outputs public inputs (no zk proof)
- Feature-gated builds: `zkvm` (default) for RISC0/SP1/agent-risc0, `tdx` for TDX attestation

## Feature flags

The `raiko2` binary supports two mutually exclusive feature sets:

| Feature | Default | Enables | Use case |
| ------- | ------- | ------- | -------- |
| `zkvm`  | ✅      | RISC0, SP1, agent-risc0 provers | Standard zkVM proving |
| `tdx`   | ❌      | TDX TEE attestation prover | Intel TDX environments |


> **Note:** `zkvm` and `tdx` are mutually exclusive. Enabling `tdx` disables RISC0/SP1
> compilation entirely, producing a smaller binary suitable for TEE deployments.

## Project structure

```
raiko2/
├── Cargo.toml                 # Workspace root
├── justfile                   # Task entrypoints (build-guest, etc.)
├── bin/                       # Standalone binaries
│   ├── raiko2/                # Prover server (HTTP API + CLI)
│   ├── rpc-proxy/             # RPC proxy service
│   ├── preflight/             # Preflight CLI (build GuestInput)
│   ├── guest-launcher/        # Local guest runner
│   └── witness-check/         # Witness/debug helpers
├── crates/                    # Reusable library crates
│   ├── primitives/            # Core types and traits
│   ├── primitives-shasta/     # Shasta-specific primitives
│   ├── protocol/              # Protocol core types
│   ├── protocol-shasta/       # Shasta types and codecs
│   ├── pipeline/              # Pipeline spec + manifest builder
│   ├── provider/              # Data provider interfaces
│   ├── engine/                # Execution engine
│   ├── queue/                 # Queue + scheduler backends
│   ├── stateless/             # Stateless validation
│   ├── prover/                # Prover backends (risc0, sp1, agent, tdx)
│   └── guests/                # Guest ELF assets (compiled outputs)
├── config/                    # Chain spec lists and config assets
├── docker/                    # Toolchain images
├── docs/                      # Documentation
├── xtask/                     # Automation (guest builds via xtask)
└── guests/                    # Guest program sources (not in workspace)
    ├── common/                # Shared guest abstractions
    ├── risc0/                 # RISC0 guest programs
    └── sp1/                   # SP1 guest programs
```

## How it works

Main flow:

1. `Preflight` builds `GuestInput` (includes `TaikoManifest`) from RPC/provider data.
2. `Validation` runs stateless checks for Shasta.
3. `Encode` serializes `GuestInput` for the prover backend.
4. `Prover` runs the zkVM using the hardfork-selected ELF.
5. `Aggregate` combines proposal proofs (aggregation stage).

Dataflow diagram:

```mermaid
flowchart LR
  P["Provider"] --> PF["Preflight"]
  PF --> VA["Validation"]
  VA --> GI["GuestInput"]
  GI --> EN["Encode"]
  EN --> PR["Prover"]
  PR --> PO["Proof"]
  PO -.-> AG["Aggregate"]
```

Key traits and types:

- `PipelineSpec`: binds `Preflight` + `Validation` + `ManifestBuilder` for a fork.
- `ProverBackend`: selects guest ELFs per `ProofStage` (proposal/aggregation).
- `Pipeline`: hardfork-agnostic preflight + validation flow.
- `Engine`: schedules pipeline stages and prover work.
- `Prover`: encode/prove/aggregate execution (RISC0 / SP1 / agent-risc0 / TDX).

High-level view:

```mermaid
flowchart LR
  C["Client / SDK"] --> A["raiko2 HTTP API"]
  A --> Q["Engine"]
  Q --> SCH["Queue/Scheduler"]
  SCH --> W["Worker"]
  W --> PL["Pipeline"]
  PL --> PF["Preflight"]
  PL --> VA["Validation"]
  VA --> GI["GuestInput"]
  W --> EN["Encode"]
  W --> PR["Prover (RISC0/SP1/TDX)"]
  PR --> PO["Proof"]
  PO --> AG["Aggregation Proof"]
  HF["PipelineSpec"] -.-> PF
  HF -.-> VA
  HF -.-> EN
  PB["ProverBackend"] -.-> PR
```

Pipeline flow:

```mermaid
flowchart TD
  Start([Start]) --> Build[Preflight: build GuestInput]
  Build --> Validate[Validation: stateless checks]
  Validate --> Encode[Encode: serialize GuestInput]
  Encode --> Prove[Prover: run zkVM]
  Prove --> Done([Proof])
```

Request sequence:

```mermaid
sequenceDiagram
  participant C as Client
  participant API as HTTP API
  participant Q as Engine
  participant W as Worker
  participant P as Provider
  participant Z as Prover

  C->>API: POST /v1/proof/proposal
  API->>Q: submit_proposal_proof
  Q->>W: Preflight task
  W->>P: fetch blocks/witnesses/accounts
  W->>Q: store Preflight output
  Q->>W: Validation task
  W->>Q: store GuestInput
  Q->>W: Encode task
  W->>Q: store EncodedInput
  Q->>W: Prove task
  W->>Z: prove(EncodedInput)
  Z-->>W: Proof
  W->>Q: store Proof
  C->>API: GET /v1/proof/{id}
  API-->>C: status + proof
```

## Build

```bash
# Default (zkVM provers: RISC0, SP1, agent-risc0)
cargo build --release -p raiko2

# TDX only (for Intel TDX environments)
cargo build --release -p raiko2 --no-default-features --features tdx
```

## Test

Before opening a PR, run:

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
```

## Build guests

Guest programs live in `guests/` as standalone crates (they are not part of the workspace). Each guest
crate has its own `[patch.crates-io]` in its `Cargo.toml` so RISC0/SP1 dependency versions stay
isolated. `xtask` builds guests using the published Docker toolchain images and copies the resulting
ELFs into `crates/guests/elf` for the host to load. Use `just` unless you have a reason not to.

Prerequisites: `docker` and `just`.

By default, `xtask` uses prebuilt toolchain images, so you don't need local `rzup` / `sp1up` installs:

- RISC0: `RISC0_TOOLCHAIN_IMAGE=ghcr.io/taikoxyz/raiko2/risc0-toolchain:latest`
- SP1: `SP1_TOOLCHAIN_IMAGE=ghcr.io/taikoxyz/raiko2/sp1-toolchain:latest`

Docker builds reuse a persistent Cargo download cache by default:

- Disable: `DOCKER_CARGO_CACHE=none`
- Override the volume name: `DOCKER_CARGO_CACHE_VOLUME=...` (e.g. `raiko2-cargo-sp1`)

To use local toolchains instead, set `RISC0_TOOLCHAIN_IMAGE=none` and/or `SP1_TOOLCHAIN_IMAGE=none`.

```bash
just build-guest all
# or individually:
just build-guest risc0
just build-guest sp1
# Override docker tags/platform if needed (only applies when toolchain images are disabled):
RISC0_TOOLCHAIN_IMAGE=none SP1_TOOLCHAIN_IMAGE=none \
  RISC0_DOCKER_CONTAINER_TAG=<risc0-tag> SP1_DOCKER_TAG=<sp1-tag> \
  DOCKER_DEFAULT_PLATFORM=linux/amd64 \
  just build-guest all
```

To build the SP1 toolchain image locally:

```bash
docker build -f docker/sp1-toolchain/Dockerfile -t raiko2-sp1-toolchain:local docker/sp1-toolchain
SP1_TOOLCHAIN_IMAGE=raiko2-sp1-toolchain:local just build-guest sp1
```

Without `just`:

```bash
cargo run -r -p xtask -- build-guest all
```

## Guest benchmarking

The `bench-guest` task measures guest execution costs (cycles + wall time):

1. (Optional) Run `preflight` to dump a `GuestInput` JSON.
2. Build SP1 guest ELFs with the `bench` feature enabled (docker).
3. Run `guest-launcher` and collect a JSON report (cycles + wall time).

Examples:

```bash
# Build ELFs (docker) + enable cycle tracking, then run
cargo run -r -p xtask -- bench-guest sp1 --input ./test.json --repeat 3

# Reuse prebuilt ELFs (skip docker build)
cargo run -r -p xtask -- bench-guest sp1 --skip-build-guest --input ./test.json --repeat 3

# Generate input via preflight and write an aggregated report
cargo run -r -p xtask -- bench-guest sp1 \
  --rpc-url http://localhost:9545 \
  --l2-chain-id 167000 \
  --proposal-id 3 \
  --repeat 3 \
  --json-out target/bench/guest-report.json
```

If `guest-launcher` panics while deserializing `GuestInput`, the checked-in ELFs are probably stale.
Rebuild them with:

```bash
cargo run -r -p xtask -- build-guest sp1 --bench
```

## Run

```bash
# Start the prover server (zkVM)
cp config.example.toml config.toml
./target/release/raiko2 --config config.toml

# Or with environment variables
RAIKO2_L1_RPC=http://localhost:8545 \
RAIKO2_L2_RPC=http://localhost:9545 \
./target/release/raiko2
```

### TDX mode

When built with `--features tdx`:

```bash
./target/release/raiko2 --config config.toml
```

TDX configuration in `config.toml`:

```toml
[prover]
prover_type = "tdx"

[prover.tdx]
instance_id = 0
socket_path = "/var/tdxs.sock"
```

The TDX prover communicates with the TDX device via the socket at `socket_path`.
`instance_id` identifies the prover instance for proof routing.

## Agent prover

To use `raiko-agent`, set the prover type to `agent-risc0` and configure the agent endpoint:

```toml
[prover]
prover_type = "agent-risc0"

[prover.agent]
url = "http://localhost:9999"
api_key = "optional-api-key"
poll_interval_ms = 1000
timeout_ms = 300000
prover_type = "boundless"
```

The agent handles ELF uploads. raiko2 uploads new ELFs when they change and retries if the agent
returns an "image not uploaded" error.

## Documentation

- [API Documentation](docs/API.md)
- [Migration Guide](docs/MIGRATION.md)
- [Regression Guide](script/regression/README.md)

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

Some files are derived from third-party projects and may include their own copyright and license
notices; those file-level terms apply.
