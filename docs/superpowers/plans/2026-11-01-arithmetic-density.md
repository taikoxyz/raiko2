# Arithmetic density experiment implementation plan

> **For agentic workers:** Use subagent-driven-development for the producer and independent review/verification required by AGENTS.md.

**Goal:** Measure actual SP1 PGU / node-known zkGas for dense ordinary ADD and MUL with explicitly qualified 256-bit operands.

**Architecture:** Add an ordinary ProgramRequest catalog producer beside the existing density producers. Reuse the unchanged native constructor, current bbce9be0 quote, guarded SDK 6.3 estimator and frozen b76 ELF. Retain controls, raw reports and failures through the existing workload CLI.

**Tech Stack:** Python standard library in the existing virtual environment, EVM bytecode, existing Raiko workload tools.

**Spec:** The session's authorized workload-ratio campaign and the scope below; this is a bounded continuation, not a new pricing design.

## Scope and constraints

- Two operations, ADD and MUL; four operand classes each: zero, small, wide/carry and deterministic mixed full-width values.
- Counts 0, 4096 and 262144; 24 cases. Use a fixed 64-operation body and fixed-width count/operand immediates, including N0.
- Check one operation result before the loop, retain actual operands on stack, then repeat DUP/DUP/op/POP without per-operation guards. An independent decoder must verify operands, results, terminal, and exact complete opcode counts.
- One user CALL, empty calldata, one block, 15M user gas, evm-gas-only profile; no artificial witness padding, dependency/policy/limit changes or guest rebuild.
- Reuse existing binaries and sequential guards: 6GiB PSS, 600-second stage timeout, 192MiB output. Avoid new build caches on the nearly full disk.
- Whole-proposal actual/node is the primary ranking. Complete quote and same-profile count increments are separate diagnostics. No invented per-op actual or global bound.
- Every result must expose all three quantities: proposal estimate, per-block node-observable estimate (and its sum), and actual proposal SP1 PGU for the same input/program. Report underpricing of both estimates against actual PGU independently; do not hide startup controls or substitute paired increments for either comparison.
- Current work prioritizes block-mining-aware metering. Introducing proposal-aware allocation back to mining is a future design question, not an assumed implementation or a correction applied in this experiment.
- Keep Reth #260 and Raiko #284/#296 pending/do-not-merge. No merge or deployment action.

## Review focus

- Operand drift or reversed DUP indexing: independent decoded execution must see the same declared pair for every operation.
- A passing receipt with short execution: expected opcode maps and independently derived counts must reject it.
- Symmetric products hiding argument defects: include asymmetric mixed operands and an oracle of the actual consumed pair.
- Arithmetic overflow: compute results modulo 2^256; include all-limb carry and overflowing multiplication.
- Artifact/model/profile drift: freeze hashes before sampling and validate canonical input/public output joins afterward.

## Tasks

1. **Producer and focused coverage.** Add `bin/zkgas/bench/cases/arithmetic_density.py`, `bin/zkgas/bench/tests/test_arithmetic_density.py` and a usage section in `bin/zkgas/bench/README.md`. Start with failing tests; check typed bounds, fixed shape, independent operand/result/count simulation, mutation rejection and optional real native/quote qualification. Preserve existing producers byte-for-byte.
2. **Independent review and qualification.** A non-author reviews the complete diff. Independently decode all24 requests and run the frozen native/quote tools, checking successful full inclusion, no exceptional instructions, exact all-user routes and resource semantics. Retain qualification identities and negative controls.
3. **Actual measurements.** Root freezes catalog/tool/source identities and uses existing `workload.py` to screen, execute sequentially, verify and report. An independent verifier checks raw PGU normalization, exact inputs/outputs and joins. Investigate any scalable deficit before changing tariffs.
4. **Record bounded conclusions.** Add a concise research record with equations, identities, controls, observed ratios and limits. Preserve reusable producer/tests. A new immutable Atlas fixture may be prepared from verified evidence without rewriting previously published revisions.
