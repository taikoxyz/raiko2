# Raiko2 Docs

This directory contains the contributor- and operator-facing documentation for `raiko2`.

## Start Here

- [Project overview](../README.md)
- [API contract](API.md)
- [Precompile status](precompile-status.md)
- [Development guide](development.md)
- [Operations guide](operations.md)
- [Hoodi tx-list witness rollout](hoodi-txlist-witness-rollout.md)
- [Regression harness](../scripts/regression/README.md)
- [Gaiko2 remote prover integration](gaiko2-remote-prover-integration.md)
- [Configuration example](../config.example.toml)

## How to Use These Docs

- Start with [../README.md](../README.md) if you are new to the repository.
- Read [API.md](API.md) for request, response, and task lifecycle semantics.
- Read [precompile-status.md](precompile-status.md) when you need the current Shasta precompile
  activation and guest hook coverage.
- Read [development.md](development.md) for local workflows, fixture testing, guest builds, and benchmarking.
- Read [operations.md](operations.md) for runtime configuration, Docker, and image publishing.
- Read [gaiko2-remote-prover-integration.md](gaiko2-remote-prover-integration.md) when updating
  `gaiko2` to match the canonical remote prover protocol and conformance harness.

## Historical Notes

The files under [`plans/`](plans) and [`issues/`](issues) are historical design, implementation,
and review notes. They are useful background, but they are not the current source of truth for
using or operating the project.
