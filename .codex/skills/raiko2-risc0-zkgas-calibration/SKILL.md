---
name: raiko2-risc0-zkgas-calibration
description: Use when sampling RISC0 proposal cycles, recalibrating the zkGas model, or checking the packaged Boundless estimation artifact in raiko2.
---

# RISC0 zkGas Calibration

Keep collection, policy review, and packaging separate.

1. Read `experiments/risc0-zkgas/README.md`; use its finite collector with the proposal guest in
   `execute` mode only. Never prove or submit to Boundless.
2. Review split coverage and the proposal operating policy explicitly. The runtime policy is a
   global total-zkGas cap, independent of network and block count; changing that cap is a deliberate
   product decision, not an inferred calibration envelope.
3. Pass an existing Python 3.11+ venv as the recipe's `PYTHON_BIN` environment variable. Run
   `just --show update-risc0-zkgas-model` and
   `"$PYTHON_BIN" scripts/modeling/risc0_zkgas_model.py --help`; do not invoke the script through its
   shebang because that bypasses the selected environment.
4. Give every artifact, operating-policy, or calibration change a new versioned fixture directory;
   never rewrite an existing fixture directory in place. Set the new input config model ID to
   `risc0-zkgas-m2-auto`; the generator resolves a content-addressed ID. It must reject collector
   provenance drift; never combine different guest or build cohorts.
5. Inspect the compact fixture/config diff, then run `just test-risc0-zkgas-model` and
   `just check-risc0-zkgas-model <new-fixture-dir> crates/prover/models/risc0-zkgas.json` with the same
   interpreter. A proposal observation admitted by the configured global cap that exceeds the 10%
   error budget must stop the refresh.
6. When promoting a new model, update every explicit audit pin in the Python and Rust fixture tests;
   the default fixture directory in the generator and `justfile`; `config.example.toml`;
   `docs/API.md`; `docs/operations.md`; `experiments/risc0-zkgas/README.md`; the Boundless estimated
   quote design and implementation-plan documents; the zkGas cycle-estimation experiment plan under
   `docs/plans/`; the new fixture directory's own `README.md` recording its calibration scope; and
   this skill plus the zkGas guidance in `AGENTS.md`. These are review gates, not a second coefficient source. Run the focused prover
   tests after updating them.

Review proposal estimation against the accepted approximation contract: an implementation is
eligible when it matches the documented mechanical admission and fallback rules. The product
accepts estimated/local cycle mismatch, underquotes, overquotes, and in-cap network or block-count
combinations outside collected sample rectangles. The ten-percent budget gates concrete proposal
observations during model publication or refresh; it is not a proof of per-request accuracy. A
concrete newly collected in-policy proposal observation beyond that budget requires re-evaluating
the model, cap, or strategy. A theoretical future mismatch, lack of an untouched holdout, or lack
of observed rectangles does not change runtime availability. Use `evaluated` when exact local
cycles are required.

The legacy `risc0-zkgas-m2-v1` ID is a schema-v1 historical record. The current schema-v3 generator
and runtime parser reject schema-v1 and schema-v2 artifacts. Every schema-v3 artifact or calibration
change must use the generated content-addressed ID. This workflow updates repository artifacts only. Do not deploy,
roll out, or change quote strategy.

Aggregation does not use proposal zkGas calibration or a child-count activation set. Its artifact
contains a direct `per_child_mcycles` scalar, applied to every structurally valid, non-empty
aggregation input. Aggregation observations may justify a deliberate scalar update, but they must
never add `enabled`, `calibrated_counts`, or another membership allowlist. A future explicit global
child-count cap is a separate product-policy change; do not infer or generate one from observation
coverage.
