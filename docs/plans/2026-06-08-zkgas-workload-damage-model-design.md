# ZKGas Workload Damage Model Design

## Goal

Quantify the maximum SP1 proving workload that can fit inside an Ethereum block gas limit, then use
that surface to evaluate candidate zk gas tables and block zk gas limits.

The first output is measurement, not enforcement. Once the maximum workload surface is known, protocol
review can decide whether the right control is:

- changing per-opcode or per-precompile zk gas costs
- changing the block zk gas limit
- changing both
- leaving the current schedule unchanged because the measured risk is acceptable

## Current Context

Alethia-reth already implements a Uzen zk gas schedule under `crates/evm/src/zk_gas/`:

- `schedule.rs` defines `ZkGasSchedule` with `block_limit`, opcode multipliers, precompile
  multipliers, and fixed spawn estimates.
- `uzen.rs` defines the current Uzen table and `BLOCK_ZK_GAS_LIMIT`.
- `adapter.rs` charges opcode steps as raw EVM gas multiplied by an opcode multiplier. It separately
  charges precompile calls using precompile gas spent multiplied by the precompile multiplier.
- `meter.rs` checks transaction and block zk gas accumulation against the schedule's block limit.

The raiko2 experiment suite under `experiments/opcode-gas/` measures local SP1 software
`proverGas` without running network proofs. That suite is the measurement source for this damage
model.

RISC0 can now be observed through proposal-level execute runs, but its primary local metric is cycle
count, not SP1 `proverGas`. Keep RISC0 on a backend-native cycle budget until a separate
cost-or-time calibration layer exists. A working SP1 heuristic such as 30M Ethereum gas to roughly
10B `proverGas` is not a RISC0 rule; RISC0 needs its own 30M Ethereum-gas to cycle anchor and its
own headroom analysis.

## Model

Treat Ethereum gas as the attacker's execution budget and SP1 prover gas as the cost objective.

```text
maximize   W(block)
subject to EthGas(block) <= L_eth
```

Where:

- `W(block)` is measured SP1 workload, approximated by software `proverGas`.
- `L_eth` is the block Ethereum gas limit.

For backend comparison, compute this model per backend first:

```text
W_sp1(block)  = measured SP1 prover_gas
W_r0(block)   = measured RISC0 padded cycles
```

Do not take a raw maximum across those values. A multi-backend envelope is only meaningful after each
backend metric has been mapped into a common cost unit, or after reviewers intentionally choose
separate SP1 and RISC0 limits and compare each block against the relevant backend-native limit.

For a single opcode or precompile scenario:

```text
eth_only_damage_i =
  floor(L_eth / eth_gas_per_unit_i) * measured_workload_per_unit_i

damage_ratio_i =
  measured_workload_per_unit_i / eth_gas_per_unit_i
```

This answers the first question: with no zk gas filter, which opcode or precompile produces the most
proving workload per Ethereum gas unit?

Then evaluate a candidate zk gas table and block zk gas limit:

```text
zkgas_per_unit_i =
  eth_gas_per_unit_i * zkgas_multiplier_i

candidate_damage_i =
  min(
    floor(L_eth / eth_gas_per_unit_i),
    floor(L_zk / zkgas_per_unit_i)
  ) * measured_workload_per_unit_i
```

This answers the second question: for a candidate table and `L_zk`, how much of the eth-valid
workload surface remains reachable?

## Decision Surface

The output must support policy decisions instead of hard-coding a single conclusion. Each candidate
schedule should report:

```text
candidate | max eth-only damage | max after zkgas | attack reduction |
p99 normal zk_util | normal reject count | top risky opcodes/precompiles
```

This lets reviewers choose between:

- raising selected multipliers when one opcode dominates the attack surface
- lowering `L_zk` when the whole block budget is too high
- raising `L_zk` when realistic blocks are too close to the limit
- keeping the table but accepting a known prover-cost envelope

## Realistic Usage Guardrail

The model should not optimize only against synthetic attack workloads. A multiplier can be high when
measurement shows it is expensive to prove, but the report must show whether it filters realistic EVM
usage.

For every real block or app benchmark:

```text
zk_util = ZKGas_table(workload) / L_zk
```

The report should include:

- anchor and system transaction pass/fail
- historical or devnet block reject count
- p50, p95, and p99 `zk_util`
- worst normal block by `zk_util`
- top opcode or precompile contributors for normal workloads

The intended interpretation is:

- zk gas may filter eth-valid adversarial proving-heavy workloads.
- zk gas should not filter anchor, system, or representative normal workloads.
- if a high multiplier causes normal workloads to approach or exceed `L_zk`, reviewers should either
  lower that multiplier, raise `L_zk`, or classify the workload as outside the supported envelope.

## Experiment Phases

### Phase 1: Synthetic Damage Frontier

Use `experiments/opcode-gas` fitted coefficients to compute:

- `measured_workload_per_unit`
- `eth_gas_per_unit`
- `damage_ratio`
- `eth_only_damage` for a chosen `L_eth`

This phase is independent of the current alethia-reth table.

For opcode coefficients, prefer the `sp1-revm-opcode-lab` fit once it has stable coverage and fit
quality. Keep the mini `sp1-opcode-lab` fit as a fast smoke/regression signal because it deliberately
removes revm execution context.

### Phase 2: Candidate Schedule Sweep

For each candidate table and `L_zk`, compute:

- reachable unit count under eth gas only
- reachable unit count under eth gas plus zk gas
- `candidate_damage`
- attack reduction relative to eth-only damage
- whether eth gas or zk gas is the binding resource

Start with the current Uzen table as candidate zero.

### Phase 3: Realistic Workload Impact

Run candidate tables against realistic workload data:

- devnet or historical proposals when available
- anchor/system paths
- focused app benchmarks such as ERC20 transfer, bridge, swap, and legitimate precompile-heavy use

This phase requires opcode/precompile contribution accounting from real execution. Until that is
available, the report should clearly mark realistic impact as pending rather than inferred from the
synthetic lab.

## V1 Output

The first statistical report should be intentionally small:

- input: `experiments/opcode-gas` fit results from the smoke suite
- input: current Uzen multipliers for the measured smoke cases
- parameters: `L_eth`, `L_zk`
- output: JSON plus Markdown damage report

The Markdown report should include:

- eth-only damage table
- current-Uzen containment table
- top measured attack surface by `damage_ratio`
- cases where current Uzen zk gas is the binding resource
- cases where eth gas remains the binding resource
- explicit coverage gaps

## Known Gaps

The current smoke suite is enough to validate the report structure, but not enough for final schedule
decisions. Required follow-up work:

- expand opcode coverage beyond ADD, MUL, KECCAK256, identity, and SHA256
- add argument and input-size sweeps for precompiles
- add memory expansion dimensions for copy and hashing operations
- add call wrapper and spawn estimate validation
- add Taiko/reth-context revm lab inputs for fork config, block env, realistic tx env, and
  warm/cold state
- add realistic block and app benchmark contribution accounting
- decide whether the table remains one-dimensional or needs scenario-specific dimensions
