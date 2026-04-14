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
python scripts/regression/shasta_regression.py --config scripts/regression/config/shasta_regression_devnet.json --range 1000:1010

# Or run the most recent completed proposals (skips the current in-progress proposal).
python scripts/regression/shasta_regression.py --config scripts/regression/config/shasta_regression_devnet.json --count 3

# Resolve the latest completed proposal spans without running preflight or guest binaries.
python scripts/regression/shasta_regression.py --config scripts/regression/config/shasta_regression_devnet.json --count 3 --discover-only
```

- Proof backend defaults to `native`; switch with `--proof-type sp1`.
- Aggregation (`--aggregate N`) is supported only when `--proof-type sp1`.
- `--discover-only` prints canonical proposal spans as JSON so live tests can reuse latest completed proposals instead of tip-based proposals.

## Direct Proposal Check

For a single L2 block, use the stress discovery helper to resolve the containing Shasta proposal
tuple without submitting work to `raiko2`:

```bash
python scripts/regression/stress_shasta_proposal.py \
  --network taiko_hoodi \
  --l1-network hoodi \
  --l2-block-range 7225500,7225501 \
  --discover-only \
  --proposal-out /tmp/proposal-7225500.discovery.json
```

The stress helper derives the default L1 RPC, L2 RPC, and Shasta inbox contract from
`config/chain_spec_list_default.json`. Use `--l1-rpc`, `--l2-rpc`, `--event-contract`,
`--abi-file`, and `--anchor-abi-file` only as overrides. Fork-specific ABI files live under
`scripts/regression/shasta/`.

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

For a host-side native replay after preflight, run the guest launcher against the generated input:

```bash
cargo run -r -p guest-launcher -- \
  --proof-type native \
  --mode prove \
  --input /tmp/proposal-17771.json \
  --output /tmp/proposal-17771.native-proof.json
```

For a direct SGX remote check without a `raiko2` server, convert the guest input into the gaiko2
Shasta request envelope and post it to the SGX prover:

```bash
cargo run -r -p raiko2-prover --example dump_gaiko2_shasta_fixture -- \
  /tmp/proposal-17771.json \
  /tmp/proposal-17771.gaiko2-request.json

curl -sS \
  -H 'content-type: application/json' \
  --data-binary @/tmp/proposal-17771.gaiko2-request.json \
  "${RAIKO2_GAIKO2_BASE_URL:-http://127.0.0.1:9090}/prove/shasta"
```

SGX checks still require the SGX prover stack or a remote SGX prover. `preflight` only builds and
optionally validates the `GuestInput`; it does not launch SGX by itself. For `sgxgeth`, point the
same request at the gaiko2 SGXGETH service instead of `raiko2-sgx-prover`.

## Outputs

Artifacts are written under `test/regression/shasta/`.

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

The file-based regression harness in this directory still only supports `native` and `sp1`.
Use the SGX stack for API-driven regression and remote-server smoke testing.
