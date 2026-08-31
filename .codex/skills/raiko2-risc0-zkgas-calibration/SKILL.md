---
name: raiko2-risc0-zkgas-calibration
description: Use when sampling RISC0 proposal cycles, recalibrating the zkGas model, or checking the packaged Boundless estimation artifact in raiko2.
---

# RISC0 zkGas Calibration

Keep collection, policy review, and packaging separate.

1. Read `experiments/risc0-zkgas/README.md`; use its finite collector with the proposal guest in
   `execute` mode only. Never prove or submit to Boundless.
2. Review candidate ranges and split coverage explicitly. Domain ranges are manual policy: do not
   infer across unobserved gaps.
3. Pass an existing Python 3.11+ venv as the recipe's `PYTHON_BIN` environment variable. Run
   `just --show update-risc0-zkgas-model` and
   `"$PYTHON_BIN" scripts/modeling/risc0_zkgas_model.py --help`; do not invoke the script through its
   shebang because that bypasses the selected environment.
4. Give every changed calibration a new fixture directory and set its input config model ID to
   `risc0-zkgas-m2-auto`; the generator resolves a content-addressed ID. It must reject collector
   provenance drift; never combine different guest or build cohorts.
5. Inspect the compact fixture/config diff, then run `just test-risc0-zkgas-model` and
   `just check-risc0-zkgas-model <new-fixture-dir> crates/prover/models/risc0-zkgas.json` with the same
   interpreter. A domain endpoint without an observation or an admitted observation beyond the 10%
   error budget must stop the refresh.
6. When promoting a new model, update every explicit audit pin in the Python and Rust fixture tests,
   the default fixture directory in the generator and `justfile`, and the documented API domain
   ranges. These are review gates, not a second coefficient source. Run the focused prover tests
   after updating them.

The current `risc0-zkgas-m2-v1` ID has exactly one compatibility exception: its initial
packaging-only metadata transition retained the ID because quote semantics did not change. Any later
artifact or calibration change must use the generated content-addressed ID. This workflow updates
repository artifacts only. Do not deploy, roll out, or change quote strategy.
