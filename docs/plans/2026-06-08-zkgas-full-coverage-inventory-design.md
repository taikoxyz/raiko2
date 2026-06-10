# ZKGas Full Coverage Inventory Design

## Goal

Track coverage for every Uzen zk gas opcode and precompile entry, then expand measurement in stages.
The inventory is complete even when measurement is not.

## Scope

The source table is alethia-reth's Uzen schedule:

- opcode multipliers from `crates/evm/src/zk_gas/uzen.rs`
- precompile multipliers from the same file
- block limit and spawn estimates remain comparison inputs, not measurement targets

Undefined opcode bytes are not measurement targets. They should remain represented by the fail-safe
default in alethia-reth, not by synthetic lab measurements.

## Coverage States

Each inventory row has one status:

- `measured`: present in the current experiment manifest and runnable by the lab suite
- `planned_pure_opcode`: valid opcode and suitable for the standalone opcode lab
- `needs_state_or_revm`: depends on state, environment, warm/cold access, logs, or full REVM context
- `needs_spawn_wrapper`: CALL/CREATE family; must distinguish spawn/no-spawn and precompile dispatch
- `needs_precompile_body`: precompile direct-body measurement exists conceptually but is not wired yet
- `needs_precompile_wrapper`: precompile must also be measured through `STATICCALL`/real dispatch
- `not_measured_zero_or_halting`: STOP/RETURN/REVERT/INVALID/SELFDESTRUCT style entries need special
  interpretation instead of ordinary marginal loops

## Measurement Order

### Phase 1: Inventory

Add an `inventory` report command under `experiments/opcode-gas/opcode_gas.py`.

The command should output:

- all Uzen opcode entries
- all Uzen precompile entries
- multiplier
- name
- coverage status
- matching manifest case, if present

### Phase 2: Pure Opcode Expansion

Expand standalone opcode-lab coverage for opcodes that can be measured without state:

- arithmetic: `SDIV`, `SMOD`, `ADDMOD`, `MULMOD`, `EXP`, `SIGNEXTEND`
- comparison/bitwise/shift: `SLT`, `SGT`, `ISZERO`, `NOT`, `BYTE`, `SHL`, `SHR`, `SAR`
- stack: `PUSH0..PUSH32`, `DUP1..DUP16`, `SWAP1..SWAP16`, `POP`
- memory without external state: `MLOAD`, `MSTORE`, `MSTORE8`, `MCOPY`, `MSIZE`
- fixed control smoke: `PC`, `GAS`, `JUMP`, `JUMPI`, `JUMPDEST`

These still use experiment templates. Their slopes are smoke damage signals until validated against a
full REVM path.

### Phase 3: Precompile Direct Body

Expand `PrecompileLabInput` beyond `identity` and `sha256` for direct body measurement:

- `ecrecover`
- `ripemd160`
- `modexp`
- BN254 add/mul/pairing
- `blake2f`
- KZG point evaluation
- BLS12 operations

Each precompile needs deterministic valid inputs. Invalid-input cases should be separate scenarios.

### Phase 4: Real Dispatch And Realistic Workloads

The `sp1-revm-opcode-lab` path is now the base real-dispatch opcode path. Continue from that path
rather than replacing it with a lower-level direct interpreter call. The remaining work is to add
realistic context around revm execution:

- `STATICCALL` wrapper overhead
- precompile dispatch metering
- CALL/CREATE spawn estimates
- stateful opcodes and warm/cold dimensions
- historical proposal and app benchmark contribution accounting
- Taiko fork config, block env, and realistic transaction env

## Output

Inventory reports should be stable enough for handoff:

```text
kind | id | name | multiplier | status | manifest_case
```

The full coverage target is:

- zero unclassified Uzen entries
- all measured rows linked to a manifest case
- all unmeasured rows marked with a reason and next phase
