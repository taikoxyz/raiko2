# Unreachable bytecode analysis: content and terminal padding

This prospective campaign tests a remaining gap in the preceding [multi-account code-loading campaign](2026-11-02-multiaccount-code-loading.md): equal-size code can have different analysis work even when its executed prefix is identical. Prices and production behavior are unchanged.

All 24 same-proposal execute-only SP1 observations completed. In all 12 near-full cases, both node and complete-proposal estimates cover actual PGU. All 12 N0 controls have node shortfalls, with a maximum of 154,091,263 PGU; complete-proposal estimates cover all 24. No shortfall exceeds the high-priority threshold of 2,000,000,000 positive actual-minus-node PGU. These are finite observations, not a global bound.

## Source hypothesis

The retained workload program declares source revision `4580faf7dc009714b310fd625b5810667019d683` and `revm-bytecode` 43.0.0. The selected path is `Bytecode::new_raw` -> `analyze_legacy`: it scans beyond STOP/RETURN, writes jump-table bits at decoded JUMPDEST positions, and skips PUSH immediate data. A final non-STOP opcode can trigger one whole-code copy; final ADD appends 1 zero byte, truncated PUSH32 appends 33. Analysis has at most L decoded iterations, L bit writes and one O(L) copy per unique code, with at most 33 padding bytes. Later account/call accesses clone the analyzed Arc-backed bytecode; they do not analyze it again.

The source binding is a declaration, not reproducible-build attestation or deployed parity. The previous raw four-byte salts also changed padding paths for some unique codes. This campaign places each salt in PUSH4 immediate data and controls the terminal separately, so that confound is not repeated.

## Finite matrix and preserved semantics

- 512 distinct predeployed target addresses with 512 unique code blobs; runtime lengths 4096 and 24576 bytes.
- Six unreachable tail profiles: STOP, dense JUMPDEST, alternating JUMPDEST/ADD, complete PUSH32 groups containing JUMPDEST immediate bytes, dense JUMPDEST ending ADD, and dense JUMPDEST ending truncated PUSH32.
- Every code starts with `3060005260206000f3`, returns its current execution ADDRESS, and halts at byte offset 9. Every suffix is `63 <index+1 as 4 bytes> <terminal>`, beginning at a decoded opcode boundary.
- Four signed top-level CALL users execute the same single-site DELEGATECALL loop as before, including return status/length/context and final-counter guards. Per-user calls 23234 at full and 0 for its own control. Each full block charges 29,999,647 EVM gas including 167,355 fixture Anchor gas. Header gas limit remains 31M; this is not a 30M-header admission claim.
- Every profile/length has its own N0 control with identical target code and transaction envelopes. Deployment/setup is outside the measured block; no free lifecycle claim. Inputs contain only accessed authenticated accounts/storage and hash-deduplicated fetched code. N0 omits unused target code.

An independent concrete interpreter, authenticated-root/code reconstruction, 96 signed user hashes, 12 own-N0 pairs, 48 byte-pattern mutants and 8 native/replay controls qualified all 24 inputs before actuals. The mutant controls include changes that preserve the executed prefix/outcome but violate intended tail content, so native success alone is insufficient qualification.

## Literal same-proposal results

The table uses new node-known PGU and full proposal PGU in their intended units. Raw Unzen score 5717892362 for full cases is retained separately in the quote artifacts and is never treated as PGU. Full proposal estimates are not substituted for the block node denominator.

| Runtime bytes | Tail | Charged block EVM gas | Literal node PGU | Full proposal PGU | Actual SP1 PGU | Node / actual |
| --- | --- | --- | --- | --- | --- | --- |
| 4096 | jumpdest | 29999647 | 1985585459 | 2160619674 | 1199574521 | 1.6552 |
| 4096 | jumpdest-copy | 29999647 | 1985585459 | 2160619674 | 1203791965 | 1.6494 |
| 4096 | jumpdest-pad32 | 29999647 | 1985585459 | 2160619674 | 1203859394 | 1.6493 |
| 4096 | mixed | 29999647 | 1985585459 | 2160619674 | 1191374492 | 1.6666 |
| 4096 | push32 | 29999647 | 1985585459 | 2160619674 | 1151927297 | 1.7237 |
| 4096 | stop | 29999647 | 1985585459 | 2160619674 | 1183184500 | 1.6782 |
| 24576 | jumpdest | 29999647 | 2656674099 | 2831708314 | 1775398206 | 1.4964 |
| 24576 | jumpdest-copy | 29999647 | 2656674099 | 2831708314 | 1805581274 | 1.4714 |
| 24576 | jumpdest-pad32 | 29999647 | 2656674099 | 2831708314 | 1805575220 | 1.4714 |
| 24576 | mixed | 29999647 | 2656674099 | 2831708314 | 1721826523 | 1.5429 |
| 24576 | push32 | 29999647 | 2656674099 | 2831708314 | 1484864596 | 1.7892 |
| 24576 | stop | 29999647 | 2656674099 | 2831708314 | 1673224573 | 1.5878 |

