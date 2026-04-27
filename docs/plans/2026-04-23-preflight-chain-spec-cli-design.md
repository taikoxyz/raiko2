# Preflight Chain Spec CLI Design

## Goal

Simplify the standalone `preflight` command for proposal regression runs by deriving chain IDs
and RPC URLs from the existing chain spec list.

## Scope

- Keep proposal tuple inputs explicit:
  - `--proposal-id`
  - `--l1-inclusion-block-number`
  - `--last-anchor-block-number`
  - `--l2-start`
  - `--l2-end`
- Add network selectors for chain spec lookup:
  - `--network` selects the L2 chain spec.
  - `--l1-network` selects the L1 chain spec.
- Keep explicit `--rpc-url`, `--l1-rpc-url`, `--l2-chain-id`, and `--l1-chain-id` as overrides.
- Keep `--blob-proof-type` as an advanced override, but do not require it for normal Shasta
  runs because Shasta defaults to `proof_of_equivalence`.
- Preserve `--chain-spec-file` so local chain spec overrides can change default RPC URLs or chain
  IDs without changing the CLI shape.

## Non-Goals

- Do not add proposal discovery mode.
- Do not infer `l1_inclusion_block_number`, `last_anchor_block_number`, proposal ID, or L2 block
  range from RPC.
- Do not change `raiko2` server API behavior.
- Do not change the Shasta pipeline or manifest validation logic.

## UX

The intended common path is:

```bash
cargo run -r -p preflight -- \
  --l1-network hoodi \
  --network taiko_hoodi \
  --proposal-id 17771 \
  --l1-inclusion-block-number 2674375 \
  --last-anchor-block-number 2674326 \
  --l2-start 7225402 \
  --l2-end 7225593 \
  --proof-type native \
  --validate \
  --output /tmp/proposal-17771.json
```

Operators can still override chain spec defaults:

```bash
--rpc-url "$L2_RPC" --l1-rpc-url "$L1_RPC"
--l2-chain-id 167013 --l1-chain-id 560048
```

## Error Handling

- If neither `--network` nor `--l2-chain-id` is provided, fail with an actionable message.
- If neither `--l1-network` nor `--l1-chain-id` is provided, default to `l1_chain_id=1` only for
  backward compatibility.
- If a selected chain spec has an empty RPC URL and no explicit RPC override is provided, fail with
  an actionable message.
- If explicit IDs conflict with selected chain specs, fail fast instead of silently mixing chains.

## Testing

- Unit-test argument resolution independently from network preflight.
- Cover network-only resolution, explicit override resolution, conflict rejection, missing RPC
  rejection, and backward-compatible explicit-ID mode.
- Keep existing pipeline tests unchanged.
