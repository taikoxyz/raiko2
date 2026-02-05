# Shasta Regression Tool

## Setup

```bash
python -m venv .venv
source .venv/bin/activate
pip install -r script/regression/requirements.txt
```

## Build binaries

```bash
script/regression/prepare_regression.sh
```

## Run

```bash
python script/regression/shasta_regression.py --config script/regression/config/shasta_regression_devnet.json --range 1000:1010

# Or run the most recent completed proposals (skips the current in-progress proposal).
python script/regression/shasta_regression.py --config script/regression/config/shasta_regression_devnet.json --count 3
```

## Outputs

Artifacts are written under `test/regression/shasta/`.
