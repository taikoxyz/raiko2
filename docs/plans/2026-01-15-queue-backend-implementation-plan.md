# Queue Backend Manual Selection Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add explicit test coverage for CLI queue-backend selection and document the CLI examples for memory vs. Redis backends in `bin/raiko2`.

**Architecture:** The runtime behavior stays unchanged; we only add tests in the `raiko2` binary crate config module and extend existing CLI documentation. No new runtime code paths or storage abstractions are introduced.

**Tech Stack:** Rust (clap, anyhow, toml), unit tests, Markdown docs.

### Task 1: Add CLI override + Redis validation tests

**Files:**
- Modify: `bin/raiko2/src/config/mod.rs`

**Step 1: Add helper + tests**

Add the following to the `#[cfg(test)] mod tests` block in `bin/raiko2/src/config/mod.rs`:

```rust
use crate::cli::Cli;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn write_temp_config(contents: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    path.push(format!("raiko2-config-{nanos}.toml"));
    std::fs::write(&path, contents).expect("write temp config");
    path
}

#[test]
fn test_queue_backend_cli_overrides_config_file() {
    let config_toml = r#"
[server]
host = "0.0.0.0"
port = 8080

[rpc]
l1_rpc = "http://localhost:8545"
l2_rpc = "http://localhost:9545"
l1_chain_id = 1
l2_chain_id = 167000

[prover]
prover_type = "risc0"

[queue]
backend = "memory"
namespace = "raiko2:queue"
workers = 1
maintenance_interval_ms = 200
"#;
    let path = write_temp_config(config_toml);

    let cli = Cli::parse_from([
        "raiko2",
        "--config",
        path.to_str().expect("path utf8"),
        "--queue-backend",
        "redis",
        "--redis-url",
        "redis://localhost:6379/",
    ]);

    let config = Config::load(&cli).expect("config load");
    assert_eq!(config.queue.backend, QueueBackend::Redis);
    assert_eq!(config.queue.redis_url.as_deref(), Some("redis://localhost:6379/"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn test_queue_backend_redis_requires_url() {
    let config_toml = r#"
[server]
host = "0.0.0.0"
port = 8080

[rpc]
l1_rpc = "http://localhost:8545"
l2_rpc = "http://localhost:9545"
l1_chain_id = 1
l2_chain_id = 167000

[prover]
prover_type = "risc0"

[queue]
backend = "memory"
namespace = "raiko2:queue"
workers = 1
maintenance_interval_ms = 200
"#;
    let path = write_temp_config(config_toml);

    let cli = Cli::parse_from([
        "raiko2",
        "--config",
        path.to_str().expect("path utf8"),
        "--queue-backend",
        "redis",
    ]);

    let err = Config::load(&cli).expect_err("expected config error");
    assert!(err.to_string().contains("redis_url"));

    let _ = std::fs::remove_file(path);
}
```

**Step 2: Run the new tests**

Run:
```bash
cargo test -p raiko2 test_queue_backend_cli_overrides_config_file test_queue_backend_redis_requires_url
```
Expected: PASS

**Step 3: Run the raiko2 crate tests**

Run:
```bash
cargo test -p raiko2
```
Expected: PASS

**Step 4: Commit**

```bash
git add bin/raiko2/src/config/mod.rs
git commit -m "test: cover queue backend cli overrides"
```

### Task 2: Document CLI examples for queue backends

**Files:**
- Modify: `docs/API.md`

**Step 1: Add CLI usage examples**

Insert into the `## CLI Usage` section in `docs/API.md`:

```bash
# Select memory queue backend (default behavior)
raiko2 --queue-backend memory

# Select Redis queue backend (requires build with --features redis-queue)
raiko2 --queue-backend redis --redis-url redis://localhost:6379/ --queue-namespace raiko2:queue
```

**Step 2: Commit**

```bash
git add docs/API.md
git commit -m "docs: add queue backend cli examples"
```

### Task 3: Final verification

**Files:**
- No additional files

**Step 1: Run full test suite if required by policy**

Run:
```bash
cargo test -p raiko2
```
Expected: PASS

**Step 2: Summarize changes**

Note the new tests and the CLI docs snippet for memory/redis backends.
