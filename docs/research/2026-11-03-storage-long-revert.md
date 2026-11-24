# Few long root-REVERT transactions

All 16 new execute-only observations completed: eight own-N0 controls and eight generated full-body points. Every control has a literal node gap; no full point has a node gap, no input has a complete-proposal gap, and none exceeds the frozen >2B actual-minus-node priority threshold at <=30M charged gas. The maximum node gap is 143,814,531 PGU in update-revert/U2/S16384/N0; the largest actual is 456,505,480 PGU in restore-revert/U2/S16384/N2725. One earlier setup failure never launched SP1 and remains unmeasured, separately retained from the 16 successful executions.

The [root-journal panel](2026-11-03-storage-root-journal.md) used 64 failures and a 320M-PGU existing REVERT allowance. This panel tests fewer longer failures, receiving only 10M/20M. Full coverage persists in these eight finite compositions, while startup/control deficits remain. This is whole-program evidence under unchanged pricing, not an isolated REVERT/storage cost or a global composition bound. See the [coverage inventory](zkgas-workload-coverage.md) for the evidence boundary and remaining interactions.

## Finite programs and gas domain

The panel crosses `update-revert`/`restore-revert`, U=2/4 ordinary signed top-level CALL users, S=8192/16384 aggregate initialized entries, and own-N0/generated-full levels: 16 distinct inputs in eight pairs. Account count equals U; each user visits a different predeployed contract exactly once, with empty calldata and no CALL wrapper. This changes packing from the previous 64-user root-journal panel. The existing 5M-PGU REVERT route contributes 10M or 20M PGU per proposal instead of 320M. No route or price changes.

For account i, `B_i=(i+1)*65536` and `K=S/U`. Raw phase slot 0 contains `B_i+1`; raw slots `B_i+1..B_i+K-1` contain 1; all others are absent. Each own-N0/full pair retains identical complete initial maps and its full point's signed envelope. Each body's first access to a key is cold. The unchanged guarded journal body checks 1, writes 2 and reads 2 into its checksum; restore mode also writes and checks 1. It verifies N, checksum 2N for update or 3N for restore, and attempted phase+N before zero-range root REVERT. Distinct attempted `(account,rawslot)` pairs total `U*(N+1)`, including the phase slot.

All user receipts legitimately fail and Anchor succeeds. Complete contract storage maps and roots restore, while sender nonces, fees and Anchor change whole-world state. Restore earns a preterminal refund counter of `2800*N` per user; rollback discards it. Effective and applied refunds are zero, so gross equals charged gas. Attempted reads/writes remain work even though the final contract maps match their originals.

| Mode | U/accounts | Full N/user | Gross/charged gas/user | Refund before terminal/user | Signed envelope/user | Charged block gas |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| update-revert | 2 | 2844 | 14911773 | 0 | 14914074 | 29990901 |
| update-revert | 4 | 1419 | 7453323 | 0 | 7455624 | 29980647 |
| restore-revert | 2 | 2725 | 14915677 | 7630000 | 14917978 | 29998709 |
| restore-revert | 4 | 1360 | 7457317 | 3808000 | 7459618 | 29996623 |

The values hold at both S choices. Own-N0 charged block gas is 214303 for U2 and 261251 for U4. Every block has a 31M header; charged block gas includes Anchor 167355. The full solver checks <=30M charged gas, each candidate's own gross+2301 envelope, <=15M/user and EIP-7825 caps, every ordered admission prefix, initialized ranges and <=8192 distinct attempted targets. Independent sufficient-envelope execution of N+1 precedes checking its own constraints; all eight N+1 candidates exceed the charged block budget. The selected maximum is finite and program-specific. Full points occupy over 99.9% of the charged budget; these are not 30M-header observations.

## Literal same-proposal results

Each row identifies `storage-long-revert-{mode}-u{U}-s{S}-n{N}`. All actuals are new; no old actual is reused. Positive gaps are `max(actual-estimate,0)` on the indicated surface. Full node/actual ratios are 1.0686–1.2105; complete-proposal/actual ratios are 1.8890–2.0066.

