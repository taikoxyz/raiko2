# Opcode Workload Metric Experiment

This experiment suite is a local, repeatable scaffold for measuring opcode and precompile
contribution to zkVM workload metrics. The direct opcode/precompile lab currently runs on SP1
software `proverGas`; opcode fixtures can run through either the fast synthetic guest or a
revm-backed guest; proposal-level GuestInput runs can also collect RISC0 cycle metrics.

V1 intentionally exports a one-dimensional result shape compatible with the current alethia-reth
Uzen table. Warm/cold storage access, argument-dependent precompile cost, and memory-size sweeps are
tracked as future dimensions, not V1 coefficients.

## Commands

Build or refresh the SP1 lab ELFs, then build a release launcher once:

```bash
cargo run -r -p xtask -- build-guest sp1 --bench
cargo build -r -p guest-launcher --features sp1-sdk/profiling
```

Generate smoke case metadata and lab inputs:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate \
  --manifest experiments/opcode-gas/manifests/sp1-smoke.toml \
  --out /tmp/raiko2-opcode-gas/fixtures
```

Run existing `guest-input.json` cases with a prebuilt launcher:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run \
  --fixtures /tmp/raiko2-opcode-gas/fixtures \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_opcode_lab.elf \
  --precompile-elf crates/guests/elf/sp1_precompile_lab.elf \
  --out /tmp/raiko2-opcode-gas/raw-runs.jsonl
```

Run the same opcode fixtures through the revm-backed SP1 guest:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run \
  --fixtures /tmp/raiko2-opcode-gas/fixtures \
  --guest-launcher target/release/guest-launcher \
  --opcode-stage revm-opcode-lab \
  --out /tmp/raiko2-opcode-gas/revm-raw-runs.jsonl
```

Run one real proposal GuestInput through RISC0 execute mode and write a normalized raw JSONL row:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-proposal \
  --guest-launcher target/release/guest-launcher \
  --guest-input /tmp/proposal-170-guest-input.json \
  --proof-type risc0 \
  --case proposal-170 \
  --target-raw-gas 30000000 \
  --out /tmp/raiko2-opcode-gas/risc0-proposal-170.jsonl
```

The RISC0 proposal path uses `guest-launcher --stage proposal --proof-type risc0 --mode execute`
and records `risc0_padded_cycles` as the primary workload metric. It also records
`risc0_user_cycles`, segment count, and segment `po2` histogram. It does not produce a proof and
does not call Boundless/Bonsai/network proving.

SP1 `prover_gas` and RISC0 cycle counts are backend-native metrics, not the same unit. The SP1
working heuristic that a 30M Ethereum-gas block maps to roughly 10B `proverGas` must not be applied
to RISC0. For RISC0, use a RISC0-native cycle budget, such as a working 30M Ethereum-gas to 10B-cycle
anchor, and report it separately until a cost or time calibration layer exists. Cross-backend
envelopes should compare normalized workload units, not `max(sp1_prover_gas, risc0_padded_cycles)`.

Fit reports from raw runs. SP1 lab runs default to `prover_gas`; RISC0 raw runs can select a cycle
metric explicitly:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit \
  --runs /tmp/raiko2-opcode-gas/raw-runs.jsonl \
  --out /tmp/raiko2-opcode-gas/report

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit \
  --runs /tmp/raiko2-opcode-gas/risc0-proposal-runs.jsonl \
  --metric risc0_padded_cycles \
  --out /tmp/raiko2-opcode-gas/risc0-report
```

Compute an eth-limit damage frontier and current-Uzen containment report from fit results:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py damage \
  --fit /tmp/raiko2-opcode-gas/report/fit.json \
  --manifest experiments/opcode-gas/manifests/sp1-smoke.toml \
  --eth-gas-limit 30000000 \
  --zk-gas-limit 100000000 \
  --out /tmp/raiko2-opcode-gas/damage
```

Write the Uzen opcode/precompile coverage inventory:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py inventory \
  --manifest experiments/opcode-gas/manifests/sp1-smoke.toml \
  --out /tmp/raiko2-opcode-gas/inventory
```

The `run` command invokes `target/release/guest-launcher` directly with `--stage opcode-lab`,
`--stage revm-opcode-lab`, or `--stage precompile-lab`, always using `--proof-type sp1 --mode
execute --sp1-prover local`. It batches generated inputs into one launcher process per lab stage
through `--input-list` and `--jsonl-out`, so the expensive SP1 executor startup cost is paid once
per stage instead of once per variant. It does not use `cargo run`, does not submit network proofs,
and does not access live L1/L2 RPC.

The `run-proposal` command invokes `guest-launcher` directly for one real Shasta proposal
`GuestInput`. Use this path for block/proposal-level calibration and cross-zkVM observation. It is
not an opcode coefficient generator by itself because one proposal is one workload sample, not a
controlled target-count sweep. Its `--target-raw-gas` value is report metadata for comparing runs;
it does not mean the proposal itself is exactly one 30M-gas block.

The `damage` command is measurement-only. It answers how much measured workload fits under an Ethereum gas
limit, then shows how much of that surface remains reachable under the current-Uzen smoke multipliers
and a chosen block zk gas limit. Realistic block and app impact is a later input to the same report,
not inferred from the smoke lab.

The `inventory` command is coverage-only. It lists every opcode and precompile entry from the current
Uzen table and marks whether the smoke manifest measures it or which future measurement path is
needed.

For a quick smoke after changing launcher code, `target/debug/guest-launcher` built with
`cargo build -p guest-launcher --features sp1-sdk/profiling` also works. Use the release binary for
longer research loops once it has been rebuilt.

## Current Limitation

The default opcode-lab guest is a small experiment-only bytecode interpreter, not revm and not a
complete EVM. It uses narrow synthetic stack and memory semantics to isolate guest-side opcode
workload. Use it as a fast smoke signal and regression anchor, not as evidence that the measured
slope is the exact cost of real EVM execution.

The `revm-opcode-lab` guest runs the same `OpcodeLabInput.bytecode` through revm with a fixed
Prague/mainnet benchmark transaction and empty benchmark database. This adds real revm dispatch,
stack, memory, and gas semantics while still removing block/proposal noise. It is the better
candidate for opcode coefficient tuning. It still is not a full Taiko block: stateful warm/cold
accesses, realistic account/storage distributions, CALL/CREATE wrappers, and block context remain
separate dimensions.

The smoke manifest now expands to every Uzen opcode that can be isolated without state, environment,
or CALL/CREATE wrapper semantics, including arithmetic, comparison, bitwise, stack, fixed
control-flow, and memory-copy templates.

The precompile-lab guest supports direct body measurements for every Uzen precompile row through
fixed deterministic inputs. These are direct body calls, not `STATICCALL` dispatch measurements.
`STATICCALL` wrapper cost, warm/cold account access, precompile argument sweeps, stateful opcodes,
and full block execution are TODOs for later suites.

Stack, control, memory, and precompile slopes include the setup needed by the lab templates, so use
them as smoke damage signals and regression anchors, not as final consensus coefficients.

The real proposal `GuestInput` path is the calibration layer for full block behavior.

For tiny stack and memory templates, always inspect `fit.json` before using `damage.md`. Low R2 means
the current variant counts are too small or the template has setup/cleanup noise. In that case, rerun
with larger variants or a cleaner template before treating the slope as a candidate coefficient.
