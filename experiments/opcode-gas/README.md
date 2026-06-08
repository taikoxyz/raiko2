# SP1 Opcode Prover Gas Experiment

This experiment suite is a local, repeatable scaffold for measuring opcode and precompile
contribution to SP1 software `proverGas`.

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

Fit reports from raw runs:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit \
  --runs /tmp/raiko2-opcode-gas/raw-runs.jsonl \
  --out /tmp/raiko2-opcode-gas/report
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

The `run` command invokes `target/release/guest-launcher` directly with `--stage opcode-lab` or
`--stage precompile-lab`, always using `--proof-type sp1 --mode execute --sp1-prover local`. It
batches generated inputs into one launcher process per lab stage through `--input-list` and
`--jsonl-out`, so the expensive SP1 executor startup cost is paid once per stage instead of once per
variant. It does not use `cargo run`, does not submit network proofs, and does not access live
L1/L2 RPC.

The `damage` command is measurement-only. It answers how much SP1 workload fits under an Ethereum gas
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

The opcode-lab guest is a small experiment-only bytecode interpreter. The precompile-lab guest
currently supports direct body measurements for `identity(0x04)` and `sha256(0x02)`. These are
enough to close the local SP1 `proverGas` loop for generated stack, memory, and basic precompile
templates, but they are not a full alethia-reth/revm execution path yet. `STATICCALL` wrapper cost,
warm/cold account access, precompile argument sweeps, and full block execution are TODOs for later
suites.

The current smoke manifest covers arithmetic/comparison/bitwise pure stack opcodes through `SAR`,
`KECCAK256`, selected stack/memory opcodes, and the two direct precompile cases. Stack slopes include
the repeated operand setup needed by the lab bytecode, so use them as smoke damage signals and
regression anchors, not as final consensus coefficients.

For tiny stack and memory templates, always inspect `fit.json` before using `damage.md`. Low R2 means
the current variant counts are too small or the template has setup/cleanup noise. In that case, rerun
with larger variants or a cleaner template before treating the slope as a candidate coefficient.
