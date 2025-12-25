# Repository Guidelines

## Project Structure & Module Organization

Reusable crates live in `crates/` (notably `engine`, `pipeline`, `prover`, `provider`, `primitives`, `stateless`, `protocol`, `queue`, and `guests`). Standalone zkVM guest programs live in `guests/` (with `common/`, `risc0/`, `sp1/`) and are excluded from the workspace. Automation lives in `xtask/` (invoked via `just`), helper scripts are in `script/`, and auxiliary binaries are under `bin/` (`raiko2`, `rpc-proxy`). Docs and additional guides live in `docs/`. Keep new modules scoped to the appropriate crate to avoid cross-crate cycles.

## Build, Test, and Development Commands

- Guest builds: `just build-guest <risc0|sp1|all>` (uses docker + cargo risczero/prove via xtask).
- Always run `cargo clippy --workspace -- -D warnings` and `cargo nextest run --workspace`.

## Coding Style & Naming Conventions

The workspace targets Rust 2024 with four-space indentation. Module and crate names use `snake_case`; types and traits use `UpperCamelCase`. Run `cargo fmt --all` to format and `cargo clippy -D warnings` to catch regressions. Favor focused crates and avoid leaking backend-specific code into shared layers.

## Testing Guidelines

Place unit tests beside the implementation with `#[cfg(test)]`. Use deterministic data; prefer `rstest` or `proptest` when variation is needed. Name tests after the behavior they assert (`fn verifies_signature_with_valid_key`). Run backend-specific suites via `TARGET=<sp1|risc0|sgx> make test` and ensure integration coverage with `make integration` before merges.

## Commit & Pull Request Guidelines

Follow Conventional Commits (`feat:`, `fix:`, `chore:`). Each PR should link relevant issues (e.g., `#123`), describe changes, list build/test steps (`make fmt clippy test`), and note doc or metrics updates. Include screenshots or logs when touching developer tooling or dashboards.

## Security & Configuration Tips

Store secrets in `.env`; local overrides live outside version control. Use performance toggles such as `CPU_OPT=1`, `MOCK=1`, `RISC0_DEV_MODE=1`, and `SP1_PROVER=mock` when profiling or running prover hosts. Rebuild after changing SGX or prover configs to refresh generated artifacts.
