# Shasta Regression Tool

## Setup

```bash
python -m venv .venv
source .venv/bin/activate
pip install -r scripts/regression/requirements.txt
```

## Build binaries

```bash
scripts/regression/prepare_regression.sh
```

## Run

```bash
# Resolve the latest completed proposal spans without running preflight or guest binaries.
python scripts/regression/proposal_regression.py --config scripts/regression/config/proposal_regression_devnet.json --count 3 --discover-only

# Run preflight and guest-launcher from full proposal metadata produced by the stress helper.
python scripts/regression/proposal_regression.py \
  --config scripts/regression/config/proposal_regression_devnet.json \
  --proposal-metadata /tmp/proposal-730.discovery.json \
  --proof-type sp1
```

- Proof backend defaults to `native`; switch with `--proof-type sp1`.
- Aggregation (`--aggregate N`) is supported only when `--proof-type sp1`.
- `--discover-only` prints canonical proposal spans as JSON so live tests can reuse latest completed proposals instead of tip-based proposals.
- Current `preflight` requires the full Shasta proposal tuple: proposal id, L1 inclusion block,
  last anchor block number, and L2 block range. `--range` and `--count` only derive L2 spans, so
  non-discover regression runs must use `--proposal-metadata`.
- The checked-in devnet regression config and default chain specs pin the internal devnet RPC
  endpoints used for proposal discovery and preflight.

## Which Tool

Use `stress_proposal.py` for live regression against a running `raiko2` host. It resolves
full Shasta proposal metadata from L1/L2, submits HTTP proof requests to `--raiko-rpc`, and is the
right path for SGX, SGXGETH, remote-prover, queue, and API status checks.

Use `proposal_regression.py` for file-based local replay. It runs `preflight` to materialize
`GuestInput` and then runs `guest-launcher` without a `raiko2` server. For current Shasta preflight,
feed it metadata from `stress_proposal.py --discover-only --proposal-out`; `--range` and
`--count` are only lightweight L2 span discovery helpers.

For SP1 prover-gas research, use `stress_proposal.py` only to discover the proposal tuple,
then run `preflight` once and reuse the generated `GuestInput` with `xtask bench-guest sp1
--skip-build-guest`. That keeps the optimization loop off live RPC and avoids SP1 network proving
costs.

## Direct Proposal Check

For a single L2 block, use the stress discovery helper to resolve the containing Shasta proposal
tuple without submitting work to `raiko2`:

```bash
python scripts/regression/stress_proposal.py \
  --network taiko_hoodi \
  --l1-network hoodi \
  --l2-block-range 7225500,7225501 \
  --discover-only \
  --proposal-out /tmp/proposal-7225500.discovery.json
```

The stress helper derives the default L1 RPC, L2 RPC, and Shasta inbox contract from
`config/chain_spec_list_default.json`. For devnet, pass the internal RPC overrides explicitly:

```bash
python scripts/regression/stress_proposal.py \
  --network taiko_dev \
  --l1-network taiko_dev_l1 \
  --l1-rpc https://l1rpc.internal.taiko.xyz \
  --l2-rpc https://rpc.internal.taiko.xyz \
  --l2-block-range 239062,239445 \
  --discover-only \
  --proposal-out /tmp/proposal-730.discovery.json
```

For a known proposal-id range against the devnet host, discover full metadata first and then submit
to the local `raiko2` API:

For real v4 proposal plus aggregate service validation, use the repository-local
`raiko2-service-regression` skill. The commands below show base-proof requests only.

