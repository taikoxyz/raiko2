# Repository Guidelines

## Purpose

Use this file as the agent-facing execution guide for this repository. Keep it focused on task routing,
verification, and safety rails. Treat `README.md` as the source of truth for architecture and operator
workflows, and treat `docs/API.md` as the source of truth for HTTP/API behavior.

## Source Of Truth

- Use `README.md` for project layout, build/run examples, guest build details, and prover workflow.
- Use `docs/API.md` for request/response contracts, config keys, and environment variables.
- Use `config.example.toml` as the canonical config shape.
- Use `.codex/skills/raiko2-image-release/SKILL.md` for image build-and-publish sequencing.
- Do not copy long command walkthroughs into this file. Add only agent-critical rules and stable entrypoints.

## Repository Layout

- `crates/`: reusable workspace crates. Keep shared types and business rules here.
- `bin/raiko2`: main server binary, CLI, config loading, and HTTP handlers.
- `bin/preflight`: standalone preflight CLI for building `GuestInput`.
- `bin/guest-launcher`: local guest runner and benchmark/proof helper.
- `bin/rpc-proxy`: RPC proxy and witness/debug support.
- `bin/witness-check`: witness inspection and validation helpers.
- `xtask/`: automation entrypoints, including guest build orchestration.
- `guests/`: standalone guest program sources for `risc0` and `sp1`; not part of the workspace.
- `crates/guests/elf`: built guest ELF assets consumed by the host. Never hand-edit generated ELF files.

## Change Routing

- Put shared domain types, enums, proof payloads, and validation invariants in `crates/primitives*` or
  `crates/protocol*`, not in binaries.
- Put fork-specific preflight, validation, manifest selection, and pipeline wiring in `crates/pipeline`.
- Put prover-backend implementations and proof encoding/aggregation logic in `crates/prover`.
- Put provider/RPC/witness fetching logic in `crates/provider`.
- Put queueing, scheduling, and orchestration logic in `crates/engine` or `crates/queue`.
- Limit `bin/*` changes to CLI, config, wiring, and surface-specific behavior.
- Do not reintroduce legacy paths or concepts from old docs such as `host/`, `lib/`, `core/`, `taskdb/`,
  or `reqpool/` unless the code in this repo actually adds them.

## Stable Command Entry Points

- Main server: `cargo run -r -p raiko2 -- --config config.toml`
- Config path override: `RAIKO2_CONFIG=/path/to/config.toml`
- Workspace checks:
  - `cargo fmt --all`
  - `cargo clippy --workspace -- -D warnings`
  - `cargo test -p raiko2-primitives -p raiko2-primitives-shasta -p raiko2-protocol -p raiko2-protocol-shasta`
  - `cargo test -p raiko2-provider -p raiko2-pipeline -p preflight`
  - `cargo test -p raiko2-queue -p raiko2-runtime`
- Guest builds:
  - `just build-guest risc0`
  - `just build-guest sp1`
  - `just build-guest all`
- Direct xtask fallback: `cargo run -r -p xtask -- build-guest <backend>`
- Image release:
  - `just release-image <backend> <tag>`
  - `cargo run -r -p xtask -- release-image <backend> --tag <tag> --repository registry.example.com/raiko2`
- Do not invent `make` targets or use outdated `TARGET=... make test` workflows in this repo.

## Project Skill Rule

For image release or image publication tasks, read `.codex/skills/raiko2-image-release/SKILL.md`
before acting.

Do not use this repository to perform Tolba or GKE rollout. Keep `release-image` scoped to guest
ELF refresh, image build/push, digest capture, and optional `register-image` checks only.

## Verification Policy

Run the smallest set of checks that proves the change safely, then scale up when the impact widens.

- Docs-only changes:
  - No Rust checks required unless commands or paths were changed; verify those facts against the repo.
- Single-crate internal Rust changes:
  - Run focused tests for the touched package when practical, plus any relevant targeted command.
- Shared types, config, workspace wiring, or cross-crate behavior changes:
  - Run `cargo clippy --workspace -- -D warnings`
  - Run the targeted `cargo test` lanes from `.github/workflows/ci.yml`
- Formatting-sensitive Rust changes:
  - Run `cargo fmt --all`
- Guest, prover backend, `xtask`, or ELF contract changes:
  - Run the relevant `just build-guest <backend>` command in addition to Rust checks.
- If a change touches request/response or config semantics, verify `README.md`, `docs/API.md`, and
  `config.example.toml` still match.

## Workflow Rules

- Follow Conventional Commits.
- In PRs and handoff notes, report the exact commands you ran and whether they passed.
- Prefer `gh` for GitHub operations.
- Keep changes on the single primary codepath; do not leave duplicate implementations behind.
- Fail fast on invalid inputs and keep one source of truth for business rules.

## Alethia Reth Integration

- Use the `feat/raiko2` branch from `https://github.com/taikoxyz/alethia-reth` as the canonical
  base for all raiko2-specific alethia-reth patches, regardless of local checkout path.
- Put every alethia-reth fix required by raiko2 on `feat/raiko2`; do not keep those fixes only in
  one-off PR branches, local worktrees, or raiko2-side workaround layers.
- Rebase `feat/raiko2` onto alethia-reth `origin/main` when upstream alethia-reth or reth changes are
  adopted, then update raiko2 lockfiles to the resulting branch commit.
- Raiko2 Cargo manifests should reference alethia-reth with `branch = "feat/raiko2"`; lockfiles are the
  exact commit pin. Do not pin arbitrary alethia-reth `main` revisions for integration-only fixes.
- Treat reth test utilities as non-production support code. Do not solve guest or no-std issues by
  routing through `test-utils` features or dev-only APIs.
- Keep RISC0 guest `getrandom` handling in `xtask`; do not add a second Cargo config source of truth
  for `getrandom_backend`.

## Safety Rails

- Keep guest sources under `guests/` and host/workspace code under `crates/` and `bin/`.
- Do not hand-edit generated artifacts under `crates/guests/elf`.
- Do not assume SGX support, old build scripts, or deprecated binaries exist unless you verified them in
  the current tree.
- When changing prover backends or proof formats, check both proposal and aggregation paths.
- When changing config loading, preserve the documented precedence: config file first, then environment
  variables and CLI flags override it.
