---
name: raiko2-service-regression
description: Use when validating an already running or isolated raiko2 v4 service with real Shasta proposal and aggregate requests, especially across remote SGX and SGX-geth lanes.
---

# Raiko2 Service Regression

## Scope

Exercise the real v4 host-to-remote-provider path without replacing, restarting, pruning, or
reconfiguring a live service. Use `shasta-proposal-regression` instead for local native GuestInput
replay.

Do not build images, register enclaves, change verifier state, or verify proofs on-chain unless the
user explicitly adds that work. An isolated host must use separate ports, runtime namespace, and
storage from any live host.

## Inputs

Obtain these from the user or current deployment config; never guess endpoints:

- `NETWORK`, `L1_NETWORK`, `L1_RPC`, `L2_RPC`, and `RAIKO_RPC`.
- A comma-separated, contiguous `PROPOSAL_IDS` group containing at least two proposals.
- `PROOF_TYPES`, normally `sgx sgxgeth`.
- `PROVER`, shared by base and aggregate requests because it is part of the task identity.
- Optional `RAIKO2_API_KEY` in the environment. Never put its value in a command or log.

`NETWORK` and `L1_NETWORK` drive discovery only. The v4 host selects its configured network pair;
confirm the target host's resolved pair matches them before submitting work.

Use the active virtual environment when present, otherwise `python3`:

```bash
set -euo pipefail
REPO_ROOT=$(git rev-parse --show-toplevel)
SKILL_ROOT="$REPO_ROOT/.codex/skills/raiko2-service-regression"
PYTHON=${PYTHON:-python3}
if [ -n "${VIRTUAL_ENV:-}" ]; then PYTHON="$VIRTUAL_ENV/bin/python"; fi
PROVER=${PROVER:-0x70997970C51812dc3A010C7d01b50e0d17dc79C8}
WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/raiko2-service-regression.XXXXXX")
```

## Workflow

1. Record `git rev-parse HEAD`, the host source/image identity, remote-provider identity, network,
   endpoint, proposal IDs, and proof lanes. Confirm `curl -fsS "$RAIKO_RPC/health"` succeeds.

2. Discover the canonical proposal tuples once:

```bash
"$PYTHON" "$REPO_ROOT/scripts/regression/stress_shasta_proposal.py" \
  --network "$NETWORK" --l1-network "$L1_NETWORK" \
  --l1-rpc "$L1_RPC" --l2-rpc "$L2_RPC" \
  --raiko-rpc "$RAIKO_RPC" --proposal-ids "$PROPOSAL_IDS" \
  --discover-only --proposal-out "$WORK_DIR/proposals.json" \
  --polling-interval 5 --log-file "$WORK_DIR/discovery.log"
```

3. For each requested lane, run base proofs, then the aggregate:

```bash
for PROOF_TYPE in $PROOF_TYPES; do
  "$PYTHON" "$REPO_ROOT/scripts/regression/stress_shasta_proposal.py" \
    --network "$NETWORK" --l1-network "$L1_NETWORK" \
    --l1-rpc "$L1_RPC" --l2-rpc "$L2_RPC" \
    --raiko-rpc "$RAIKO_RPC" --proposal-ids "$PROPOSAL_IDS" \
    --prove-type "$PROOF_TYPE" --api-version v4 --prover "$PROVER" \
    --polling-interval 5 --log-file "$WORK_DIR/${PROOF_TYPE}-base.log"

  "$PYTHON" "$SKILL_ROOT/scripts/v4_aggregate.py" \
    --raiko-rpc "$RAIKO_RPC" --proposal-file "$WORK_DIR/proposals.json" \
    --expect-proposal-ids "$PROPOSAL_IDS" \
    --proof-type "$PROOF_TYPE" --prover "$PROVER" --poll-interval 15 2>&1 \
    | tee "$WORK_DIR/${PROOF_TYPE}-aggregate.log"
done
```

Keep aggregate submission separate. The stress script's exact `--proposal-ids` path completes base
proofs but does not flush and continuously poll its aggregate queue. Always pass `--api-version v4`;
the script default is v3. The base run is an independent lane check; the aggregate request can
register missing dependencies itself, but matching `PROVER` makes it reuse the completed base tasks.
The commands run lanes sequentially and stop at the first failure. Repeated aggregate POST polling
also increments duplicate-request logs and metrics on the host.

## Acceptance

Pass only when every requested base proposal and aggregate reaches `completed`. Exact-ID discovery
and base proving now exit nonzero when any requested proposal is missing or fails. Report proposal
tuples, proof type, task ID, final status, proof byte length, elapsed seconds, and host/remote errors.
Do not paste full proofs or secrets. Treat `failed`, `cancelled`, timeout, changing task IDs, missing
aggregate proof, HTTP/ACL failure, or non-contiguous proposal metadata as failures; preserve the work
directory and exact commands for diagnosis.
