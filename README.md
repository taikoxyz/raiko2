# Raiko V2

Raiko V2 is the next-generation zkVM prover for Taiko, built on top of [alethia-reth](https://github.com/taikoxyz/alethia-reth).

## Features

- **Modular Architecture**: Clean separation between primitives, protocol, pipeline, provider, engine, and prover
- **alethia-reth Integration**: Uses Taiko's new reth fork for improved performance
- **Shasta Protocol**: Native support for Taiko Shasta (Based Contestable Rollup)
- **zkVM Provers**: Support for RISC0 and SP1 provers

## Project Structure

```
raiko2/
├── Cargo.toml          # Workspace root
├── crates/
│   ├── primitives/     # Core types and traits
│   ├── protocol/       # Shasta protocol implementation
│   ├── pipeline/       # Pipeline spec + manifest builder
│   ├── provider/       # Data provider interfaces
│   ├── engine/         # Execution engine
│   ├── stateless/      # Stateless validation
│   ├── prover/         # zkVM prover adapters (risc0, sp1)
│   └── guests/         # Guest ELF assets (compiled outputs)
├── bin/
│   ├── raiko2/         # Main binary (HTTP server + CLI)
│   └── rpc-proxy/      # RPC proxy service
├── docs/               # Documentation
├── guests/             # Guest program sources (out-of-workspace)
│   ├── common/         # Shared guest abstractions
│   ├── risc0/          # RISC0 guest programs
│   └── sp1/            # SP1 guest programs
└── script/             # Build scripts
```

## Architecture

Core flow:

1. **Preflight** builds `GuestInput` from RPC/provider data.
2. **Validation** checks `GuestInput` (stateless validation for Shasta).
3. **Prover** runs the zkVM using hardfork-selected ELF.

Dataflow diagram:

```mermaid
flowchart LR
  P["Provider"] --> PF["Preflight"]
  PF --> VA["Validation"]
  VA --> GI["GuestInput"]
  GI --> PR["Prover"]
  PR --> PO["Proof"]
```

Key abstractions:

- `PipelineSpec`: binds `Preflight` + `Validation` + ELF selection for a fork.
- `Engine`/`Pipeline`: hardfork-agnostic orchestration.
- `Prover`: backend execution (RISC0 / SP1).

Architecture diagram:

```mermaid
flowchart LR
  C["Client / SDK"] --> A["raiko2 HTTP API"]
  A --> Q["Engine"]
  Q --> PL["Pipeline"]
  PL --> PF["Preflight"]
  PL --> VA["Validation"]
  VA --> GI["GuestInput"]
  Q -->|Prove| PR["Prover (RISC0/SP1)"]
  PR --> PO["Proof"]
  HF["PipelineSpec"] -.-> PF
  HF -.-> VA
  HF -.-> PR
```

Pipeline flow:

```mermaid
flowchart TD
  Start([Start]) --> Build[Preflight: build GuestInput]
  Build --> Validate[Validation: stateless checks]
  Validate --> Prove[Prover: run zkVM]
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

  C->>API: POST /v1/proof/batch
  API->>Q: submit_batch_proof
  Q->>W: Preflight task
  W->>P: fetch blocks/witnesses/accounts
  W->>Q: store Preflight output
  Q->>W: Validation task
  W->>Q: store GuestInput
  Q->>W: Prove task
  W->>Z: prove(GuestInput)
  Z-->>W: Proof
  W->>Q: store Proof
  C->>API: GET /v1/proof/{id}
  API-->>C: status + proof
```

## Building

```bash
cd raiko2
cargo build --release
```

## Building Guests

Guest programs live in `guests/` as standalone crates (not part of the root workspace). Each guest
crate carries its own `[patch.crates-io]` in `Cargo.toml` to keep RISC0 and SP1 dependencies isolated.
ELF outputs are copied into `crates/guests/elf` for use by the host.

```bash
./script/build-guest.sh all
# or individually:
./script/build-guest.sh risc0
./script/build-guest.sh sp1
```

## Running

```bash
# Start the prover server
./target/release/raiko2 --config config.toml

# Or with environment variables
RAIKO_RPC_URL=http://localhost:8545 ./target/release/raiko2
```

## Documentation

- [API Documentation](docs/API.md)
- [Migration Guide](docs/MIGRATION.md)

## License

MIT License - see [LICENSE](../LICENSE)
