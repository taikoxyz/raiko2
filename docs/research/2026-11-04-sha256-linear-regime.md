# SHA-256 floor crossover and linear-length workloads

All 18 new execute-only observations completed once without failure or retry: six own-zero controls, six singles and six maximal-uniform-full points across zero/a5 tails and 1269664/1269665/2097152-byte inputs. Twelve literal node gaps remain (six controls and six singles), with maximum 134,146,506 PGU. No full point has a node gap, no input has a complete-proposal gap and none crosses the frozen >2B actual-minus-node threshold within 30M charged gas. Maximum actual is 870,463,736 PGU. Full node/actual ratios are 1.17334601631–1.41245084998; complete-proposal/actual ratios are 1.47981859111–1.59051101601. Prices remain unchanged.

The persistent-storage 236, transient-storage 8, pairing 18, [default SHA-256 18](2026-11-04-sha256-bulk.md) and new linear-regime 18 remain separate read-only cohorts. Historical measurements are neither rerun nor counted in a new union. See the [coverage inventory](zkgas-workload-coverage.md).

## Exact inputs and finite gas domain

The existing [producer](../../bin/zkgas/bench/cases/sha256_bulk.py) selects this distinct `unzen-sha256-linear-regime-20261104` corpus through the functional `--linear-regime` CLI flag and strict keyword-only Boolean selector. It uses one shared producer and native guard. Default mode still emits the original 18 requests at 64/4096/1048576 bytes: canonical request bytes and complete runtime/gas/receipt metadata remain byte-identical to the frozen default corpus. New source/catalog identities do not relabel old measurements.

Two signed users call distinct authenticated code accounts with empty persistent storage and fixed-width owner tags 0/1. Call i hashes `BE256((tag<<128)|i) || pattern_byte*(L-32)`, i=0..R-1; every input is unique across both users within a proposal. Empty outer calldata, no storage instructions, no refunds and no creation bound this selected fixture. Input seeding and logarithmic MCOPY doubling occur once, including at R0. Counter/XOR occupy words 0/32; input begins at 64 and output at 64+L. Sentinel 2, status 1, returndata length 32, exact final counter/XOR and STOP are guarded. XOR is noninjective: independent byte interpretation checks every ordered input/digest (232 calls across the 18 programs).

Memory allocation is `3+ceil(L/32)` words. The 1269665-byte output is unaligned; exact final MCOPY length, rounded copy gas, unaligned MSTORE/MLOAD/STATICCALL writes, nonoverlap and untouched trailing allocation padding are checked independently. Native gas is `60+12*ceil(L/32)`, warm STATICCALL access is 100 separately, memory expansion is `3*w+w*w//512`, MCOPY is `3+3*ceil(n/32)` and intrinsic gas is 21000. EIP-150 caps requested GAS and unused child gas returns.

| Tail | Bytes L | Full R/user | Charged gas/user | Signed gas envelope/user | Charged block gas | Occupancy |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| zero | 1269664 | 24 | 14769379 | 14779239 | 29706113 | 99.020377% |
| zero | 1269665 | 24 | 14769828 | 14779688 | 29707011 | 99.023370% |
| zero | 2097152 | 7 | 14311215 | 14326000 | 28789785 | 95.965950% |
| a5 | 1269664 | 24 | 14769379 | 14779239 | 29706113 | 99.020377% |
| a5 | 1269665 | 24 | 14769828 | 14779688 | 29707011 | 99.023370% |
| a5 | 2097152 | 7 | 14311215 | 14326000 | 28789785 | 95.965950% |

Six full selectors yield only four near-full observations (>=29.7M charged gas). The two 2MiB full points have R7 and 28,789,785 charged gas; they are not near-full. Anchor contributes 167355 gas. The bounded solver enforces <=30M charged block gas, <=15M/user fixture cap, the separate 16,777,216 EIP-7825 cap, a 31M header, every ordered admission prefix, 24576-byte code cap and EIP-150 continuation. Zero/single controls retain the matching full signed gas envelope and memory/copy preparation. Full+1 executes with sufficient interpreter gas before checking its candidate constraints. Maximal uniform R does not establish a global packing maximum.

## Literal same-proposal results

Actual is entire-proposal SDK 6.3 `ExecutionReport.gas * 10 // 191`. Literal node PGU sums known-PGU per-block ceilings (one block here). Complete-proposal PGU separately includes proposal/input terms; raw execution gas is not directly compared with PGU. Positive gaps are `max(actual-estimate,0)`; all complete-proposal gaps are zero.

