# Split Fixture Server From Runtime Image Design

## Problem

The main `raiko2` binary currently always compiles the fixture-backed local HTTP harness:

- `bin/raiko2/src/cli.rs` always exposes `fixture-server`
- `bin/raiko2/src/main.rs` always wires that command into the runtime
- `bin/raiko2/src/server/fixture.rs` embeds checked-in JSON with `include_str!(...tests/fixtures...)`

Because the default `raiko2` binary references that module, the normal runtime Docker build also
copies `tests/fixtures/` into the build context. That is the wrong dependency direction for a
production runtime image.

## Goal

Keep the fixture-backed local server available for manual development, but remove it from the
default `raiko2` runtime build path so production images no longer depend on `tests/fixtures/`.

## Chosen Approach

Use a non-default crate feature named `fixture-server`.

Under this design:

- the `fixture-server` CLI subcommand only exists when `--features fixture-server` is enabled
- `bin/raiko2/src/server/fixture.rs` is only compiled with that feature
- the default `raiko2` build no longer references `tests/fixtures/`
- the top-level `Dockerfile` can stop copying `tests/fixtures/` into the normal runtime image build

## Why This Approach

This is the smallest change that fixes the actual image-build problem.

It avoids:

- a larger refactor to move shared server internals into another package
- a second binary/package surface just to preserve a local-only harness
- leaving the default runtime image sensitive to fixture layout

The fixture-backed workflow remains available for developers with an explicit command:

```bash
cargo run -p raiko2 --features fixture-server -- fixture-server --host 127.0.0.1 --port 8087
```

## Scope

### In Scope

- add the non-default `fixture-server` feature in `bin/raiko2/Cargo.toml`
- gate the fixture server CLI/type/module wiring behind that feature
- remove `tests/fixtures` from the default Docker build context
- update README and development docs to show the feature-enabled command

### Out Of Scope

- changing the fixture JSON assets themselves
- moving remote prover fixtures out of `tests/fixtures/remote_prover`
- introducing a dedicated fixture-server package or image
- changing runtime API behavior outside the fixture-backed local harness

## User-Facing Result

Normal runtime usage stays the same:

```bash
cargo run -p raiko2 -- --config config.toml
```

Fixture-backed manual testing becomes explicit:

```bash
cargo run -p raiko2 --features fixture-server -- fixture-server --host 127.0.0.1 --port 8087
```

## Verification Strategy

- default build/test path confirms `fixture-server` is not available without the feature
- feature-enabled build/test path confirms `fixture-server` still parses and wires correctly
- Dockerfile no longer copies `tests/fixtures/` for the normal runtime image
- formatting and targeted package tests remain green
