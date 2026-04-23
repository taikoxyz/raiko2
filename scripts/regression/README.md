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

For a single known proposal, use `preflight` directly instead of starting the `raiko2` server or
running discovery. The proposal tuple stays explicit, while RPC URLs and chain IDs come from the
chain specs selected by `--l1-network` and `--network`:

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
