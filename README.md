![Raiko2 — Taiko proof orchestration for Shasta](docs/assets/readme-banner.png)

[![CI status](https://img.shields.io/github/actions/workflow/status/taikoxyz/raiko2/ci.yml?branch=main&label=CI)](https://github.com/taikoxyz/raiko2/actions/workflows/ci.yml)

Home / [Docs](docs/README.md) / [Architecture](docs/architecture.md) / [API](docs/API.md) /
[Development](docs/development.md) / [Operations](docs/operations.md) /
[Regression](scripts/regression/README.md) / [Config](config.example.toml)

Raiko2 is a Shasta proof service for Taiko. It builds canonical guest inputs from RPC data,
validates them, runs local or remote proving routes, and exposes a typed v4 API for
asynchronous proposal-side proof requests.

## At a Glance

- Typed v4 proposal-side proof endpoint
- Canonical routes: `native/local`, `risc0/local`, `risc0/network`, `sp1/local`, `sp1/network`
- Default binaries include RISC Zero local/network proving and SP1 proving
- Optional remote SGX routes for configured external prover providers
- Shasta-first pipeline for preflight, validation, proving, and aggregation
- Config-driven RPC pair allowlist and optional L1 beacon overrides via `rpc.pairs`
- Exactly one live instance per isolated runtime namespace; replacements never overlap
- GCS for durable operation or local-only memory mode, with no cross-namespace data sharing
- In-process queue projected from the namespaced runtime store

## Quickstart

Run the fixture-backed server for a dependency-free local API smoke test:

```bash
cargo run -p raiko2 --features fixture-server -- fixture-server --host 127.0.0.1 --port 8087
```

Use this only for local API-surface smoke testing, request/response contract checks, and simple
task/report workflow validation when you do not want real RPC or prover dependencies. It is not a
substitute for preflight correctness, remote-provider integration, or full proposal regression.

Run the real server with an explicit config file:

```bash
cp config.example.toml config.toml
cargo run -r -p raiko2 -- --config config.toml
```

Configuration is loaded from `--config` or `RAIKO2_CONFIG`. CLI flags and environment variables
override values from the file. The real server checks configured RPC endpoints and hosted prover
capabilities before it starts, so replace example RPC endpoints and set required prover secrets
such as `NETWORK_PRIVATE_KEY` for SP1 network proving or a Boundless signer key when the selected
route is `risc0/network`. The prover loads guest ELF files from `RAIKO2_GUEST_ELF_DIR` when set,
otherwise from `crates/guests/elf`. For unreleased testing, build ELFs locally with
`just build-guest all`. Packaged deployments can download released ELF assets with
`cargo run -r -p xtask -- download-guest-elves --tag <tag> --dir <guest-elf-dir>`.

## Architecture And Operator Contract

This README is the normative source for Raiko2 architecture and operator workflow. The detailed
[Architecture](docs/architecture.md) and [Operations](docs/operations.md) documents expand this
contract; if they conflict with this section, this README governs.

The runtime is governed by these invariants:

1. The configured runtime store is authoritative for task state, artifact registration, and remote
   submission checkpoints. The in-process queue is an execution projection of that state.
2. Each `(runtime.environment, runtime.namespace)` has exactly one live process. Replacements never
   overlap, and the application has no distributed owner lease, owner epoch, or ownership heartbeat.
3. Namespaces are isolated persistence domains. They never share tasks, artifacts, checkpoints, or
   invalidation markers, although roots inside one namespace may reuse one canonical artifact.
4. The runtime fence covers every task mutation and external-store write for the whole instance and
   namespace. Inactive or draining runtimes reject new mutations and wait for in-flight writes. The
   only draining-time write is the request-ID checkpoint authorized by a provider-submission permit
   acquired before the fence closed; it must finish within the bounded shutdown deadline.
5. Proof computation is not task completion. Completion requires a normalized proof to be durably
   published, registered, readable, and synchronized to the runtime root.
6. Proof manifests are create-only and first-valid-wins. Content is immutable and addressed by
   SHA-256; invalidation binds to one manifest generation and content hash.
7. Remote proving resumes a request identifier only after its submission checkpoint is durable.
   Request-level retry settings may lower, but never raise, operator-owned limits.
8. Durable deployments use one GCS runtime store; memory mode is explicitly ephemeral. The service
   does not dual-write or automatically fail over between runtime stores.
9. A replacement starts only after the old process has stopped admissions, drained work, stopped
   workers, and exited. Deployment configuration must enforce this non-overlapping sequence.
10. Each runtime task lifetime has an immutable `incarnation_id`. It rejects a delayed worker
    checkpoint or cancellation callback after a replacement reuses the same deterministic task ID.
    Pending proof outboxes persist the exact eligible incarnations, and the completion permit carries
    that set through the artifact/root CAS; it is not a namespace owner epoch, lease, or distributed
    lock.
11. Each scheduler lease also carries a non-reused local token. This prevents remove/recreate ABA
    from accepting an old completion even when task ID, worker label, and attempt number repeat.
    The runtime additionally issues an execution permit mapping task IDs to the incarnations present
    when execution starts. Proof checkpointing may add a distinct late-joining shared root, but never
    a replacement incarnation for an already captured task ID. The permit does not authorize runtime
    writes; the namespace lifecycle fence remains authoritative.

## Core Flow

The detailed runtime lifecycle, publication transaction, recovery flow, and deployment sequence are
illustrated in [Architecture](docs/architecture.md).

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

- V4 is the active public API. Legacy v3 and `/proof/*` compatibility routes are not mounted by
  the server while clients are using v4.
- The legacy v3 contract remains documented and covered by compatibility tests while the code is
  still present.
- Single-proof aggregation is allowed for compatibility with existing `raiko` clients.
- Shasta manifests support `blob_proof_type = "proof_of_equivalence"` only; legacy
  `kzg_versioned_hash` manifests are rejected.
- Public batch request proof types are `native`, `risc0`, `sp1`, `sgx`, `sgxgeth`, and
  admission-time `zk_any` for proposal sampling. `native` is accepted only for internal native
  regression when the server route is `native/local`.
- Hosted SP1 proposal proving emits Compressed proofs and SP1 aggregation emits Plonk proofs.
- `proof_type=risc0` resolves to the server's configured RISC Zero prover type. The
  `prover_type=network` path submits to Boundless and exposes Boundless quote metadata; Boundless
  is not a separate proof type.
- `proof_type=boundless` is not accepted; use `proof_type=risc0` with the server configured for
  `risc0/network` when targeting Boundless.

## Routes

- `native/local` executes the proving pipeline locally and returns public inputs
  instead of a zk proof.
- `risc0/local` generates RISC Zero proofs locally.
- `risc0/network` submits RISC Zero proving directly to Boundless from the `raiko2` process.
- `sp1/local` and `sp1/network` select the SP1 pipeline. The task `prover_type` reports whether
  SP1 ran in `mock`, `local`, or `network` mode.
- `sgx/remote` submits Shasta proving to the dedicated remote SGX runtime. This repo now ships
  `raiko2-sgx-prover` for `proof_type=sgx`; that runtime can run in `tee` or `native` mode
  without changing the remote API. `proof_type=sgxgeth` is served by an external remote prover
  implementation such as `gaiko2` over the same remote protocol.
- `docker/docker-compose.sgx.regression.yml` starts both SGX remote services and can optionally
  add a dockerized `raiko2` for regression work.

## Remote Prover Conformance

`raiko2` owns the canonical remote prover request fixtures under:

- `tests/fixtures/remote_prover/shasta_aggregate_request_v1_single_fixture_proof.json`

The aggregate request fixture is the strict protocol golden for:

- `raiko2-shasta-aggregate-request-v1`

Run the ignored black-box conformance harness against a provider endpoint with:

```bash
RAIKO2_REMOTE_PROVER_BASE_URL=http://127.0.0.1:8080 \
cargo test -p raiko2-prover --no-default-features \
  --test remote_prover_conformance -- --ignored --nocapture
```

The harness builds the proposal request from the shared Shasta `GuestInput` fixture and posts it to:

- `POST /prove/shasta`

This harness targets providers whose `/prove/shasta` input is the v1
`raiko2-shasta-request-v1` packet with `payload.guest_input`. `raiko2-sgx-prover` consumes the
same request shape and runs the Shasta guest validation path before signing.

It then builds a live aggregate request from the returned proposal proof and posts that derived
request to:

- `POST /prove/shasta-aggregate`

This keeps aggregate conformance provider-agnostic while preserving provider identity continuity
for implementations that require aggregate subproofs to come from the current prover instance.

The harness verifies the provider returns a `raiko2-proof-v1` envelope with an `input` value that
is self-consistent with the submitted proof carry data.

For the first external provider migration, see
[`docs/gaiko2-remote-prover-integration.md`](docs/gaiko2-remote-prover-integration.md).

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
