# Shared Shasta Fixture Design

## Goal

Replace the gitignored repo-root `test.json` dependency with a tracked, shared Shasta `GuestInput`
fixture so local compilation, tests, and fixture-backed API flows remain self-contained.

## Problem

- `bin/raiko2/src/server/fixture.rs` and
  `crates/primitives-shasta/tests/guest_input_bincode_roundtrip.rs` use `include_str!` on
  `../../../test.json` / `../../../../test.json`.
- `.gitignore` excludes `test.json`, so fresh checkouts cannot compile those targets.
- The file appears to have been a local preflight artifact rather than a repo-managed asset.

## Options Considered

1. Keep generating the file via `preflight`
   - Pro: matches how the original artifact was likely produced.
   - Con: depends on external RPC availability and stable chain data, so builds/tests stay fragile.

2. Generate the input at test runtime
   - Pro: no checked-in JSON.
   - Con: slower, less transparent for docs/manual benchmarking, and harder to reuse across crates.

3. Check in a shared fixture and document that `preflight` can regenerate it
   - Pro: deterministic, CI-friendly, reusable by tests and docs, and still compatible with the
     existing `preflight` workflow as an offline bootstrap tool.
   - Con: adds a maintained JSON sample to the repo.

## Decision

Use option 3.

- Add a tracked fixture under
  `tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json`.
- Make it minimal but semantically aligned with the existing fixture-backed e2e flow:
  `proposal_id = 3`, `l2_block_numbers = [3]`, `last_anchor_block_number = 0`.
- Update all hardcoded `include_str!(...test.json)` call sites and docs to reference the shared
  fixture path explicitly.
- Preserve the distinction that `preflight` may generate candidate inputs, but the repo does not
  depend on live RPC access to build or test.

## Verification

- `cargo fmt --all`
- `cargo test -p raiko2-primitives-shasta --test guest_input_bincode_roundtrip guest_input_json_to_bincode_roundtrip_test_json`
- `cargo check -p raiko2 --bin raiko2`