| Tail | Bytes L | R/user | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| zero | 1269664 | 0 | 6836513 | 163133403 | 28989149 | 183982235 | 134144254 |
| zero | 1269664 | 1 | 7789413 | 192376709 | 79020618 | 234014217 | 113356091 |
| zero | 1269664 | 24 | 29706113 | 870459719 | 1229464695 | 1384458294 | 0 |
| zero | 1269665 | 0 | 6836835 | 163132326 | 28990104 | 183983191 | 134142222 |
| zero | 1269665 | 1 | 7789759 | 192375857 | 79022038 | 234015636 | 113353819 |
| zero | 1269665 | 24 | 29707011 | 870456332 | 1229476786 | 1384470385 | 0 |
| zero | 2097152 | 0 | 17775173 | 164331909 | 60796354 | 215789441 | 103535555 |
| zero | 2097152 | 1 | 19348689 | 212610834 | 136893152 | 291886751 | 75717682 |
| zero | 2097152 | 7 | 28789785 | 505732443 | 593400966 | 748394565 | 0 |
| a5 | 1269664 | 0 | 6836513 | 163135655 | 28989149 | 183982235 | 134146506 |
| a5 | 1269664 | 1 | 7789413 | 192376555 | 79020618 | 234014217 | 113355937 |
| a5 | 1269664 | 24 | 29706113 | 870452231 | 1229464695 | 1384458294 | 0 |
| a5 | 1269665 | 0 | 6836835 | 163134298 | 28990104 | 183983191 | 134144194 |
| a5 | 1269665 | 1 | 7789759 | 192375683 | 79022038 | 234015636 | 113353645 |
| a5 | 1269665 | 24 | 29707011 | 870463736 | 1229476786 | 1384470385 | 0 |
| a5 | 2097152 | 0 | 17775173 | 164331768 | 60796354 | 215789441 | 103535414 |
| a5 | 2097152 | 1 | 19348689 | 212610103 | 136893152 | 291886751 | 75716951 |
| a5 | 2097152 | 7 | 28789785 | 505733993 | 593400966 | 748394565 | 0 |

## Whole-proposal contrasts and quote interpretation

The 33 retained derived diagnostics comprise six zero→single, six zero→full, six single→full, nine a5-minus-zero and six direct 1269664→1269665 length-boundary contrasts. They add no requests or measurements. All 18 own-R increments and all six direct boundary increments are covered by both exact scaled estimate increments; absolute node control/single gaps remain. Every comparison below retains its signed actual difference and exact rational estimate difference. `N/10^18` means the exact signed scaled numerator divided by the frozen scale, before ceiling; comparison uses these rationals, not rounded literal estimate differences.

Flags are `R/L/G/E/S`: equal repetitions / hash-input length / charged block gas / complete signed user-transaction envelopes / authenticated selected persistent-storage support. Counts are 15/27/9/27/33. E compares full signed user transactions, not just gas limits: the original 27 pairs have identical signed transaction bytes, while their runtime/code and complete GuestInput bytes differ. All six boundary pairs have different lengths, gas and envelopes. S compares two authenticated owners, each with an empty storage-node set and SHA256(empty); account/code/global witness work remains separate.

