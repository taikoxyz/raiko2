# Stack density experiment implementation plan

> **For agentic workers:** Use subagent-driven-development with independent source review and native/actual verification before interpreting results.

**Goal:** Test whether repeatable arithmetic-composition deficits also exist in basic stack operations, without assigning the observed composite cost to ADD/MUL.

**Architecture:** Add a small ProgramRequest producer reusing the existing assembler, native constructor, quote and guarded sampler. Keep all earlier24 arithmetic and12 paired-control inputs frozen. No price, guest or dependency changes.

**Tech Stack:** Existing Python standard-library tools, EVM bytecode, SDK6.3/frozen b76 program.

**Spec:** The authorized underpricing campaign and the arithmetic-density result record; all results retain full proposal estimate, node-per-block/sum estimate and actual proposal SP1 PGU, with independent underpricing flags.

## Scope

- Eight balanced body patterns: PUSH0/POP; DUP1/POP, DUP3/POP, DUP8/POP, DUP16/POP; SWAP1/SWAP1, SWAP8/SWAP8, SWAP16/SWAP16.
- For each pattern, counts0/4096/262144 pairs:24 cases. Fixed64-pair body, fixed-width count immediates and same code length across the count sweep, including unused N0 body.
- Seed17 distinct deterministic full-width stack words. Keep the batch counter in memory word0, outside the stack during each body; otherwise DUP1 would accidentally measure the changing counter. Check all17 final words with semantic guards.
- Independently decode every actual accessed value/order/index, stack preservation, final results, exact counts and counter/memory behavior. Native observer counts must match; failed/short/OOG mutations must not masquerade as completed workloads.
- One CALL user, empty calldata, one block,15M gas, honest minimal witness. Reuse current artifacts and sequential6GiB/600second guards; no Rust/guest build or installed dependencies.
- Pairs and loop increments are composite observations. Preserve N0, full ratios and both underpricing flags; do not claim an identifiable isolated POP or DUP tariff from balanced pairs alone.

## Tasks

1. Author `bin/zkgas/bench/cases/stack_density.py`, focused independent-interpreter tests, and a concise bench README section. Begin with failing tests, preserve existing producers, validate typed options/bounds and ensure no overwrite.
2. Complete independent adversarial source review and all24 real native/quote qualifications, including semantic/short-loop/OOG negatives. Freeze source/catalog/tool/input identities.
3. Root runs existing workload CLI sequentially; independently audit raw actual PGU normalization, canonical input/public outputs, three estimate/actual quantities, all controls and resource cleanup.
4. Combine results with earlier arithmetic and paired controls before choosing any pricing refinement. If isolated coefficients remain unidentifiable, state that limitation and use a separately justified conservative policy rather than fabricate a fit.