### Full-body points

| Mode | U | S | N/user | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive full gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| update-revert | 2 | 8192 | 2844 | 29990901 | 398616012 | 469491155 | 786105354 | 0 | 0 |
| update-revert | 2 | 16384 | 2844 | 29990901 | 439331622 | 469491155 | 847932962 | 0 | 0 |
| update-revert | 4 | 8192 | 1419 | 29980647 | 395813131 | 479125959 | 794227550 | 0 | 0 |
| update-revert | 4 | 16384 | 1419 | 29980647 | 436297801 | 479125959 | 856192758 | 0 | 0 |
| restore-revert | 2 | 8192 | 2725 | 29998709 | 418194529 | 489698166 | 802287461 | 0 | 0 |
| restore-revert | 2 | 16384 | 2725 | 29998709 | 456505480 | 489698166 | 862333941 | 0 | 0 |
| restore-revert | 4 | 8192 | 1360 | 29996623 | 416746709 | 499420019 | 811078274 | 0 | 0 |
| restore-revert | 4 | 16384 | 1360 | 29996623 | 454328609 | 499420019 | 871713890 | 0 | 0 |

### Preserved own-N0 controls

| Mode | U | S | N/user | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive full gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| update-revert | 2 | 8192 | 0 | 214303 | 161580819 | 17808590 | 173412725 | 143772229 | 0 |
| update-revert | 2 | 16384 | 0 | 214303 | 161623121 | 17808590 | 173495981 | 143814531 | 0 |
| update-revert | 4 | 8192 | 0 | 261251 | 162307115 | 28328630 | 184962860 | 133978485 | 0 |
| update-revert | 4 | 16384 | 0 | 261251 | 162335963 | 28328630 | 185034860 | 134007333 | 0 |
| restore-revert | 2 | 8192 | 0 | 214303 | 161582890 | 17810318 | 173414453 | 143772572 | 0 |
| restore-revert | 2 | 16384 | 0 | 214303 | 161622792 | 17810318 | 173497709 | 143812474 | 0 |
| restore-revert | 4 | 8192 | 0 | 261251 | 162298680 | 28330358 | 184964588 | 133968322 | 0 |
| restore-revert | 4 | 16384 | 0 | 261251 | 162331156 | 28330358 | 185036588 | 134000798 | 0 |

## Matched diagnostics

The retained analysis contains 32 diagnostics: eight full-minus-own-N0 increments, eight S16384−S8192 state contrasts, eight U4−U2 packing contrasts and eight restore−update mode contrasts. Each contrast retains sample IDs, scaled quote differences before rounding, actual differences and `same_body_n`/`same_charged_gas` flags. No per-operation actual allocation or N0 linear extrapolation follows.

All eight own-N0 actual increments are positive, 233,506,016–294,882,688 PGU, and covered by their matched node and complete-proposal increments. All eight state contrasts increase actual, by 28,848–40,715,610 PGU, with identical literal node estimates; complete-proposal differences cover each. The four full state increases are 40.716M/40.485M for update U2/U4 and 38.311M/37.582M for restore U2/U4.

U4−U2 increases control actual by 0.708M–0.726M PGU, but lowers full actual by 1.448M–3.034M while both estimates rise. Each full packing comparison changes N, gas, envelope and account/support structure. Full restore−update actual differences are 17.174M–20.934M PGU; the U4/S8192 difference of 20,933,578 exceeds the scaled node difference of approximately 20,294,060, and all four full mode differences exceed the corresponding complete-proposal differences. These incremental ranking limits remain visible despite all full absolute quotes covering actual. Control-mode actual differences range from −8435 to +2071 PGU; the U2/S8192 increase of 2071 exceeds both scaled estimate differences of 1728 PGU by 343. These are observed whole-program differences, not isolated restoration costs.

## Attempted work retention and observability