| Contrast (to minus from) | Actual Δ PGU | Node Δ PGU (exact) | Proposal Δ PGU (exact) | R/L/G/E/S |
| --- | ---: | ---: | ---: | --- |
| zero-to-single: zero/L1269664/R0 → zero/L1269664/R1 | 29243306 | 50031469677210483424130816/10^18 | 50031981677210483424130816/10^18 | F/T/F/T/T |
| zero-to-full: zero/L1269664/R0 → zero/L1269664/R24 | 707326316 | 1200475546253051602179139584/10^18 | 1200476058253051602179139584/10^18 | F/T/F/T/T |
| single-to-full: zero/L1269664/R1 → zero/L1269664/R24 | 678083010 | 1150444076575841118755008768/10^18 | 1150444076575841118755008768/10^18 | F/T/F/T/T |
| zero-to-single: zero/L1269665/R0 → zero/L1269665/R1 | 29243531 | 50031933677210483424130816/10^18 | 50032445677210483424130816/10^18 | F/T/F/T/T |
| zero-to-full: zero/L1269665/R0 → zero/L1269665/R24 | 707324006 | 1200486682253051602179139584/10^18 | 1200487194253051602179139584/10^18 | F/T/F/T/T |
| single-to-full: zero/L1269665/R1 → zero/L1269665/R24 | 678080475 | 1150454748575841118755008768/10^18 | 1150454748575841118755008768/10^18 | F/T/F/T/T |
| zero-to-single: zero/L2097152/R0 → zero/L2097152/R1 | 48278925 | 76096797677210483424130816/10^18 | 76097309677210483424130816/10^18 | F/T/F/T/T |
| zero-to-full: zero/L2097152/R0 → zero/L2097152/R7 | 341400534 | 532604611740473383968915712/10^18 | 532605123740473383968915712/10^18 | F/T/F/T/T |
| single-to-full: zero/L2097152/R1 → zero/L2097152/R7 | 293121609 | 456507814063262900544784896/10^18 | 456507814063262900544784896/10^18 | F/T/F/T/T |
| zero-to-single: a5/L1269664/R0 → a5/L1269664/R1 | 29240900 | 50031469677210483424130816/10^18 | 50031981677210483424130816/10^18 | F/T/F/T/T |
| zero-to-full: a5/L1269664/R0 → a5/L1269664/R24 | 707316576 | 1200475546253051602179139584/10^18 | 1200476058253051602179139584/10^18 | F/T/F/T/T |
| single-to-full: a5/L1269664/R1 → a5/L1269664/R24 | 678075676 | 1150444076575841118755008768/10^18 | 1150444076575841118755008768/10^18 | F/T/F/T/T |
| zero-to-single: a5/L1269665/R0 → a5/L1269665/R1 | 29241385 | 50031933677210483424130816/10^18 | 50032445677210483424130816/10^18 | F/T/F/T/T |
| zero-to-full: a5/L1269665/R0 → a5/L1269665/R24 | 707329438 | 1200486682253051602179139584/10^18 | 1200487194253051602179139584/10^18 | F/T/F/T/T |
| single-to-full: a5/L1269665/R1 → a5/L1269665/R24 | 678088053 | 1150454748575841118755008768/10^18 | 1150454748575841118755008768/10^18 | F/T/F/T/T |
| zero-to-single: a5/L2097152/R0 → a5/L2097152/R1 | 48278335 | 76096797677210483424130816/10^18 | 76097309677210483424130816/10^18 | F/T/F/T/T |
| zero-to-full: a5/L2097152/R0 → a5/L2097152/R7 | 341402225 | 532604611740473383968915712/10^18 | 532605123740473383968915712/10^18 | F/T/F/T/T |
| single-to-full: a5/L2097152/R1 → a5/L2097152/R7 | 293123890 | 456507814063262900544784896/10^18 | 456507814063262900544784896/10^18 | F/T/F/T/T |
| a5-minus-zero: zero/L1269664/R0 → a5/L1269664/R0 | 2252 | 0/10^18 | 0/10^18 | T/T/T/T/T |
| a5-minus-zero: zero/L1269664/R1 → a5/L1269664/R1 | -154 | 0/10^18 | 0/10^18 | T/T/T/T/T |
| a5-minus-zero: zero/L1269664/R24 → a5/L1269664/R24 | -7488 | 0/10^18 | 0/10^18 | T/T/T/T/T |
| a5-minus-zero: zero/L1269665/R0 → a5/L1269665/R0 | 1972 | 0/10^18 | 0/10^18 | T/T/T/T/T |
| a5-minus-zero: zero/L1269665/R1 → a5/L1269665/R1 | -174 | 0/10^18 | 0/10^18 | T/T/T/T/T |
| a5-minus-zero: zero/L1269665/R24 → a5/L1269665/R24 | 7404 | 0/10^18 | 0/10^18 | T/T/T/T/T |
| a5-minus-zero: zero/L2097152/R0 → a5/L2097152/R0 | -141 | 0/10^18 | 0/10^18 | T/T/T/T/T |
| a5-minus-zero: zero/L2097152/R1 → a5/L2097152/R1 | -731 | 0/10^18 | 0/10^18 | T/T/T/T/T |
| a5-minus-zero: zero/L2097152/R7 → a5/L2097152/R7 | 1550 | 0/10^18 | 0/10^18 | T/T/T/T/T |
| crossover-length-boundary: zero/L1269664/R0 → zero/L1269665/R0 | -1077 | 955332613507934748296/10^18 | 955332613507934748296/10^18 | T/F/F/F/T |
| crossover-length-boundary: zero/L1269664/R1 → zero/L1269665/R1 | -852 | 1419332613507934748296/10^18 | 1419332613507934748296/10^18 | T/F/F/F/T |
| crossover-length-boundary: zero/L1269664/R24 → zero/L1269665/R24 | -3387 | 12091332613507934748296/10^18 | 12091332613507934748296/10^18 | T/F/F/F/T |
| crossover-length-boundary: a5/L1269664/R0 → a5/L1269665/R0 | -1357 | 955332613507934748296/10^18 | 955332613507934748296/10^18 | T/F/F/F/T |
| crossover-length-boundary: a5/L1269664/R1 → a5/L1269665/R1 | -872 | 1419332613507934748296/10^18 | 1419332613507934748296/10^18 | T/F/F/F/T |
| crossover-length-boundary: a5/L1269664/R24 → a5/L1269665/R24 | 11505 | 12091332613507934748296/10^18 | 12091332613507934748296/10^18 | T/F/F/F/T |

