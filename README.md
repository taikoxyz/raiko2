![Raiko2 — Taiko proof orchestration for Shasta](docs/assets/readme-banner.png)

[![CI status](https://img.shields.io/github/actions/workflow/status/taikoxyz/raiko2/ci.yml?branch=main&label=CI)](https://github.com/taikoxyz/raiko2/actions/workflows/ci.yml)

Home / [Docs](docs/README.md) / [API](docs/API.md) /
[Development](docs/development.md) / [Operations](docs/operations.md) /
[Regression](scripts/regression/README.md) / [Config](config.example.toml)

Raiko2 is a Shasta proof service for Taiko. It builds canonical guest inputs from RPC data,
validates them, runs local or remote proving routes, and exposes an asynchronous,
Hoodi-compatible v3 API.

## At a Glance

- Asynchronous, Hoodi-compatible v3 API for Shasta proofs and aggregation
- Canonical routes: `native/local`, `risc0/local`, `risc0/boundless`, `sp1/local`
- Shasta-first pipeline for preflight, validation, proving, and aggregation
- Config-driven RPC pair allowlist via `rpc.pairs`
- Persisted runtime state, task workdirs, and reusable proof artifacts under `./data/runtime`
- In-process memory queue by default, with an optional Redis-backed queue

## Quickstart

Run the fixture-backed server for a dependency-free local API smoke test:

```bash
cargo run -p raiko2 -- fixture-server --host 127.0.0.1 --port 8087
```

Run the real server with an explicit config file:

```bash
cp config.example.toml config.toml
cargo run -r -p raiko2 -- --config config.toml
```

Configuration is loaded from `--config` or `RAIKO2_CONFIG`. CLI flags and environment variables
override values from the file. The real server checks configured RPC endpoints and hosted prover
capabilities before it starts, so replace example RPC endpoints and set required prover secrets
such as `NETWORK_PRIVATE_KEY` for SP1 network proving or a Boundless signer key for
`risc0/boundless`.

## Core Flow

1. `Preflight` resolves canonical Shasta inputs from L1 and L2 RPC.
2. `Validation` checks request invariants and witness-derived data.
3. `Prover` runs the selected backend and runner.
4. `Aggregate` combines proposal proofs when the request asks for it.

```mermaid
flowchart LR
  RPC["L1/L2 RPC"] --> PF["Preflight"]
  PF --> VA["Validation"]
  VA --> PR["Prover"]
  PR --> AG["Aggregate"]
  PR --> API["Task API"]
  AG --> API
```

## API Compatibility

- `POST /v3/proof/batch/shasta` registers proposal proof work and, when `aggregate=true`, also
  registers the aggregation work for that batch.
- `POST /v3/proof/aggregate` registers aggregation work from externally supplied proposal proofs.
- Single-proof aggregation is allowed for compatibility with existing `raiko` clients.
- Shasta manifests support `blob_proof_type = "proof_of_equivalence"` only; legacy
  `kzg_versioned_hash` manifests are rejected.
- Hosted SP1 proposal proving emits Compressed proofs and SP1 aggregation emits Plonk proofs.
- `proof_type=risc0` resolves to the server's configured RISC Zero runner. The `risc0/boundless`
  route submits to Boundless and exposes Boundless quote metadata.

## Routes

- `native/local` executes the proving pipeline locally and returns public inputs
  instead of a zk proof.
- `risc0/local` generates RISC Zero proofs locally.
- `risc0/boundless` submits RISC Zero proving directly to Boundless from the `raiko2` process.
- `sp1/local` runs the SP1 flow; request-scoped SP1 settings also cover execute
  mode and network proving.

## Repository Map

- `bin/raiko2`: HTTP server and CLI
- `crates/pipeline`: preflight, manifest building, and validation wiring
- `crates/prover`: prover backends and aggregation adapters
- `xtask`: guest build, verifier registration, benchmarking, and release automation

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

Some files are derived from third-party projects and may include their own copyright and license
notices; those file-level terms apply.
