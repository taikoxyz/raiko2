# Guest Digests Export Design

## Goal

Expose the current Shasta guest registration digests directly in GitHub Actions so auditors and
operators can verify which on-chain `image id` / `program` digests correspond to a given commit,
without needing to run local tooling or query verifier contracts.

## Scope

This design only covers local digest export.

It does **not** add:

- automatic verifier registration
- verifier readback checks
- chain-specific profile resolution
- deployment or rollout behavior

## Requirements

- Reuse the existing `guest-elf-consistency` self-hosted CI job so digest export does not trigger a
  second guest rebuild.
- Keep the digest computation fully local:
  - no RPC
  - no verifier profile
  - no trusted/untrusted chain state
- Export the exact digest values that matter for Shasta verifier registration:
  - `risc0` proposal image id
  - `risc0` aggregation image id
  - `sp1` proposal `vk_bn254`
  - `sp1` proposal `vk_hash_bytes`
  - `sp1` aggregation `vk_bn254`
  - `sp1` aggregation `vk_hash_bytes`
- Make the results visible in GitHub in two places:
  - job summary
  - downloadable JSON artifact

## Approach

Add a new lightweight `xtask-build-guest` `guest-digests` command that computes digests from the
checked-in guest ELF directory and writes a JSON summary file.

The subcommand should stay separate from `register-image` because:

- the user need is audit visibility, not registration orchestration
- `register-image` currently mixes digest computation with verifier contracts, RPC profiles, and
  optional transaction broadcasting
- keeping digest export offline avoids ambiguity about whether the workflow is checking chain state

`guest-elf-consistency` will then:

1. rebuild the guest ELFs
2. run `cargo run -r -p xtask-build-guest --bin guest-digests -- --output <path>`
3. publish the JSON as an artifact
4. render a short Markdown summary into `GITHUB_STEP_SUMMARY`
5. fail if the rebuilt ELF tree is dirty

This keeps the digest output tied to the exact ELF bytes used by the consistency gate.

## Data Shape

The JSON summary should include:

- generation timestamp
- guest ELF directory
- a flat list of digest entries

Each digest entry should include:

- proof system (`risc0` or `sp1`)
- object name
- stage (`proposal` or `aggregation`)
- digest source (`image_id`, `vk_bn254`, `vk_hash_bytes`)
- digest value

## Testing Strategy

- Add `xtask-build-guest` unit tests that validate the exported digest set shape:
  - all expected object names are present
  - `risc0` exports exactly two `image_id` entries
  - `sp1` exports exactly four verification-key-derived entries
- Verify the workflow YAML still parses after CI changes.

## Non-Goals

- replacing `register-image`
- publishing digest summaries to a registry or release page
- encoding verifier addresses or chain IDs into the digest artifact
