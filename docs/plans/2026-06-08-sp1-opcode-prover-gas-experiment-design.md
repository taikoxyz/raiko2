# SP1 Opcode Prover Gas Experiment Design

## Goal

Build a repeatable experiment suite that maps EVM opcode and precompile usage to SP1 software
`proverGas`, while minimizing block-level noise. The first version should make the suite easy to
regenerate after SP1, raiko2, alethia-reth, or zkVM guest changes. Producing a consensus-ready
replacement for the current alethia-reth Uzen table is not required for V1.

## Scope

V1 focuses on a one-dimensional opcode/precompile cost suite that is compatible with the current
alethia-reth zk gas table shape:

- one multiplier per opcode byte
- one multiplier per precompile low-byte address
- fixed spawn estimates for CALL and CREATE families

Scenario-specific dimensions such as warm/cold `SLOAD`, argument-dependent precompile costs, and
memory-size curves are important, but they do not fit the current table cleanly. V1 records them as
metadata and TODOs instead of exporting separate multipliers.

## Current Table

The current alethia-reth model lives under `crates/evm/src/zk_gas/` in alethia-reth:

- `uzen.rs` defines the fixed Uzen schedule.
- `schedule.rs` defines the table shape.
- `adapter.rs` records REVM opcode execution and charges `raw_evm_gas * multiplier`.
- `meter.rs` accumulates checked block and transaction zk gas.

This table is useful as a seed and comparison target, but the experiment should not directly mutate
it. The suite exports a candidate report first; table updates are a separate review step.

## Measurement Strategy

Use matched synthetic variants. For each opcode or precompile, generate several variants with
different target operation counts:

- baseline: zero target operations
- target variants: `N`, `2N`, `3N`, and optionally larger counts
- identical block environment
- identical transaction count
- identical caller, target account, chain spec, and calldata shape
- identical setup/control/cleanup bytecode shape where practical

Fit:

```text
prover_gas = intercept + slope * target_feature
```

The intercept absorbs fixed guest, block, transaction, setup, and cleanup costs. The slope estimates
the marginal SP1 prover gas contribution of the target opcode or precompile.

## Why Not Real Proposals For Fitting

Real Shasta proposals carry useful production signal, but they are too noisy for first-pass opcode
fitting. They mix:

- block and proposal invariant costs
- header and manifest costs
- witness size and state-shape effects
- storage warm/cold behavior
- contract interactions
- precompile and memory-size side effects

Use real proposals as holdout validation only:

```text
observed_sp1_prover_gas(real proposal) vs predicted_sp1_prover_gas(model)
```

## Experiment Layout

```text
experiments/opcode-gas/
  README.md
  manifests/
    sp1-smoke.toml
    sp1-full.toml
  templates/
    arithmetic.toml
    memory.toml
    storage.toml
    call-precompile.toml
  fixtures/
    <suite>/<opcode>/<scenario>/<variant>.json
  reports/
    <run-id>/
      environment.json
      raw-runs.jsonl
      fit.json
      coefficients.json
      uzen-vs-fit.md
      schedule-candidate.rs
```

## Suite Commands

V1 should expose stable commands:

```bash
python3 experiments/opcode-gas/opcode_gas.py generate \
  --manifest experiments/opcode-gas/manifests/sp1-smoke.toml \
  --out /tmp/raiko2-opcode-gas/fixtures

python3 experiments/opcode-gas/opcode_gas.py run \
  --fixtures /tmp/raiko2-opcode-gas/fixtures \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_opcode_lab.elf \
  --out /tmp/raiko2-opcode-gas/runs/smoke/raw-runs.jsonl

python3 experiments/opcode-gas/opcode_gas.py fit \
  --runs /tmp/raiko2-opcode-gas/runs/smoke/raw-runs.jsonl \
  --out /tmp/raiko2-opcode-gas/runs/smoke
```

The `run` command must call `target/release/guest-launcher` directly with `--stage opcode-lab` or
`--stage precompile-lab`. It should batch the generated inputs through `--input-list` and
`--jsonl-out`; otherwise the loop pays SP1 profiling startup cost once per variant. It must not use
`cargo run -r -p xtask -- bench-guest`, because that wrapper can rebuild host binaries and slow down
iteration. It also must use `--sp1-prover local` and `--mode execute`, so it never submits a network
proof.

## Data Model

Each generated case records:

- opcode or precompile id
- scenario name
- variant id
- target operation count
- expected target raw EVM gas, when known
- full expected opcode count vector, when available
- fixture path
- generator version

Each raw run records:

- case metadata
- `prover_gas`
- `wall_time_ms`
- `total_instruction_count`
- `total_syscall_count`
- `cycle_tracker`
- `public_values`
- `exit_code`

Each fit records:

- slope
- intercept
- R2
- residuals
- sample count
- skipped/outlier cases
- comparison against current Uzen multiplier

## V1 Coverage

Start with a smoke suite:

- arithmetic: `ADD`, `MUL`, `DIV`, `MOD`, `ADDMOD`, `MULMOD`
- bitwise and comparison: `AND`, `OR`, `XOR`, `EQ`, `LT`, `GT`, `ISZERO`
- memory: `MLOAD`, `MSTORE`, `MSTORE8`, `KECCAK256`, `MCOPY`
- stack/control: `PUSH0`, `PUSH1`, `DUP1`, `SWAP1`, `POP`, `JUMP`, `JUMPI`
- environment: `ADDRESS`, `CALLER`, `CALLVALUE`, `TIMESTAMP`, `NUMBER`, `BASEFEE`
- precompile smoke: direct body measurements for `identity(0x04)` and `SHA256(0x02)`
- later call/precompile smoke: `STATICCALL` wrapper cost, `ECRECOVER`, BN precompiles, and modexp

The full suite can expand from this list after the generator and fit pipeline are stable.

## TODO: Dimensions Not Exported In V1

These are intentionally not exported as separate table entries in V1 because the current Uzen table
does not support them directly:

- warm vs cold storage/account access
- memory expansion slope vs opcode base cost
- input-size sweeps for `KECCAK256`, copy opcodes, and precompiles
- precompile argument-shape sweeps
- `STATICCALL` wrapper overhead for precompile calls
- call/create spawn vs no-spawn split beyond the current fixed spawn estimate comparison
- multi-dimensional metering model

The suite should still preserve enough metadata to add these dimensions later.

## Success Criteria

- The smoke suite can generate fixtures, run SP1 local execute, and fit coefficients without live
  L1/L2 RPC.
- The V1 lab guest keeps block/proposal overhead out of the measured loop.
- The report compares fitted coefficients against the current alethia-reth Uzen table.
- Direct guest-launcher execution is used for cached fixtures.
- The suite records environment metadata so runs are comparable across zkVM upgrades.
- V1 keeps scenario-specific dimensions as TODOs instead of forcing them into the current one-table
  model.