All controls and failed startup records remain in the data. The N0 node subtotal omits proposal startup, so small node shortfalls on controls are recorded as such, not subtracted away from the literal full-case result. Complete-proposal estimates include their proposal terms.

| Runtime bytes | Tail vs STOP | Actual full difference PGU | Own-N0 adjusted difference PGU | Node full difference scaled |
| --- | --- | --- | --- | --- |
| 24576 | jumpdest-copy | 132356701 | 132357023 | 0 |
| 24576 | jumpdest | 102173633 | 102173760 | 0 |
| 24576 | jumpdest-pad32 | 132350647 | 132351676 | 0 |
| 24576 | mixed | 48601950 | 48602310 | 0 |
| 24576 | push32 | -188359977 | -188359498 | 0 |
| 4096 | jumpdest-copy | 20607465 | 20606964 | 0 |
| 4096 | jumpdest | 16390021 | 16389152 | 0 |
| 4096 | jumpdest-pad32 | 20674894 | 20674608 | 0 |
| 4096 | mixed | 8189992 | 8189850 | 0 |
| 4096 | push32 | -31257203 | -31257716 | 0 |

Scaled values use 10^18 units per PGU. These contrasts measure whole inputs: code hashes, authenticated prestate and public outputs change with the tail. They do not isolate a per-byte coefficient or prove that every local content increment is covered. The literal whole-block node/actual ratio remains the primary metric.

## Reproduction and evidence boundary

The retained reusable producer and semantic/native tests are documented in [the tool README](../../bin/zkgas/bench/README.md). Use `cases/code_analysis_tail.py --output-dir <new-dir>` to regenerate requests, then compose the ordinary native fixture, quote and guarded workload commands. Canonical GuestInput remains the file interface; no vendor decoder, guest/SDK/ELF rebuild, host cap, limit or pricing change is required.

| Identity | Value |
| --- | --- |
| Producer source | `59dc5c41ea0ee97d25b6e368b1ed3fb1b1298f92` |
| Model | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Native | `1d555010c8440093e3da6325ea8c93e9ca5ba560bc466eb9249cf81c22188001` |
| Quote | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| ELF | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| Frozen campaign catalog | `cf3daff0df0b9030859d7e6c7de4a0c8e44f7c982c1ce899517cbf949a2dbbd5` |
| Resource-adjusted config | `2293616e8ff49240eb590997db9f071669c95cf084090af0e21fc791dd135243` |
| Resource-adjusted pre-actual freeze | `bc1e2c38ddf63a9a09974580b4c78375f037c0a9891996b72014df1a7ce63675` |
| Results | `8b76d986334760253385ba70642df5552b62434ea85123963f7dcc1528929497` |
| Independent actual audit | `181f69dfc5beb0fa9a87bd43ee80371355b31187acc63437dab699c65a038ed5` |

Initial 24 attempts were rejected before quote/SP1 by the startup disk reservation: 512MiB minimum plus 320MiB bounded retention exceeded available disk. Their null actuals, status receipts and original policy freeze are retained. An explicit resource-only contract used the established 256MiB minimum plus the same 320MiB retention reservation, with a fresh output root; all source/input/tool/model identities were unchanged. There were no hidden retries or discarded completed actuals.

Actuals use SDK 6.3.0 and `ExecutionReport.gas * 10 // 191`, once per adjusted-contract sample. The observed maximum process-tree PSS was 4670351360 bytes, bounded by 6GiB and 600s per sample. This is execute-only estimator PGU, not a generated proof, GPU throughput or production-SDK parity assertion. A read-only host namespace check confirmed all 96 owned shared-memory names absent after completion; the independent audit verified the name and receipt joins. Unrelated objects were untouched.

## Disposition and next point

No pricing coefficient is changed in this campaign. Retain the full/own-N0 observations and content-sensitive code-analysis boundary as regression probes. The 512-account fixture cap and two lengths do not establish a bound over every possible 30M block. Do not infer global closure from this panel.

The next distinct source-supported target is committed mutation of many storage leaves: nonzero updates, deletion and mixed insertion/deletion, first in one nonempty storage trie and then across accounts. Existing read growth and repeated same-slot dirty/revert tests do not cover that final-root topology. Node pricing has unique account/slot/write counts but no MPT depth/materialization/split/merge selector; full proposal pricing has MPT selectors. Guest state-root calculation runs once per block after execution, so caught REVERT does not redo trie split/merge for every temporary SSTORE. Measure both node and proposal shortfalls against the same SP1 actual and apply the 30M/>2B severity rule.