The independent private-VM/native event audit passed all 16 inputs. Exposed node/prove aggregate `normal_dispatch_count` agrees with the independently traced user instructions plus a fixed observed Anchor allowance of 12363; unique storage access/write pairs agree with `U*(N+1)+4`. The four system slots and dispatch allowance are observed fixture allowances, not independently derived Anchor opcode counts. Attempted user storage/dispatch survives root rollback in these exposed surfaces. Unique pair counts are distinct from SSTORE occurrences; neither proves per-opcode occurrence charging. A collection drop on REVERT would be an accounting-correctness finding separate from a low tariff.

## Interpretation and remaining work

Actual PGU is the entire proposal's SDK6.3 `ExecutionReport.gas * 10 // 191`. The primary node denominator is the literal sum of block known-PGU ceilings; each proposal here has one block. Complete-proposal estimates retain their separate input/proposal terms. Raw Unzen scalars and incremental differences do not replace either denominator. Own-N0 retains the users, phase body and full signed envelope; it is not an empty proposal.

Retain all controls and deficits under the frozen rule: positive actual-minus-node >2B PGU at <=30M charged gas is high priority. Smaller misses remain explicit refinements. Covered increments cannot erase absolute control gaps. Mode and U2-to-U4 full contrasts change N, gas, signed envelopes and account/support structure; they do not measure an isolated restoration, transaction, account or REVERT cost. Background-state contrasts hold the body and charged gas fixed but still compare complete programs, not per-trie costs.

Keep prices unchanged. Large conservative estimates can reduce ordinary admission capacity even where a synthetic actual is covered. Exact production ELF/SDK parity and representative historical usability remain separate promotion work. These single-visit cold-many-key programs do not close many-key work with high warm-toggle counts, nested/caught rollback, deliberately skewed trie paths/depth or internal lookup distributions, arbitrary mutation topology, heterogeneous order or cross-family interactions. No fit on opened samples, global bound, proof generation, GPU proving throughput or production certification follows.

## Tools, qualification and retained evidence

