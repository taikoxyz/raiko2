# Open Source Readiness Sanitization Implementation Plan

> Implement this plan task-by-task, validating each step before moving to the next.

## Goal

Remove internal or operator-specific infrastructure references from the public repo surface while
preserving existing network identifiers and test behavior.

## Steps

1. Update public config examples.
   - Replace raw RPC IPs in `config.example.toml` with placeholder hostnames.
   - Replace Boundless example endpoints with placeholder hostnames.

2. Update shipped chain-spec defaults.
   - Replace internal or raw-IP RPC values in `config/chain_spec_list_default.json` with sanitized
     public or placeholder endpoints.

3. Update public operational docs.
   - Replace operator-specific release tags and registries in `docs/operations.md`.
   - Reword register-image guidance so it is still accurate without implying a single internal
     environment is the public default.

4. Update code-side CLI defaults that still expose raw infrastructure.
   - Replace raw-IP defaults in `xtask/src/register_image.rs`.
   - Replace raw-IP fallbacks in `xtask/src/latest_proposal_request.rs`.

5. Sanitize checked-in fixtures.
   - Replace embedded raw endpoint strings in `test/guest_inputs/shasta/**/proposals/*.json`.
   - Keep proposal contents otherwise unchanged.

6. Validate.
   - Verify JSON and TOML edits are structurally intact.
   - Run `git diff --check`.
   - Spot-check the sanitized endpoints with `rg`.
