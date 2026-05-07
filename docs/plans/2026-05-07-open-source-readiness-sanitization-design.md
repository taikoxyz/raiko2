# Open Source Readiness Sanitization Design

## Goal

Complete the second-stage open-source-readiness cleanup by removing operator-specific
infrastructure details from public-facing defaults, examples, and checked-in fixtures without
changing the supported network names or breaking existing code/test routing.

## Scope

This pass sanitizes:

- `config.example.toml`
- `config/chain_spec_list_default.json`
- `docs/operations.md`
- public CLI default profiles in `xtask`
- checked-in guest input fixtures under `test/guest_inputs/shasta`

This pass does **not** rename supported networks such as `taiko_dev`, `taiko_hoodi`, or
`taiko_masaya`. Those names are used widely across code, tests, and fixtures and are part of the
current repo contract.

## Sanitization Policy

### Public examples

- Replace raw IP-based RPC endpoints with stable placeholder hostnames under `example.com` or a
  similarly obvious non-production suffix.
- Replace internal or operator-specific image registry examples with placeholder registries.
- Keep commands structurally valid, but avoid implying one operator environment is the canonical
  public deployment.

### Built-in defaults

- For shipped CLI default profiles, prefer stable public RPC endpoints over raw infrastructure IPs
  when a public endpoint already exists.
- If no public endpoint should be implied, prefer an explicit placeholder over a private-looking
  default.

### Fixtures

- Keep fixture network names and proposal IDs unchanged.
- Sanitize embedded chain-spec RPC fields and repeated raw endpoint strings so checked-in artifacts
  no longer expose operator infrastructure.
- Do not rewrite witness payload semantics or non-endpoint data.

## Expected Outcome

After this pass, the repository should still behave the same for developers, but the public tree
should no longer advertise internal RPC hosts, raw operator IPs, or operator-specific image
registry examples in its default docs and fixtures.
