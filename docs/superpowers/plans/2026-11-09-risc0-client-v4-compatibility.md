# RISC0 Client V4 Compatibility Implementation Plan

> **For agentic workers:** Use `superpowers:subagent-driven-development` or
> `superpowers:executing-plans`; keep implementation, review and verification distinct.

**Goal:** Establish that the restored RISC0 client in taiko-mono#22346 can use
raiko2's public API while the internal GPU-service integration remains deferred.

**Architecture:** Reuse existing v4 routes and wire types. Add client-shaped HTTP
conformance coverage and operator documentation; change production code only if
those tests reveal a real mismatch. Preserve all existing configuration defaults.

**Tech Stack:** Rust, Axum, serde, existing temporary runtime/engine test fixtures.

**Spec:** User request on 2026-11-09; taiko-mono#22346 head
`89349c57827888922d65e9b6ed1dbb0526e5dd2e`. API work starts from raiko2 main
`bf5485ea78429439f56bf507aa74607f69f9d774` after CUDA support merged in #300.

## Scope and decisions

- The client's proof composer is unchanged from its PR base. Public v4 already
  recognizes `risc0`; no new proof request field or GPU URL is needed.
- Both proposal and aggregation use `POST /v4/proof/proposal`; the `aggregate`
  flag selects the operation. Requests carry proposal/checkpoint/range metadata.
- Backlog operations are `GET /v4/prover/status?proof_type=risc0` and
  `POST /v4/prover/clear` with `{"proof_type":"risc0"}`.
- Preserve HTTP 200 and client-consumed response fields, including nonempty
  `0x` proof output, proof_type echo, and `registered`/`work_in_progress` polling.
- Match actual Go JSON: numeric IDs, explicit prover address, optional checkpoint,
  and checkpoint hashes containing 64 hex characters without a `0x` prefix.
- Keep the existing clear ACL. RISC0-only clear must preserve SP1 and other tasks.
- Built-in `Config::default()` leaves RISC0 disabled; the example config explicitly
  enables its local route. Preserve both and do not change deployed settings.
- Disabled RISC0 proof submission must fail before task admission. Empty backlog
  means `clean`; it is not a claim that the GPU backend is configured or ready.
- Do not add GPU transport, queue placeholder work, enable RISC0, restore
  Boundless, change the SDK, rebuild guests, publish images, or deploy services.

## Approaches

Existing wire compatibility makes new request fields or a second API path
unnecessary. Expanding this PR into GPU transport is excluded by the user.
Use the existing API plus regression coverage and documentation.

## Work

- [x] Run an existing RISC0 admission baseline on the isolated latest-main tree.
- [x] Add client-shaped HTTP tests for proposal and aggregate admission/polling,
  checkpoint handling, disabled-route rejection, and backlog status/clear isolation.
  Use deterministic engine fixtures; no live RPC or proof execution is required.
- [x] Update `docs/API.md` with client compatibility and the GPU readiness boundary;
  keep README/config examples consistent without changing their defaults.
- [x] Run focused server tests, relevant server regression tests, formatting,
  Clippy, and diff/path hygiene checks using the existing shared Cargo target.
- [x] Obtain independent adversarial review and independent behavioral verification.

Publish a separate API compatibility PR after readiness passes; report its exact
validation and the still-deferred GPU service integration in the PR and final handoff.

## Review focus

Check actual request parsing rather than enum-only tests; verify no task is
created on disabled admission; exercise RISC0 clear with live SP1 records;
check that HTTP statuses and JSON data are consumable by the Go client.

## Verification boundaries

Passing these tests establishes public HTTP compatibility. It does not establish
real GPU proving, remote GPU cancellation/recovery, or Taiko on-chain proof validity.

## Validation results

The final CPU-only server suite passed all 369 tests. The three new parameterized
HTTP tests cover the actual client request shapes, repeated polling, aggregate
output waiting, disabled admission, ACL and RISC0-only cancellation routing.
Independent verification reran those three and thirteen related regressions;
independent source review has no unresolved findings.

```bash
cargo test --locked --offline -p raiko2 --bin raiko2 --no-default-features
cargo clippy --locked --offline -p raiko2 --no-default-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

All commands above passed on Rust 1.95. The stricter Clippy invocation with
`--tests` reports 27 existing test-code diagnostics. An actual run with the
unchanged base source produced the same 27 diagnostics, matched by rule, message,
file and mapped source location; this change adds none. No suppressions or
unrelated lint fixes were introduced.

Proof outputs and queue engines in these tests are synthetic. The detach spy
checks RISC0 owner cancellation and the absence of SP1 cancellation calls; it
does not establish real GPU-worker teardown or remote cancellation behavior.
