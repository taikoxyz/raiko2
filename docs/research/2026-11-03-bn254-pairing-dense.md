# Dense BN254 pairing inputs and bounded repetition

All 18 new execute-only observations completed once without failure or retry: six own-zero controls, six single-call points and six generated full points across identity/nonidentity and K=2/16/64 pairs. Ten literal node gaps remain: six controls and four single points, with maximum 153,329,861 PGU. No full point has a node gap, no input has a complete-proposal gap and none exceeds the frozen >2B actual-minus-node threshold within 30M charged gas. Maximum actual is 904,358,119 PGU. Full node/actual ratios are 3.950218–7.523373 and complete-proposal/actual ratios are 4.133810–7.701087. These conservative synthetic observations do not select a tariff change or certify ordinary-transaction usability.

The 18 inputs remain separate from the historical persistent 236 and transient 8 cohorts. Neither historical cohort is rerun, recounted or joined into a new union. See the [coverage inventory](zkgas-workload-coverage.md) for remaining interactions.

## Finite programs and gas domain

Two signed users call one common code context with empty persistent storage. Each input contains K finite, nonzero, canonically encoded pairing elements. Identity uses inverse G1 pairs `(jP,Q),(-jP,Q)` for j=1..K/2, so the product is 1. Nonidentity uses `(jP,Q)` for j=1..K, whose exponent sum K(K+1)/2 is nonzero modulo the group order, so the result is 0. Both boolean values are successful precompile outcomes: call status is 1 and all user receipts succeed. [EIP-197](https://eips.ethereum.org/EIPS/eip-197) supplies the group parameters and 192-byte element encoding; [EIP-1108](https://eips.ethereum.org/EIPS/eip-1108) supplies native gas `45000+34000K`. Independent checks validate canonical coordinates, G1 curve membership and G2 curve/subgroup membership without implementing a pairing benchmark.

The legacy runtime embeds exact operands after an unreachable data label; outer calldata is empty. CODECOPY loads the full operand buffer even for zero-call controls. Memory reaches `2+6K` words. The output sentinel 2 is reset and overwritten on each STATICCALL; guards check status 1, returndata length 32, exact expected result and final loop counter. Independent actual-byte execution verifies input ranges, copy, memory, control flow, requested gas, EIP-150 forwarding, returned gas and continuation. Ordinary opcode gas is `82+155R`; intrinsic 21000, CODECOPY `3+18K`, memory `3w+w*w//512`, and each call's 100 warm-access gas plus native gas are separate. There are no user persistent reads/writes or refunds.

| Kind | K | Full R/user | Charged gas/user | Signed envelope/user | Charged full block gas | Occupancy |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| identity | 2 | 131 | 14857568 | 14861663 | 29882491 | 99.608% |
| identity | 16 | 25 | 14753060 | 14764711 | 29673475 | 98.912% |
| identity | 64 | 6 | 13351216 | 13388771 | 26869787 | 89.566% |
| nonidentity | 2 | 131 | 14857568 | 14861663 | 29882491 | 99.608% |
| nonidentity | 16 | 25 | 14753060 | 14764711 | 29673475 | 98.912% |
| nonidentity | 64 | 6 | 13351216 | 13388771 | 26869787 | 89.566% |

Anchor succeeds and contributes 167355 gas. Every block has a 31M header. The bounded solver enforces <=30M charged block gas, <=15M/user fixture cap, the separate 16,777,216 EIP-7825 transaction cap, every ordered admission prefix, 24576-byte code cap and EIP-150 continuation. The signed envelope reserve `ceil(native_gas/63)+2301` is a fixture policy, not an SSTORE sentry or protocol requirement. Zero and single points keep their matching full signed envelope and nonempty operand preparation. Full+1 executes with sufficient interpreter gas before checking its own candidate constraints. Only the two K2 full points reach 99% occupancy; K16 reaches 98.912% and K64 89.566%. Maximal uniform R is not a global packing maximum.

## Literal same-proposal results

Actual is the entire proposal's SDK 6.3 `ExecutionReport.gas * 10 // 191`. Literal node PGU is the sum of per-block known-PGU ceilings, here one block. Complete-proposal PGU separately includes input/proposal terms. Positive gaps are `max(actual-estimate,0)`; all complete-proposal gaps are zero. Raw execution gas is not directly compared with PGU.

| Kind | K | R/user | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| identity | 2 | 0 | 209681 | 161220332 | 7890471 | 162851045 | 153329861 |
| identity | 2 | 1 | 436191 | 166650799 | 57919738 | 212880569 | 108731061 |
| identity | 2 | 131 | 29882491 | 872533694 | 6560143491 | 6715104322 | 0 |
| identity | 16 | 0 | 210725 | 161317186 | 9119554 | 164080128 | 152197632 |
| identity | 16 | 1 | 1389235 | 190835397 | 163252183 | 318213014 | 27583214 |
| identity | 16 | 25 | 29673475 | 904358119 | 3862143408 | 4017104239 | 0 |
| identity | 64 | 0 | 214727 | 161673466 | 13757367 | 168717942 | 147916099 |
| identity | 64 | 1 | 4657237 | 276101863 | 567171299 | 722132129 | 0 |
| identity | 64 | 6 | 26869787 | 844049576 | 3334180145 | 3489140976 | 0 |
| nonidentity | 2 | 0 | 209681 | 161220052 | 7890471 | 162851045 | 153329581 |
| nonidentity | 2 | 1 | 436191 | 166646124 | 57919738 | 212880569 | 108726386 |
| nonidentity | 2 | 131 | 29882491 | 871968407 | 6560143491 | 6715104322 | 0 |
| nonidentity | 16 | 0 | 210725 | 161316573 | 9119554 | 164080128 | 152197019 |
| nonidentity | 16 | 1 | 1389235 | 190830177 | 163252183 | 318213014 | 27577994 |
| nonidentity | 16 | 25 | 29673475 | 904249883 | 3862143408 | 4017104239 | 0 |
| nonidentity | 64 | 0 | 214727 | 161672943 | 13757367 | 168717942 | 147915576 |
| nonidentity | 64 | 1 | 4657237 | 276096818 | 567171299 | 722132129 | 0 |
| nonidentity | 64 | 6 | 26869787 | 844022698 | 3334180145 | 3489140976 | 0 |

## Whole-proposal contrasts and counter limits

The retained 27 diagnostics comprise six zero→single, six zero→full, six single→full and nine identity→nonidentity contrasts at matched K/level. Exact scaled node/proposal differences are retained before rounding. All 18 repetition increases are covered by both estimate increments. This does not erase the ten absolute node gaps. Across the 27 diagnostics, equality flags total 9 for repetitions, 27 for pair count, 9 for charged gas, 27 for signed envelope and 27 for authenticated selected persistent-storage support.

The nine signed nonidentity-minus-identity actual differences below have exact node and complete-proposal estimate differences of zero:

| K | Zero difference PGU | Single difference PGU | Full difference PGU |
| --- | ---: | ---: | ---: |
| 2 | -280 | -4675 | -565287 |
| 16 | -613 | -5220 | -108236 |
| 64 | -523 | -5045 | -26878 |

These are directional whole-program differences. Negative observations are not a two-sided coverage guarantee or an isolated boolean-result cost: operand point schedules and runtime hashes differ. A shared signed envelope does not make signed transaction bytes or code identical. Selected support equality compares exactly one authenticated owner mapped to an empty storage-node set, with SHA256(empty); it is not an empty owner dictionary or equality of account/code/global witness work.

Native STATICCALL counts, decoded target-8 operands, forwarding and returned gas establish the selected executions. Original quote APIs expose aggregate `operations_scaled`, ordinary dispatch and unique resources, without per-PC8 event/native-gas pricing details. Attempted dispatch agrees with user work plus the observed 12363 Anchor allowance; observed system persistent access/write selectors remain four, and the PC8 account observation appears only for nonzero calls. The Anchor allowances are observed fixture values, not independently derived Anchor charging proofs. Three counter negatives reject dropped wrapper dispatch, dropped PC8 account observation and invented persistent pair slots. Aggregate agreement does not prove per-event price charging.

The frozen model's PC8 source route uses the maximum of its 20M-PGU floor and native gas multiplied by approximately 122.328830 PGU/gas, with other structural and wrapper charges. This source-derived composition helps explain conservative quotes; it is not an observed per-PC charge allocation, isolated actual pairing PGU or a replacement coefficient.

## Qualification and retained evidence

See the [producer](../../bin/zkgas/bench/cases/bn254_pairing_dense.py), [tool README](../../bin/zkgas/bench/README.md), [binding specification](../superpowers/specs/2026-11-03-bn254-pairing-dense.md) and [implementation plan](../superpowers/plans/2026-11-03-bn254-pairing-dense.md). Final exact-source qualification binds `b49a05a5c2750a09461a96b24653722adf787add`. Existing helpers/oracles remain unchanged. The focused source suite recorded six passes and one optional native skip; three separate original native specimens passed in 8.376s. An interrupted broad old suite exited 130 and is not reported as passing.

Qualification independently checked all 18 canonical inputs, 36 signed users, operand/product intent, actual-byte wrapper, gas, code/account/whole-world/empty-root/witness support, receipts, counters and 18 fresh original real quotes. Native evidence comprises 16 fresh canonical invocations plus two retained original positive canonical smokes through explicit byte/source bridges. A retained valid reordered K2 specimen is natively accepted but rejected by ordered fixture intent; it is not another canonical measurement. Original per-invocation command/start/exit/elapsed/180s-bound evidence is preserved, without relabelling or backfilling old invocations. Native bounds are 180s; original quotes are bounded at 1800s with at most two workers.

The original 276-file qualification manifest remains frozen. All 18 actuals ran sequentially once in six zero, six single, then six full phases, reusing retained original quotes without a fresh quote invocation or implicit retry. Bounds were 6GiB sampled process-tree PSS and 600s, with 192MiB output/16MiB logs. Disk preflight required 576MiB: 256MiB free-space floor plus 320MiB retention reserve. Peak sampled PSS was 4544118784 bytes and maximum execution wall time 31.91030551120639s. No resource-limited or unmeasured case remains.

Independent final verification passed closure of 18 raw rows and 27 diagnostics, exact scaled arithmetic, original quote/native source bridges, phase order, resources and the separate unchanged persistent 236-input/transient 8-input contexts without rerunning measurements. A separate live host receipt confirms all 72 unique owned shared-memory names absent; file-only checks do not establish host absence. Post-actual closure verifies 163 tracked source files unchanged at the producer head before these documentation edits. The final independent manifest covers 296 files and does not replace the original qualification manifest.

| Identity | Value |
| --- | --- |
| Producer source | `b49a05a5c2750a09461a96b24653722adf787add` |
| Frozen model ID | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Retained ELF / SDK | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` / `6.3.0` |
| Native executable | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Quote executable | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| Frozen config | `e4f138a652b2a582634ca48d4b112e53f15198dfb1b9f065f9cd120db31d99b4` |
| Qualification verification | `c92b476940fca7325d305e856cd8e39c7b954d75cafd47db35f3fe44f48a9818` |
| Qualification index | `27dbecc788ac463196cd6a3d4977ddee7f6a94d02558b049ea71077dd4a9e6fc` |
| Retained-smoke bridge | `50c42ab9fdc6674ecaf01c5c89b2c3483e7b4abe36ecba71a43719cbdc54bc43` |
| Original qualification manifest | `532857ce957b0e6d9024e8c23fe7335482d877f3d490f46ec430df04788e108b` |
| Pre-actual policy | `c73562b0af49d4c2e9ce1542f4114227a66aeaa469f1bbddab3de1f053971288` |
| Actual analysis | `088eeabd4a6373c81ba82d648b79a16ca25b48633ae4c0dce69f2cfbb120518f` |
| Gas-budget report | `e1847c27aff3b6232f2fa0ea40b28bc11dc873fcd96a37298a078a3826d71176` |
| Counter retention audit | `bcc66c716b838020886e174d198f056c518c75309f3b527e8e1c51f50bc1ba29` |
| Pairing semantics audit | `14987d3b35a055dd196c193bf7bed5f9535c47ac79a435063fd60e23eb212635` |
| Independent raw final audit | `a1921fd5dfd119afb000fb2c86d36244ac2b397fcc12bad35263ab10f80a03c7` |
| Host shared-memory receipt | `de0cfd581c18cb6329616b596ce1c855bdd05cb2b3ff20df677eca3d83eafe75` |
| Post-actual source closure | `9db99564d8f13bd47af2599ef47bf5691b6c831dfbcbef17f051738c0c1767a7` |
| Independent focused final checks | `bf3718f3813b3ff9af4a9bcee5fde1788a1a987d155c9d6a8362246fb43abc90` |
| Independent actual closure | `dc9c8afcbec95d3a7a3b180b741d1d933ebe5e1285a239affe52b442f98f236d` |
| Independent final audit report | `27b0907eb8ae2606b0ad36fcd6c55545f0cc4662cc3df1e8a8cb4c504dd97efe` |
| Independent 296-file final manifest | `684de916a4eb8e926372429ec74be2ec76e8cda50a2491757ffdae6bc1f5bd5a` |

The verified local recovery archive is SHA256 `073de83d9703ee1001935fcbcf3447a65f57f3471a0d3970c32551b32123cb50`, 2,336,747 compressed bytes, with 576 regular members carrying 26,082,897 payload bytes, 162 restorable duplicate aliases and four external runtime artifacts identified by hash. All streamed member hashes passed verification without extraction. It retains canonical requests/inputs, original native/quote invocation provenance and bridges, raw actuals, sources/helpers/runtime identities, controls, resource receipts and independent audits. This is recovery evidence, not a standalone auditor runner or published Atlas fixture.

## Interpretation and remaining work

Prices remain unchanged. Preserve zero/single deficits and absolute/incremental distinctions; positive actual-minus-node >2B PGU at <=30M charged gas remains the frozen high priority. Smaller gaps remain explicit refinements. Low-call gaps resemble previously observed startup-scale deficits, but this panel does not causally assign their complete cost. Large conservative quotes can reduce useful admission capacity; this selected rare synthetic workload does not certify natural-history usability or select an admission limit.

Coverage improves only for selected finite valid 0x08 inputs, successful results 0/1 and bounded repetition of identical operand buffers. Arbitrary points, malformed/failure paths, unique inputs, cryptographic cache effects, heterogeneous composition/order and full packing remain open. Group mathematics, EIP semantics, source checks and execute-only success do not attest the experimental ELF build or exact production program. Representative natural proposals and exact production ELF/SDK parity require a separate promotion lane. No price fit, model change, Rust/guest/SDK/ELF/pin change, isolated pairing-PGU allocation, global/IID bound, proof generation, GPU throughput, production certification, merge, deployment or chain action follows. Current-head broad CI is not established by this panel; older SDK 6.8.1 runner/RISC0 failures are not a passing current-head result.
