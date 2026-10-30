# Repeated Initcode Preparation Implementation Plan

> **For agentic workers:** use coherent implementation ownership, independent adversarial review and independent native behavioral verification under the existing autonomous workflow.

**Goal:** Preserve and measure24 repeated-initcode boundary cases against frozen a298 pricing.

**Architecture:** Add one Python producer/test module and usage docs; reuse the unchanged Rust fixture constructor, native quote, SP1 sampler and bounded harness.

**Tech Stack:** Existing Python virtual environment and ProgramRequest/GuestInput contracts; frozen binaries and ELF.

**Spec:** `docs/research/2026-10-30-initcode-preparation.md`.

## Global constraints

- Literal actual SP1 PGU / node-known zkGas remains the ranking metric; full quotes/deltas are separate.
- Freeze a298 prices, b049 native,0ead quote,1998 sampler,b76 ELF and original source manifests.
- All24 cases are one-block, one user,15M gas, empty calldata and evm-gas-only; normal EVM/proposal validation stays active.
- Preserve all controls/rejections/failures. No guest/Rust rebuild, tariff change, activation, merge or deployment.

## Review focus

- Exact constructed memory image, successful create-address guard and no hidden early exit.
- Repeats0 skips the child loop; empty input/output and nonempty output have distinct expected counts.
- CREATE2 salts are unique within a call; collision never masquerades as successful analysis.
- Native receipt/opcode/nonce gates catch off-by-one, wrong REVERT behavior and OOG.
- Data joins bind exact input/output/program/model identity; quote-only hash instrumentation is not actual PGU.

## Tasks

- [x] Producer: test first in `bin/zkgas/bench/tests/test_initcode_preparation.py`; implement `bin/zkgas/bench/cases/initcode_preparation.py` with `make_case(...)` and `prepare(output: Path) -> int`, the exact24 matrix, expected counts, reference links and catalog identities. Update bench README usage; run focused/Python suite and real frozen-native gates. Independent source review before commit.
- [x] Qualification: independently verify real native memory/work/receipt/nonce gates; freeze committed producer/catalog/config. Screen all24 using the existing current quote, retain exclusions. No SP1 until those gates pass.
- [x] Measurement: sequentially execute all qualified inputs using the frozen ELF; independently audit original reports, identity/output joins and owned-resource cleanup. Report literal ranking, full quote closure and matched increments without refitting.
- [ ] Retention: commit reusable framework/tests/docs in Raiko; preserve fixed inputs and audited data in a new immutable Atlas fixture. Update maintained Draft source PR records, leaving guest artifact alignment separate.
