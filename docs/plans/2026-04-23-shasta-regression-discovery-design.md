# Shasta Regression Discovery Design

## Goal

Provide a main-targeted standalone regression flow that starts from a network and one L2 block
height, discovers the full Shasta proposal tuple, builds a `GuestInput` with `preflight`, and
replays it with the native guest launcher.

## Scope

- Copy the mature stress discovery helper into the main-targeted CLI branch.
- Keep discovery in Python for now; do not move anchor/proposal discovery into `preflight`.
- Resolve default L1/L2 RPC URLs and Shasta inbox contract address from
  `config/chain_spec_list_default.json`.
- Move ABI fixtures under `scripts/regression/shasta/` so future fork-specific ABIs can sit next
  to them.
- Add a discover-only output mode for agents and scripts.
- Add a repo-local skill that documents the repeatable regression workflow.

## CLI Shape

Common discovery input should be:

```bash
python scripts/regression/stress_shasta_proposal.py \
  --network taiko_hoodi \
  --l1-network hoodi \
  --l2-block-range 7225500,7225501 \
  --discover-only \
  --proposal-out /tmp/shasta-proposal.json
```

Explicit `--l1-rpc`, `--l2-rpc`, `--event-contract`, `--abi-file`, and `--anchor-abi-file` remain
overrides for non-standard environments.

## Data Flow

1. Stress discovery reads the selected chain specs.
2. It derives L1 RPC from `--l1-network`, L2 RPC and Shasta L1 contract from `--network`.
3. It uses existing stress logic to expand the provided L2 block range to full proposal ranges.
4. It emits proposal tuple JSON for each discovered proposal.
5. The skill feeds one proposal tuple into `preflight --validate --output`.
6. The skill runs `guest-launcher --proof-type native --mode prove`.

## Non-Goals

- Do not add `preflight` discovery in this task.
- Do not remove existing stress submission mode.
- Do not require SGX or a running `raiko2` server for native regression.

## Testing

- Unit-test chain spec resolution and override behavior in the stress script.
- Unit-test discover-only JSON shape without live RPC by exercising pure formatting helpers.
- Keep existing stress L1 search window tests.
- Run `python -m unittest scripts/regression/tests/test_stress_shasta_proposal.py`.
- Run `cargo test -p preflight` and `cargo run -p preflight -- --help`.
- Treat `cargo run -p guest-launcher -- --help` as an optional native-path check when SP1 build
  artifacts are available, because the current binary can compile SP1 dependencies even for
  native-only commands.
