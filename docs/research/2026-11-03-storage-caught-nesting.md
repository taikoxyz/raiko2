# Many-key storage through caught nested calls

All 16 new execute-only observations completed once: eight own-R0 controls and eight generated full points. Six literal node gaps remain: four K16 controls and two full caught-REVERT/K16 points. Maximum full gap is 105,568,665 PGU; maximum all-point gap is 145,680,932 PGU. No complete-proposal gap or >2B actual-minus-node gap at <=30M charged gas appears. Maximum actual is 887,875,249 PGU. Prices remain unchanged; smaller deficits remain explicit TODOs.

This panel carries the unchanged [many-key warm-dirty leaf](2026-11-03-storage-dirty-keys.md) through separate CALL owners at wrapper depths 1/4. Eight exact old direct observations are joined read-only, without rerun or recount. The [coverage inventory](zkgas-workload-coverage.md) records this finite interaction and the remaining work.

## Programs, frame gas and finite limits

The 16 new inputs cross leaf STOP/REVERT, K=16/1024, wrapper depth=1/4 and own-R0/generated-full. Four signed top-level users visit four disjoint chains once with empty calldata. Depth counts wrappers; configured chain accounts total 4*(depth+1), with exactly four populated leaf storage owners. Wrappers encode their own next target and have explicit empty maps. The unused primary is STOP. Every user receipt succeeds, including caught leaf REVERT; Anchor also succeeds.

For leaf i, start=(i+1)*65536+1, phase slot 0 contains start, and keys start..start+K−1 contain 1. The original leaf reads phase, seeds all K keys 1→5, performs R warm dirty 5→7→5 pairs per key in 64-pair batches plus tail, then cleans 5→1. Executed round and marker checks retain K*R; memory remains 192 bytes. Own-R0 includes seed/cleanup. Complete leaf maps and roots restore under both terminals; wrapper maps remain empty. Fees, nonces and Anchor still change whole-world state.

Every wrapper requests 12M gas with zero value and empty input/output ranges, checks child status and zero RETURNDATASIZE, then STOPs. The innermost expects leaf status 1/0; outer wrappers expect 1. Each wrapper consumes ordinary pre-CALL 21/post-CALL 46 gas plus cold CALL 2600, adding 2667 gross. Let C=original leaf gross−21000: transaction gross=21000+C+2667*depth. No repeated child intrinsic enters that total.

