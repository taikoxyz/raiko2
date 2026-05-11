# Split Fixture Server From Runtime Image

## Problem

The main `raiko2` Docker image currently compiles `bin/raiko2/src/server/fixture.rs` as part of the
normal server binary. That module embeds checked-in fixture JSON via `include_str!`, which means the
runtime image build depends on `tests/fixtures/` being present in the Docker build context.

This is undesirable because:

- fixture-only assets leak into the production image build path
- runtime image builds become sensitive to test fixture layout
- `sgx/remote`-only service builds still pay for fixture-server compilation

## Desired Outcome

- fixture-backed local server code should not be part of the normal runtime image build path
- production `raiko2` image builds should not require `tests/fixtures/`
- fixture server logic should move behind a dedicated feature, binary, or test-only harness

## Follow-Up Direction

Evaluate one of:

1. move the fixture server into a dedicated binary
2. gate fixture-server code behind a non-default feature
3. move fixture-only assets/loading behind test-only compilation paths

The preferred result is that `docker build` for the normal `raiko2` service does not compile or embed
fixture JSON at all.
