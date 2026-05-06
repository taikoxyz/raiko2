# Contributing

## Development Setup

Start with:

- [README.md](README.md) for project overview and local run commands
- [docs/development.md](docs/development.md) for local workflows, fixture testing, and guest builds
- [docs/API.md](docs/API.md) for request and response semantics

The canonical configuration shape lives in [config.example.toml](config.example.toml).

## Recommended Checks

Run the smallest relevant set of checks for your change. Common baselines are:

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test -p raiko2-primitives -p raiko2-primitives-shasta -p raiko2-protocol -p raiko2-protocol-shasta
cargo test -p raiko2-provider -p raiko2-pipeline -p preflight
cargo test -p raiko2-queue -p raiko2-runtime
```

When you touch guest, prover backend, or ELF-related code, also run the relevant guest build:

```bash
just build-guest risc0
just build-guest sp1
```

## Pull Requests

- Keep changes on the primary codepath; avoid duplicate implementations.
- Follow Conventional Commits.
- In PR descriptions, list the exact verification commands you ran.
- Update docs when request/response, config, or operational behavior changes.
