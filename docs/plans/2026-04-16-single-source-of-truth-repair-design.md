# Single Source of Truth Repair Design (raiko2)

> Historical design document. It may not match the current implementation. Use `README.md`,
> `docs/API.md`, and `config.example.toml` as the current source of truth.

## Goal

Repair three single-source-of-truth failures in `raiko2`:

1. Separate persisted runtime lifecycle from API-facing proof status.
2. Move proving route identity to a single canonical shared type.
3. Move external aggregate proof validation out of HTTP handlers into shared domain logic.

## Non-Goals

- Introducing new proving routes or new proof backends.
- Redesigning the public `/v3` API envelope.
- Migrating `runtime.sqlite` schema in this change.
- Refactoring unrelated pipeline, provider, or queue internals.

## Decision

The canonical owners become:

- Root runtime lifecycle: `raiko2_runtime::RuntimeTaskRecord.runner_status`
- Proving route identity: a shared canonical route type in `crates/pipeline`
- Aggregate proof admission rules: shared validation logic in `crates/prover`

The HTTP layer remains responsible for request parsing and policy decisions, but it no longer owns
business rules that are shared with runtime, pipeline, or prover code.

## Architecture

### 1. Runtime status semantics

`data.status` remains the proof-oriented API status (`pending`, `proving`, `completed`, `failed`,
`cancelled`). `data.runtime.runner_status` becomes a direct projection of the persisted runtime
record and is no longer derived from `data.status`.

This restores the intended dual-view model:

- runtime lifecycle answers "what does the persisted runner think happened?"
- proof status answers "what proof progress should the client see?"

### 2. Canonical proving route

Introduce a shared canonical route type in `crates/pipeline` that owns:

- canonical route string
- `PipelineKey`
- `ProofType`
- guest system
- runner kind

The type provides:

- `Display` and `FromStr`
- conversion from `PipelineKey`
- conversion to `PipelineKey`
- accessors for `proof_type`, guest system, and runner

`bin/raiko2` keeps API policy only:

- `native` selects the native local route
- `sp1` selects the SP1 local route
- `risc0` selects the configured default RISC0 route
- `zk_any` performs admission-time selection first, then resolves to the shared route

Once a route is selected, runtime registration, task metadata, and handler responses all use the
same shared canonical value.

### 3. Aggregate proof validation

External aggregate proof validation moves to shared logic in `crates/prover`.

The shared validator owns the route-specific admission contract for externally supplied proposal
proofs. The validator either:

- returns success for a valid proof batch, or
- returns a precise invalid-request error describing the first missing or incompatible field

The HTTP handler becomes a thin adapter:

1. parse request
2. resolve canonical route
3. call shared aggregate validator
4. enqueue aggregation task

Deeper prover-side validation still remains in place for semantic checks such as carry-data
consistency or image-id consistency. The shared validator is the canonical owner of "which fields
must be present"; downstream prover code remains the canonical owner of "whether those fields form
a valid aggregation input".

## Components and Data Flow

### Runtime view path

1. Runtime registration persists `runner_status` in `runtime.sqlite`.
2. Task loading reads `RuntimeTaskRecord`.
3. API response builds:
   - `data.runtime.runner_status` from the persisted runtime record
   - `data.status` from proposal and aggregate proof progress summarization
4. The two fields may intentionally differ.

### Route resolution path

1. API request chooses a high-level `proof_type`.
2. Server policy resolves that choice to a canonical shared route.
3. Canonical route determines `PipelineKey`, `ProofType`, guest system, and runner.
4. Runtime registration and task metadata store values derived from the canonical route.
5. Runtime loading reconstructs or validates canonical route identity from stored values.

### Aggregate admission path

1. API handler receives aggregate request.
2. Shared route is resolved.
3. Shared prover validation checks external proof field requirements for that route.
4. Prover aggregation build path performs deeper semantic validation.

## Storage and Compatibility

No schema migration is required.

The existing runtime columns remain:

- `pipeline_key`
- `route`
- `guest_system`
- `runner`

In this design they are treated as denormalized projections of the canonical route, not independent
truth sources. Runtime reads must fail fast when the stored values disagree.

## Error Handling

- Invalid route strings fail during parsing, not later in handler logic.
- Inconsistent persisted route identity fails fast when runtime rows are loaded.
- Invalid external aggregate proofs fail with a shared validation error mapped to `bad_request`.
- `data.runtime.runner_status` no longer silently rewrites persisted runtime state into a derived
  proof status.

## Testing and Validation

### Unit tests

- Canonical route round-trip tests:
  - route string -> canonical route
  - `PipelineKey` -> canonical route
  - canonical route -> route string and `PipelineKey`
- Runtime projection tests:
  - persisted `runner_status` is returned unchanged in API runtime view
  - API proof status still reflects proof progress independently
- Aggregate validator matrix tests:
  - native local requirements
  - SP1 local requirements
  - RISC0 local requirements
  - RISC0 Boundless requirements
- Runtime consistency tests:
  - mismatched `pipeline_key`/`route`
  - mismatched `route`/guest system
  - mismatched `route`/runner

### Verification commands

- `cargo fmt --all`
- `cargo clippy --workspace -- -D warnings`
- `cargo nextest run --workspace`

## Implementation Notes

- Prefer moving existing types instead of introducing parallel aliases.
- Avoid widening API request types; only move ownership of validation.
- Keep the route and aggregate validation APIs small and deterministic so they are reusable from
  tests, CLI tools, and future non-HTTP entry points.