Four of the nine a5-minus-zero actual differences are positive (2252, 1972, 7404, 1550 PGU), five negative (-154, -7488, -174, -141, -731), and both estimate differences are zero. All four positive deficits remain visible; these directional whole-program differences do not establish an isolated SHA cost or two-sided component bound. Direct boundary actual differences are -1077/-852/-3387 for zero and -1357/-872/11505 for a5 at zero/single/full; changed preparation, code, length, gas and envelopes prevent isolated SHA-cost attribution.

The frozen PC02 route is `max(20M,42*native_gas)`, with a separate confirmed-spawn wrapper of 5M PGU. At L1269664, native gas 476184 yields structural 19999728 and selected 20000000 PGU; L1269665 gives native 476196 and selected 20000232 PGU; 2MiB gives native 786492 and selected 33032664 PGU. This frozen-source composition explains the quotes. The quote API exposes aggregate operations, ordinary dispatch and unique resources, without per-PC02 event/native-gas exports. Ordinary dispatch is already inside aggregate operations and must not be added again. Aggregate agreement does not prove per-event charging or allocate actual PGU per operation.

Zero-call account selectors are six; called points add the precompile account for seven. Persistent access/write selectors remain four from system work. Independent negative checks reject dropped dispatch, dropped PC02 account observation and invented persistent slots; empty selected storage does not eliminate authenticated account/code/global work.

## Qualification and retained evidence

See the [tool README](../../bin/zkgas/bench/README.md), [binding specification](../superpowers/specs/2026-11-04-sha256-linear-regime.md) and [implementation plan](../superpowers/plans/2026-11-04-sha256-linear-regime.md). Independent qualification checked 18 inputs, 36 signed users, exact ordered bytes/digests, code/accounts, empty roots and owner-qualified witness support, native receipts/gas/counters, ceil memory/partial MCOPY/unaligned output, return/forwarding, envelopes and full+1. Native evidence is 15 fresh guarded canonical calls plus three exact retained original positive smokes bridged by source/helper/object/request/runtime/GuestInput identities. All 18 original real quotes are fresh. This does not claim 18 fresh native calls.

The focused producer module discovered 12 tests: ten executed, two optional native tests skipped; the separately guarded three new positive native smokes ran once and passed. Independent semantic/domain negatives number 14/17 and byte-oracle self-checks 36 positive/10 negative. The earlier native-accepted altered-pattern specimen stays an old reference, not a new canonical execution. Default 18 byte compatibility and old source/measurement provenance remain intact.

All actuals ran sequentially once, six zero then six single then six full, with no implicit retry. Native/actual/quote timeouts were 180/600/1800s, quotes at most two workers; guards retained 6GiB sampled process-tree PSS, 192MiB output and 16MiB logs. Disk preflight required 256MiB floor plus 320MiB retention reserve. Peak sampled PSS was 4,544,553,984 bytes, maximum actual wall time 14.067108048126101s. No resource-limited or unmeasured case remains.

Independent raw 18-row/33-diagnostic arithmetic, phase order, native/quote bridges, public values, gas conversion, identities and resources passed without execution reruns. A fresh host receipt confirms all 72 unique owned shared-memory names absent; file-only verification cannot establish host absence. Post-actual closure verifies 166 frozen tracked source files unchanged at the producer revision before documentation edits. The original 378-file qualification snapshot remains immutable; the final independent manifest has 383 files.

