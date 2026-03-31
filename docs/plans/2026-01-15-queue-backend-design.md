# Queue Backend Selection Design (raiko2)

> Historical design document. It may not match the current implementation. Use `README.md`,
> `docs/API.md`, and `config.example.toml` as the current source of truth.

## Goal
Provide a simple, explicit way to select a single queue backend (memory or Redis) for `bin/raiko2` at startup without introducing backup/replication semantics.

## Non-Goals
- Automatic failover or hot backup between memory and Redis.
- Cross-backend state replication or reconciliation.
- Runtime switching of backends after the process starts.

## Decision
Use a single backend selected via CLI/env configuration (`--queue-backend memory|redis`). The backend is fixed for the lifetime of the process. Memory is intended for local/single-process use; Redis is intended for multi-process or persistent queueing. No dual-write or fallback logic is introduced.

## Architecture
Configuration is loaded in `bin/raiko2` by `Config::load`, which merges the config file and CLI overrides. `QueueConfig::validate` enforces required fields (workers, maintenance interval, Redis URL/namespace when Redis is selected). `AppState::new` constructs the queue engine for each pipeline using the same configured backend. The store instance is injected into the engine via `Engine::with_store_and_scheduler_config`.

## Components and Data Flow
1. CLI parses `--queue-backend` and related queue options.
2. `Config::load` sets `QueueConfig.backend` and validates it.
3. `AppState::new` constructs scheduler config and starts worker loops.
4. For each pipeline:
   - `QueueBackend::Memory` => `MemoryStore::new()`.
   - `QueueBackend::Redis` => `RedisStore::connect(url, namespace, lease)` (requires `redis-queue` feature).
5. Queue operations flow through `Scheduler::{submit,next_ready,complete}` and `TaskStore` implementations.

## Error Handling and Runtime Behavior
- Invalid queue configuration fails fast during startup with clear errors.
- Redis backend without `redis-queue` feature fails fast with an explicit message.
- Redis connectivity or (de)serialization errors are surfaced as `TaskStoreError::Backend`/`CorruptData` and reported at enqueue or task processing time.
- Memory backend is ephemeral; task loss on restart is expected and documented.

## Testing and Validation
- Unit tests in `crates/queue` continue to validate scheduler behavior and memory store semantics.
- Redis integration tests in `crates/queue/tests/redis_store.rs` validate persistence and lease behavior.
- Add/keep a config merge test in `bin/raiko2/src/config/mod.rs` to ensure `--queue-backend` overrides the config file and validates required Redis fields.
- Provide operational examples for both backends and remind operators to compile with `--features redis-queue` for Redis usage.

## Operational Notes
- Memory backend is for local or single-process deployments.
- Redis backend is required for shared queueing across processes or restarts.
- Feature gating: `--features redis-queue` is required to build Redis support.
