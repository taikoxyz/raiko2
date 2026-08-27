---
name: proposal-regression
description: Use when validating a Shasta proposal locally from a Taiko L2 block height; discovers the proposal tuple, runs preflight, and verifies the generated GuestInput with the native guest launcher.
---

# Shasta Proposal Regression

## Inputs

Required:
- `network`: L2 chain spec key, usually `taiko_hoodi`.
- `l2_block`: any L2 block height inside the proposal to validate.

Optional:
- `l1_network`: defaults to `hoodi` for `taiko_hoodi`.
- `work_dir`: defaults to `/tmp/raiko2-shasta-regression-<l2_block>`.

## Workflow

1. Set variables:

```bash
NETWORK=taiko_hoodi
L1_NETWORK=hoodi
L2_BLOCK=7225500
WORK_DIR=/tmp/raiko2-shasta-regression-${L2_BLOCK}
mkdir -p "$WORK_DIR"
```

2. Discover the proposal tuple from the L2 block:

```bash
python scripts/regression/stress_proposal.py \
  --network "$NETWORK" \
  --l1-network "$L1_NETWORK" \
  --l2-block-range "${L2_BLOCK},$((L2_BLOCK + 1))" \
  --discover-only \
  --proposal-out "$WORK_DIR/proposal.discovery.json"
```

3. Read `proposal.discovery.json` and use the first item in `.proposals[]`.

```bash
eval "$(
python - "$WORK_DIR/proposal.discovery.json" <<'PY'
import json
import sys

proposal = json.load(open(sys.argv[1]))["proposals"][0]
print(f'PROPOSAL_ID={proposal["proposal_id"]}')
print(f'L1_INCLUSION_BLOCK_NUMBER={proposal["l1_inclusion_block_number"]}')
print(f'LAST_ANCHOR_BLOCK_NUMBER={proposal["last_anchor_block_number"]}')
print(f'L2_START={proposal["l2_start"]}')
print(f'L2_END={proposal["l2_end"]}')
PY
)"
```

4. Run preflight with the discovered tuple:

```bash
cargo run -r -p preflight -- \
  --network "$NETWORK" \
  --l1-network "$L1_NETWORK" \
  --proposal-id "$PROPOSAL_ID" \
  --l1-inclusion-block-number "$L1_INCLUSION_BLOCK_NUMBER" \
  --last-anchor-block-number "$LAST_ANCHOR_BLOCK_NUMBER" \
  --l2-start "$L2_START" \
  --l2-end "$L2_END" \
  --proof-type native \
  --validate \
  --output "$WORK_DIR/guest-input.json"
```

5. Run native verification/proof:

```bash
cargo run -r -p guest-launcher -- \
  --proof-type native \
  --mode prove \
  --input "$WORK_DIR/guest-input.json" \
  --output "$WORK_DIR/native-proof.json"
```

## Notes

- Do not start a `raiko2` server for this workflow.
- The intended guest-launcher backend is `native`; do not switch this workflow to SP1 or SGX.
- If Cargo starts compiling or downloading SP1 artifacts because of unconditional workspace
  dependencies, report that separately and still keep stress discovery plus preflight validation
  results; that is a build-environment issue, not a discovery/preflight failure.
- Chain specs provide default RPC URLs and the Shasta inbox contract. Override with stress
  `--l1-rpc`, `--l2-rpc`, or `--event-contract` only for non-standard environments.
- Fork-specific ABI files live under `scripts/regression/abi/`.
- Report the discovered proposal tuple and exact commands run in handoff notes.