| Identity | Value |
| --- | --- |
| Producer source | `6fa78cd44a7df9ce1e533cd90f3af5098939d2cc` |
| Declared guest source (not ELF attestation) | `4580faf7dc009714b310fd625b5810667019d683` |
| Frozen model ID | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| elf artifact | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| model artifact | `4cb6c65c6ad28e25e1f30639803958f9555e43d51e6d35d2f1945b0757ce1ac7` |
| native artifact | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| quote artifact | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| sp1 artifact | `1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b` |
| Frozen config | `2009780b7687af813a64c1c8862d1b2d05bf1f968665b83b09e78c4a0f417841` |
| Qualification verification | `93d8dbdbbe5c9f90490825824126e1f6acb586210a6b2c0ceff6b4917ded70cc` |
| Qualified input index | `62462efd4acc8bedc6b6ffcd93fb0e987a86ea90603d3849c2d8fb78235f67d7` |
| Retained-smoke bridge | `2f152aeb559b8741c62a794149ed591c6e73d50e836057158b5774590cb4236c` |
| Default 18 compatibility | `eebe4665bf71e1282959bf661984fd363ed5569f70b061a6700126f17c7fde10` |
| Original 378-file qualification manifest | `062d96e3999d370d91e97e37f1db9da92030962b2bdc591cf69d9873f9081294` |
| Pre-actual policy | `7067c5469ec2fa43a9a89a4eef0bf46dc5c94366f58d542e76cf2ced047a0d6e` |
| Actual analysis | `286e6711f3455495ebc7662696bbdd53e174b70d856260475c53cfca248bb21c` |
| Gas-budget report | `3f9f906b2fa0ce3454cc6313772729ebd6012afe222879e8bba8869c41a5a87d` |
| Counter retention audit | `f25104768250972e60e3eaf36a403916f92788febbd696ab7deccff17e0974ad` |
| Host shared-memory receipt | `63bd5dc42c61f0c50e068dbcfbdb9b4469a6ecf2be7e124ebf649cdceb9ef4ea` |
| Post-actual source closure | `c6ade6b7b178a3c27c8417f4f6369821dff0b11de5b55c96353a20c4358b2f47` |
| Independent raw actual audit | `61c6b81bf0308b587b24cf2b08029adc6c5269c7c85256614b55caf2d506084f` |
| Independent raw final closure | `883621efe3ef23c9381923b9dd269f7ecbae3b451eff019adef288522b84c7f5` |
| Independent raw final report | `50bbf7bc4493ddf1723f71b8c6c2afc3207068065a46ae217181c2a6c5ba4f43` |
| Independent final artifact manifest | `ccfd1390b1391173d7f8c75dafc05fae1ae6f0ec2d875520b1ef7791b1a2dc1c` |

The verified private local recovery archive is SHA256 `874363ff9cf0fa3e0e3fafe9c6062dcc4561d31213d25a688455ede3575b164a`, 2,339,164 compressed bytes, with 665 regular members carrying 25,409,427 payload bytes, 258 restorable duplicate aliases and 4 external runtime artifacts identified by hash. All streamed member hashes passed verification without extraction. Recovery receipt SHA256 is `f64c457f38e27c251b93caaf9f90f719e4bd511df46cec8f3f4df0469a315b95`. It retains canonical requests/inputs, original native/quote provenance and bridges, raw actuals, source/helper/runtime identities, controls, resources and independent audits. This is private recovery evidence, not a published standalone Atlas fixture or auditor runner.

## Interpretation and remaining work

Prices remain unchanged. Preserve every absolute gap, signed tail difference and exact increment comparison. The frozen high priority remains positive actual-minus-literal-node >2B PGU within <=30M charged gas; smaller gaps remain explicit refinements. Startup-scale resemblance does not causally assign cost. Conservative synthetic coverage does not certify ordinary admission usability or select an admission limit.

Coverage improves only for these exact two-tail unique-input schedules, selected crossover/linear lengths, successful calls and bounded uniform repetitions. SHA padding boundaries, arbitrary messages/cache behavior, malformed/failure paths, deeper composition and heterogeneous packing/order remain open. The declared source revision and SDK6.3/ELFb76 execute-only success do not attest the ELF build or exact production program. Exact production ELF/SDK parity and representative natural proposals require a separate promotion lane. Private recovery evidence is not a published standalone Atlas fixture or production attestation. No price fit, model/Rust/guest/SDK/ELF/pin change, proof generation, GPU-throughput claim, global/IID bound, merge, deployment or chain action follows.
