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

## Outputs

Artifacts are written under `test/regression/shasta/`.
