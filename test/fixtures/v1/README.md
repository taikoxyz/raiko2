# Raiko2 Fixture Envelopes

This directory stores `raiko2-fixture-v1` case envelopes.

The large provider-independent inputs stay under `test/guest_inputs/...`. A fixture envelope
references one of those inputs by repo-relative path, pins its raw SHA-256 digest, and stores the
expected public input commitment plus the opening needed to recompute it locally.

Phase 1 supports proposal-only Shasta fixtures:

```bash
cargo run -p xtask -- fixture generate \
  --network taiko_hoodi \
  --proposal 17460 \
  --proof-type native
```

```bash
cargo run -p xtask -- fixture check \
  --case test/fixtures/v1/shasta/taiko_hoodi/proposals/proposal_17460.fixture.json \
  --mode open-commitment
```

By default, `fixture generate` reads from `test/guest_inputs/shasta` and writes the canonical
`test/fixtures/v1/shasta/<network>/proposals/proposal_<id>.fixture.json` envelope. Use `--output`
for a different path and `--overwrite true` when intentionally replacing an existing envelope.

This mode does not call a provider. It verifies:

- the input file digest matches `input.sha256`
- `ProofCarryData` derived from `GuestInput` matches the stored opening
- `hash_shasta_subproof_input(ProofCarryData)` matches the stored commitment
- the Shasta guest input validates locally