```bash
ids=$(seq -s, 203 302)

python3 scripts/regression/stress_proposal.py \
  --network taiko_dev \
  --l1-network taiko_dev_l1 \
  --l1-rpc https://l1rpc.internal.taiko.xyz \
  --l2-rpc https://rpc.internal.taiko.xyz \
  --raiko-rpc http://127.0.0.1:18080 \
  --proposal-ids "$ids" \
  --prove-type sgx \
  --discover-only \
  --proposal-out /tmp/devnet-proposals-203-302.json \
  --polling-interval 5 \
  --log-file /tmp/devnet-discover-203-302.log

python3 scripts/regression/stress_proposal.py \
  --network taiko_dev \
  --l1-network taiko_dev_l1 \
  --l1-rpc https://l1rpc.internal.taiko.xyz \
  --l2-rpc https://rpc.internal.taiko.xyz \
  --raiko-rpc http://127.0.0.1:18080 \
  --proposal-ids "$ids" \
  --prove-type sgx \
  --api-version v4 \
  --prover 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 \
  --polling-interval 5 \
  --log-file /tmp/devnet-sgx-203-302.log

python3 scripts/regression/stress_proposal.py \
  --network taiko_dev \
  --l1-network taiko_dev_l1 \
  --l1-rpc https://l1rpc.internal.taiko.xyz \
  --l2-rpc https://rpc.internal.taiko.xyz \
  --raiko-rpc http://127.0.0.1:18080 \
  --proposal-ids "$ids" \
  --prove-type sgxgeth \
  --api-version v4 \
  --prover 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 \
  --polling-interval 5 \
  --log-file /tmp/devnet-sgxgeth-203-302.log
```

Use `--l1-rpc`, `--l2-rpc`, `--event-contract`, `--abi-file`, and `--anchor-abi-file` only as
overrides for non-standard environments. Fork-specific ABI files live under
`scripts/regression/abi/`.

Then run `preflight` directly against one discovered proposal. The proposal tuple stays explicit,
while RPC URLs and chain IDs come from the chain specs selected by `--l1-network` and `--network`:

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

Use `--rpc-url` and `--l1-rpc-url` only when you need to override the chain spec defaults. Use
`--l2-chain-id` and `--l1-chain-id` only for custom chain spec files or compatibility with older
commands.

If you already know the proposal tuple, you can skip discovery and run `preflight` directly with
the same chain-spec-derived defaults.

For a host-side native replay after preflight, run the guest launcher against the generated input:

```bash
cargo run -r -p guest-launcher -- \
  --proof-type native \
  --mode prove \
  --input /tmp/proposal-17771.json \
  --output /tmp/proposal-17771.native-proof.json
```

For a direct `raiko2-sgx-prover` check without a `raiko2` server, convert the guest input into a
Shasta request envelope that includes the full `GuestInput` and post it to the SGX prover:

```bash
cargo run -r -p raiko2-prover --example dump_gaiko2_shasta_fixture -- \
  target/regression/proposal-17771.json \
  target/regression/proposal-17771.raiko2-sgx-request.json

curl -sS \
  -H 'content-type: application/json' \
  --data-binary @/tmp/proposal-17771.raiko2-sgx-request.json \
  "${RAIKO2_REMOTE_SGX_BASE_URL:-http://127.0.0.1:9090}/prove/shasta"
```

SGX checks still require the SGX prover stack or a remote SGX prover. `preflight` only builds and
optionally validates the `GuestInput`; it does not launch SGX by itself.

For a fixed Masaya fork-boundary replay case, use the checked-in
`taiko_masaya/shasta_unzen_transition` fixture suite. It captures proposals `25125`, `25126`,
and `25127`, with `25127` spanning the `SHASTA -> UNZEN` transition and the first two proposals
serving as pre-fork controls.

## Outputs

Artifacts are written under `test/regression/proposal/`.

## SGX Regression Stack

For SGX-backed API regression, start the dedicated compose stack first:

```bash
cp docker/.env.sgx.regression.sample docker/.env.sgx.regression
docker compose --env-file docker/.env.sgx.regression -f docker/docker-compose.sgx.regression.yml --profile init up raiko2-sgx-init gaiko2-sgxgeth-init
docker compose --env-file docker/.env.sgx.regression -f docker/docker-compose.sgx.regression.yml up -d
```

That stack starts:

- `raiko2-sgx-prover` for the `sgx` lane
- `gaiko2-sgxgeth` for the `sgxgeth` lane

If you also want a dockerized `raiko2`:

```bash
docker compose --env-file docker/.env.sgx.regression -f docker/docker-compose.sgx.regression.yml --profile raiko2 up -d raiko2
```

The dockerized or local `raiko2` process can then target both remote lanes in the same stack:

- `proof_type=sgx` -> `raiko2-sgx-prover`
- `proof_type=sgxgeth` -> `gaiko2-sgxgeth`

The file-based regression harness in this directory still only supports `native` and `sp1`.
Use the SGX stack for API-driven regression and remote-server smoke testing.