Leaf preterminal refund is 2800K. STOP propagates it to the successful user; caught REVERT discards it, so the user preterminal/effective counter is zero. Refund applies once at the transaction as min(propagated counter,gross//5). The envelope recurrence is T0=C+2301 and Tj=2621+max(ceil(64*T(j−1)/63),T(j−1)+46); the signed user envelope is 21000+Tdepth. Independent frame checks reconstruct actual min(12M,available−available//64) forwarding, child requirements, returned gas and the retained post-CALL 46.

All blocks have a 31M header and <=30M charged gas including Anchor 167355; constraints include <=15M/user and EIP-7825, every ordered admission prefix, leaf SSTORE gas-left >2300, <=8192 targets and <=24576 code bytes. Full means maximal uniform R for this fixed program. Its signed envelope is shared with own-R0. Sufficient-envelope R+1 programs execute before candidate constraints are checked; no global packing maximum follows.

| Leaf | K | Depth | Full R | Gross/user | Applied refund/user | Charged/user | Signed envelope/user | Charged block gas |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| revert | 16 | 1 | 2094 | 7456061 | 0 | 7456061 | 7576328 | 29991599 |
| revert | 16 | 4 | 2092 | 7457086 | 0 | 7457086 | 7942407 | 29995699 |
| revert | 1024 | 1 | 5 | 7406125 | 0 | 7406125 | 7525599 | 29791855 |
| revert | 1024 | 4 | 5 | 7414126 | 0 | 7414126 | 7896654 | 29823859 |
| stop | 16 | 1 | 2107 | 7501399 | 44800 | 7456599 | 7622385 | 29993751 |
| stop | 16 | 4 | 2105 | 7502424 | 44800 | 7457624 | 7990692 | 29997851 |
| stop | 1024 | 1 | 12 | 8968743 | 1793748 | 7174995 | 9113020 | 28867335 |
| stop | 1024 | 4 | 11 | 8753512 | 1750702 | 7002810 | 9323126 | 28178595 |

Six full points exceed 99% charged occupancy. STOP/K1024/depth1 charges 28,867,335 and depth4 charges 28,178,595; their next R fails an admission prefix despite remaining below 30M charged gas. These are prefix-limited full selectors, not near-full or arbitrary-capacity observations.

## Literal same-proposal results

IDs are `storage-caught-nesting-{mode}-k{K}-d{depth}-r{R}`. Actual PGU is whole-proposal SDK6.3 `ExecutionReport.gas * 10 // 191`. Literal node PGU is the sum of per-block known-PGU ceilings; these inputs each have one block. Complete-proposal estimates remain separate. Raw Unzen scalars are not actual PGU. Positive gaps are max(actual−estimate,0). Full node/actual ratios are 0.88109285–1.18757498; complete-proposal/actual ratios are 1.05814736–1.66924937.

| Full leaf | K | Depth | R | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive proposal gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| revert | 1024 | 1 | 5 | 29791855 | 513448536 | 592065560 | 845459335 | 0 | 0 |
| revert | 1024 | 4 | 5 | 29823859 | 514119942 | 604348887 | 858194390 | 0 | 0 |
| revert | 16 | 1 | 2094 | 29991599 | 887824384 | 782255719 | 939449030 | 105568665 | 0 |
| revert | 16 | 4 | 2092 | 29995699 | 887875249 | 793830185 | 951475224 | 94045064 | 0 |
| stop | 1024 | 1 | 12 | 28867335 | 582403149 | 676003384 | 929397159 | 0 | 0 |
| stop | 1024 | 4 | 11 | 28178595 | 567069914 | 673438040 | 927283543 | 0 | 0 |
| stop | 16 | 1 | 2107 | 29993751 | 681094017 | 766860320 | 924053631 | 0 | 0 |
| stop | 16 | 4 | 2105 | 29997851 | 689193906 | 778434786 | 936079825 | 0 | 0 |

All eight own-R0 controls retain their complete leaf maps and own full signed envelope; complete-proposal gaps are zero.

| Control leaf | K | Depth | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| revert | 1024 | 1 | 25327215 | 407649499 | 517823489 | 771217264 | 0 |
| revert | 1024 | 4 | 25359219 | 408356578 | 530106816 | 783952319 | 0 |
| revert | 16 | 1 | 663471 | 165833204 | 40048189 | 197241499 | 125785015 |
| revert | 16 | 4 | 695475 | 166583374 | 52331259 | 209976298 | 114252115 |
| stop | 1024 | 1 | 20295227 | 399787319 | 497822413 | 751216188 | 0 |
| stop | 1024 | 4 | 20320827 | 400494704 | 510105484 | 763950987 | 0 |
| stop | 16 | 1 | 564231 | 165727789 | 20046857 | 177240168 | 145680932 |
| stop | 16 | 4 | 589835 | 166462125 | 32329928 | 189974966 | 134132197 |

## Matched diagnostics and attempted work

The retained 32 diagnostics comprise eight full−own-R0, eight leaf REVERT−STOP, eight depth4−depth1 and eight depth1−exact-old-direct comparisons. They retain actual differences and exact pre-rounding node/proposal differences scaled by 10^18 units/PGU. Rounded-ceiling subtraction does not replace those scaled differences. The five equality columns below count true flags within each group of eight.

| Diagnostic | Count | Same R | Same K | Same charged gas | Same signed envelope | Same selected storage support | Actual difference range PGU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Full−own-R0 | 8 | 0 | 8 | 0 | 8 | 8 | 105763364..721991180 |
| Leaf REVERT−STOP | 8 | 4 | 8 | 0 | 0 | 8 | -68954613..206730367 |
| Depth4−depth1 | 8 | 5 | 8 | 0 | 0 | 8 | -15333235..8099889 |
| Depth1−old direct | 8 | 6 | 8 | 0 | 0 | 8 | -130086..1175098 |

True flags total 15/32/0/8/32 respectively. Selected-storage equality compares canonical per-address authenticated node hash sets/digests for exactly the four leaf owners, with underlying nodes retained. Empty wrappers have no storage targets; their account/code proofs and global support are authenticated separately. Equal leaf support does not mean equal complete inputs, account/code support or global witness. Old direct REVERT user receipts fail, while new caught-REVERT user receipts succeed. Full/terminal/depth/direct contrasts can change R, refunds, gas, envelopes and account/code loading. They do not isolate frame, storage or rollback actual cost or select a tariff.

Independent attempted counters retain completed child work through caught REVERT: CALL and every wrapper STOP remain counted alongside leaf STOP/REVERT, dispatch and distinct access/write/account resources. Literal user access pairs are 4(K+1), writes 4K; read-only phase0 is excluded from literal writes. Fixed observed system additions are +4 access/write/accounts and 12363 dispatch. These aggregate observations are not an independently derived Anchor opcode total or proof of per-opcode occurrence charging.

## Tools, qualification and retained evidence

See the [producer](../../bin/zkgas/bench/cases/storage_caught_nesting.py), [frame test interpreter](../../bin/zkgas/bench/tests/storage_caught_vm.py), [usage](../../bin/zkgas/bench/README.md#caught-nested-storage-calls) and [binding specification](../superpowers/specs/2026-11-03-storage-caught-nesting.md). Model c893, native/quote, SDK6.3 and ELFb76 remain unchanged. No price fit, Rust/guest/host/GuestInput/SDK/ELF/pin change or build occurred.

Retained managed-Python `-S` checks passed; these are original results, not reruns for this record. Portable command forms below use `PYTHON_BIN` for the existing managed interpreter and `NATIVE_BIN` for the frozen native path:

- `STORAGE_CAUGHT_NATIVE_BIN="$NATIVE_BIN" "$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests -p test_storage_caught_nesting.py -v`: original six tests passed, including four native specimens: STOP/K16/depth1 and caught-REVERT/K1024/depth4 plus dirty9 mutants. Native accepts mutants; literal intent rejects them.
- `PYTHONPATH=bin/zkgas/bench/tests "$PYTHON_BIN" -S -m unittest test_storage_caught_nesting test_storage_dirty_keys test_storage_key_shift test_storage_root_journal test_storage_mutation -v`: original 38 tests, 33 executed successfully and five optional-native skips.
- After the refund metadata fix, `"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests -p test_storage_caught_nesting.py -v`: seven tests, six passes and one optional-native skip. The 38-test lane was not rerun after that fix.

Independent source review found that original f8f metadata copied the leaf preterminal refund into the top/user field for caught REVERT. Reviewed e818 separates leaf and user counters. All 16 construction requests and leaf/wrapper runtimes remain byte-identical; gas, prices and envelopes are unchanged. Original producer/catalog and invocation provenance remain retained. The first three native commands bind f8f, the remaining 13 native and 16 original quotes bind e818; reviewed catalog/source-transition records bind the identities without relabelling or repeating originals. The same reviewer rechecked the fix and passed.

All 16 original native and original real quotes passed before actual freeze, including 64 signatures, all successful outer receipts, complete original/final worlds and authenticated account/code/storage roots, mutation witnesses, literal leaf events/markers, frame gas/refunds and attempted counters. Four populated leaf roots and all empty wrapper roots are checked. A private empty-trie scalar-root boundary assertion was corrected before qualification; a source guard later interrupted before the fourth native invocation, with no new attempt started. Both events are retained harness/source-transition facts, not SP1 failures. Semantic negatives, native-accepted dirty9 mutants and counter controls remain retained.

Independent raw16/diagnostic32/cohort188 verification passed exact gas conversion, original input/output/quote joins, literal node sums and complete totals, old-reference identities, all five flags, gas/refund/resource bounds and canonical-ID deduplication. All 16 actuals completed once with no actual failure/retry. Sequential actual limits were 600s and 6GiB sampled process-tree PSS; maximum retained wall time was 26.21506785415113s and peak sampled PSS 4,571,302,912 bytes. Native was bounded at 180s, quotes at 1800s/max two concurrent. Policy retained 256MiB free plus 320MiB reserve, 192MiB output and 16MiB logs. A separate host receipt confirms all 64 exact owned shared-memory names absent.

The eight-panel storage subset contains 188 distinct canonical inputs, 84 generated full selectors and 78 near-full points. It has 18 full node gaps, maximum full gap 111,328,749 PGU, maximum all-point gap 164,072,990 PGU, no >2B/30M gap and no complete-proposal gap. Eight reused references are excluded. Identical model/ELF/SP1 identities and canonical-input deduplication are checked. The historical 172/76/72 snapshot is unchanged; maxima are not monotonic, and this is not an all-research count, IID bound or global coverage percentage.

| Identity | SHA256 / version |
| --- | --- |
| Original / reviewed producer source | `f8f00822e15e8212ad9b89e859b414f5c481ceb1` / `e8186fd0f54d8d1319a97b63e3e268ba89051607` |
| Model | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Retained ELF / SDK | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` / `6.3.0` |
| Native executable | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Quote executable | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| SP1 executable | `1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b` |
| Frozen config | `f55060e88569505ae0863d8444f0427cbda3836bc846d98d876d42c76011e5ea` |
| Pre-actual policy | `8d283eada4dd45ef6a7d322c71de12396c409d7aae8c9df34c6a88b2b1fde410` |
| Source transition | `5247de2ebffe93b248dffd170bda4309a04f8261c2a85cbed85c703f49d6fc94` |
| Campaign input/quote index | `366e29e0a3f4a3032a254bedc033ba59abe250d2b107b9c471ba95c7abb753ac` |
| Independent qualification | `67493f382bd7c6b8c369279eba0d4a618f28a0d2d18e50909f37666ad6b58064` |
| Reference join | `a3e41d5712bada621a1070c40a75bd21fa20dc934c0a1739c6cd14e7806b15a2` |
| Frame semantics audit | `414e4e6bf888f698e4da61e2e0c16716e7c220851effb6d5ec3abaa893e08edc` |
| Counter retention audit | `14efb50e455bb6f2ac42acb0cd87a67eaa56fb6b4bc4258b38fe4219da7e895f` |
| Actual analysis | `aad21aac18d66db43f9ea58792eaa5df01c96218c0295f0e28eeaaa18ef3af2f` |
| Gas-budget report | `9292a96db65119d8d2415502406fa4ae03ef6b9d6954f70b68365fc7ee28c22c` |
| Independent raw audit | `6e3c162031835cd623e77f5f0376890bbc03a92178fc03b83ff385a36c7c71d2` |
| Independent focused checks | `4d51a2820b834b1305c4102c1ee9ac25a86fd04459a51432c38ceb6fcac11d3a` |
| Independent closure | `fe6c37cb9737d52b4d8d409e1ae329932399d1134d97594225f1df60e6eb6a64` |
| Independent final report | `2a0e6394fc7ad0694a3583bd94eda52a7fd24ba198a1421728037bede15fc10d` |
| Independent final 284-file manifest | `43ce23444e8a6362cea9e86fef2566756f15ea1ed45e542c3dc1527fca55a7b3` |
| Previous-head CI observation | `676d4b9b1b34640640ebe1eea2bd1a08bb46f167861191d0e8b41ab398c01bba` |
| Eight-panel storage summary | `79d5cdf73c508fda6e4cdf20424df23c1221fe194b812e8f4123080690d95b2d` |
| Runtime summary | `40e8987deeaaffeadaa7a13506d4d2321125591429e81d9a636290361a4f7ae6` |
| Host shared-memory receipt | `4d3c460220ef204142382e24e959f3f08fc30d895d38cd7f1b18ef767388f481` |
| Recovery receipt | `0999ce01a9e5d45e9bee24fed7f08469d346b403f1052730f360539d62d79674` |

The ELF declares source revision `4580faf7dc009714b310fd625b5810667019d683`; this is not a build attestation. These observations execute the guest; they generate no proof and establish no GPU throughput, exact production parity, natural historical admission result or global bound. The previous published e5 head CI run 41596522594 failed in prover job 125785151128 at the SDK6.8.1 nested runner's missing binary and in xtask job 125785150986 at the retained RISC0 fingerprint check. This Python/tests/docs increment does not repair those failures or establish current-head CI success.

Raw requests/canonical inputs, original quotes/invocations, native diagnostics, all 16 successful actual attempts, eight exact prior references, helper/dependency identities and audit/resource receipts are retained privately. The verified local archive `storage-caught-nesting-evidence-20261103.tar.gz` is SHA256 `09f9b680e0b7d3b4424132121c33af57316cfb257285b58303886cd715710b6d`: 26,460,673 compressed bytes, 544 regular members carrying 74,772,038 payload bytes, 96 restorable duplicate aliases and four external runtime artifacts identified by hash. All streamed member hashes were verified without extraction. The eight old references remain external prior evidence through warm-dirty archive SHA256 `3d45e466a9610fa369ed6f8b0b70694599789777a15329c7148bbf3b35b258ff`; they are not newly measured archive inputs. This private recovery evidence is not a standalone auditor runner or a public immutable Atlas fixture.

Remaining interactions include DELEGATECALL storage ownership, callers that prewarm storage, ancestor rollback, broader nested graphs and exceptional exits, deliberate hashed-path skew/depth and mutation topology, cache/lookup distributions, heterogeneous packing/order and exact-production-program/natural-history promotion. This panel samples many-key warm work through successful ancestors and caught child rollback; it does not close those wider domains. No merge, deployment or on-chain action is part of it.