See the [tool README](../../bin/zkgas/bench/README.md#few-long-root-revert-calls), [producer](../../bin/zkgas/bench/cases/storage_long_revert.py) and [binding specification](../superpowers/specs/2026-11-03-long-root-revert.md). The producer reuses the original root-journal runtime and gas accounting directly, and builds the request through the existing top-level CALL schema. It adds no alternate runtime, gas implementation or price path. Host, guest, GuestInput, dependencies, pins, SDK, ELF and model remain unchanged.

Retained source validation includes eight focused tests (seven executed and one optional-native skip), a separate one-test native lane with four specimens, eleven old-journal tests (one optional-native skip), six mutation tests (one optional-native skip), and a late two-test strict-validation lane. The native lane includes full specimens for both modes and matching-value/checksum mutants that native accepts but the literal semantic oracle rejects. These are distinct retained runs, not newly reproduced measurements or a combined suite count. Independent source review passed the exact Task1 increment.

Independent qualification passed all 16 canonical inputs, 48 signed users, original real quotes, complete original/final maps and authenticated roots, expected included failed receipts and canonical witness support before the resource freeze. Original quote bytes and command/start/outcome provenance are retained. Retained reuse must bind the exact input/key/model/profile/executable/public output; it is not a fresh quote invocation. The ELF's declared source revision is `4580faf7dc009714b310fd625b5810667019d683`; that declaration is not a build attestation or production-program parity evidence. No Rust or guest build occurred.

The frozen config and policy retain their original bytes. A first setup attempt failed before SP1 launched: the outer 256MiB retention reserve did not meet the workload runner's existing 576MiB free-space guard (`256MiB + 5*64MiB` maximum input staging). A retained correction sidecar supersedes the outer reserve with 320MiB, separate from the 256MiB minimum-free floor. The first sample was explicitly resumed with `--retry-failed`; this is not an implicit retry or a second SP1 execution. The failed setup remains evidence and receives no PGU. No input, price or execution cap changed.

Across the retained 17 attempt folders, the one setup failure is separate from 16 sequential successful SP1 executions; no successful same-input SP1 execution was repeated. Each execution used 6GiB sampled process-tree PSS and 600s bounds, with 192MiB output and 16MiB logs; qualification retained 180s native and 1800s quote bounds. Peak sampled PSS was 2725233664 bytes. The independent final audit passed all 16 raw actuals, all 32 diagnostics, all 48 signed users, the five-panel 132-input/56-full join, and the resource correction. It rejoined raw SDK conversion, canonical input/public-output/artifact identities, original quote provenance, literal node sums/full totals, gas/refunds/receipts, complete maps/roots and guards. A separate host-context receipt verified all 64 exact owned shared-memory names absent; the raw audit alone did not establish live host absence. No measurement was rerun by the independent final auditor.

The current five-panel join is a separately verified finite storage subset: 132 distinct canonical inputs, including 56 generated full points, 10 full node gaps, maximum full gap 58,675,269 PGU, maximum across all inputs 164,072,990 PGU, no >2B/30M case and no complete-proposal gap. It excludes reused references and preserves the historical four-panel 116/48 snapshot. Neither cohort is a total research-test count, IID confidence statement or coverage percentage.

| Identity | Value |
| --- | --- |
| Producer source | `7934d2c5a9187a206ef3de3634dff007472a4642` |
| Model | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Retained ELF / SDK | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` / `6.3.0` |
| Native executable | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Frozen config | `34a79ea2a02f61155526b1d4fa4f7c656ab3e87ce5cd9c8445531e5bfdb9f5c5` |
| Original pre-actual policy | `369114f15072f6fab4aff8279a82ee5c37a41a38e8ccec591bde05a7f185be28` |
| Resource-only resume correction | `56004e3fbf2016d17c5e7adf9dfeaaae20f27d6c75344bda0d2e1bc75d083519` |
| Input/quote qualification | `01f3c5b88af9d9f8644c4e367282b14a000d736c424f727c420aa7a1718543f0` |
| Counter retention audit | `6739b0cde8604b86cace56ad16fc02ff3cd511c4bfcbe6864e945146148474bd` |
| Actual analysis | `c6d01d719c07dc6096b7450e6efe02d3f4b9be1a6007890889750d8ffe224ed7` |
| Gas-budget report | `61503b19e88ea562f41b5eb540c2482df725c00201fb27747af01ab5cab4f42e` |
| Independent raw final audit | `664ffba80b86948eb9cf7a636d00a32e554be245f30b88721e673ba41641f9e5` |
| Independent diagnostic/cohort checks | `556de60eee426fc606da72c5a42d21f84fd8de034f7c52575336bda5d131f11d` |
| Host shared-memory audit | `f60d3874489eb80995fcf0a957c6fa6a3eac908c467cb973d567b913c64994da` |
| Five-panel storage-cohort summary | `f224ff5fbc6380cf39185ba4e4bb449e52f0d81b5677d36daeb02335bdaa0508` |
| Independent 243-file evidence manifest | `2f08142e6a8fa2cf49fd64f89b8ec282efffadb9661fdca89fa91f8415cb43fb` |

Raw requests/canonical inputs, original quotes and invocation provenance, native diagnostics, all 17 attempt folders including the setup failure, controls, source/helper/artifact identities, resource correction and final audit receipts are retained. The verified local recovery archive is SHA256 `e57a2264eaa9b55db6d70f159e5a87725528c82a6c19d66152c6c24d579aa6eb`, 26468423 compressed bytes, with 504 regular members carrying 63265672 payload bytes, 96 restorable duplicate aliases and four external runtime artifacts identified by hash. Streamed member hashes were verified without extraction. Private helper versions/dependencies and cache-cleanup receipts are retained. This is recovery evidence, not a standalone auditor runner or published Atlas fixture. No merge, deployment, repricing or on-chain action is part of this panel.
