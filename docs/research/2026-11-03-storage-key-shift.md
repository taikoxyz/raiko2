# High-limb storage-key distributions

All 24 new execute-only observations completed once: 12 own-R0 controls and 12 generated full points. Nine literal node gaps remain: six K16 controls and three full REVERT/K16 points. Maximum full gap is 111,328,749 PGU; maximum all-point gap is 149,491,488 PGU. No complete-proposal gap or >2B actual-minus-node gap at <=30M charged gas appears. Maximum actual is 889,897,409 PGU. Prices remain unchanged; the smaller deficits remain explicit TODOs.

This panel varies valid raw storage-key limbs under the [many-key warm-dirty program](2026-11-03-storage-dirty-keys.md). Eight exact old unshifted observations are joined as references, without rerunning or counting them as new. These finite observations do not establish arbitrary hash-path, cache or nested-rollback coverage. See the [coverage inventory](zkgas-workload-coverage.md).

## Programs and gas limits

The 24 new inputs cross STOP/root REVERT, K=16/1024 keys/account, shift=64/128/192 and own-R0/generated-full levels. Four signed top-level CALL users each visit a distinct predeployed account once, with shared runtime and empty calldata; the unused primary is STOP. For account i, start=(i+1)*65536+1; raw phase slot 0 contains start, and body slot j is start+(j<<shift), j=0..K−1, initialized to 1. All other entries are absent. Slots are distinct U256 values, exported as hexadecimal identities, with no custom hashed-slot injection.

Each user reads phase0, seeds all K keys 1→5, performs R warm dirty 5→7→5 pairs/key in 64-pair batches plus exact tail, then cleans all K keys 5→1. Executed counters build marker K*R, followed by phase/marker checks. Memory remains six words (192 bytes). R0 still performs seed/cleanup. There is no nested frame, CALL wrapper or per-pair SLOAD. Both terminals restore complete contract maps and authenticated roots; fees, nonces and Anchor change the whole world. STOP users succeed; REVERT users fail intentionally; Anchor succeeds.

The shared implementation adds exactly three static PUSH3/SHL sites, executed 3K times/user in seed/dirty/cleanup key calculation before adding start. It retains the computed pointer during dirty pairs. This adds 18K ordinary gas/user at fixed R; storage gas remains 2200+K*(5600+200R), intrinsic gas 21000 and memory expansion 18. Preterminal refund is 2800K; STOP applies min(refund,gross//5), while REVERT discards effective/applied refund. The default shift0 path preserves all 16 preceding request/runtime/core-metadata goldens.

Literal user access pairs are 4(K+1), write pairs 4K: read-only phase0 is excluded from writes. Independent aggregate attempted counters survive root rollback and match original quotes plus fixed **observed** system allowances (+4 access/+4 write, 12363 ordinary dispatch). These allowances are not independently derived Anchor opcode totals; aggregate/unique collection does not prove per-opcode occurrence charging.

Every block retains a 31M header, <=30M charged gas including Anchor 167355, <=15M/user and EIP-7825 caps, gross+2301 full envelope, every ordered admission prefix/SSTORE sentry, <=8192 targets and <=24576 runtime bytes. Full means maximal uniform R for this finite program, re-solved after adding shift gas. The same solved R applies to all three shifts.

| Mode | K | Full R | Gross/user | Applied refund/user | Charged/user | Full signed envelope/user | Charged block gas |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| stop | 16 | 2108 | 7502508 | 44800 | 7457708 | 7504809 | 29998187 |
| stop | 1024 | 12 | 8984508 | 1796901 | 7187607 | 8986809 | 28917783 |
| revert | 16 | 2095 | 7457170 | 0 | 7457170 | 7459471 | 29996035 |
| revert | 1024 | 5 | 7421890 | 0 | 7421890 | 7424191 | 29854915 |

Nine full inputs are near-full (>=29.7M). The three STOP/K1024 inputs charge 28,917,783 (96.39261%). Their R13 programs charge only 29,632,123 but fail the last gross-envelope prefix: envelope 9,210,041 exceeds remaining header gas 8,734,069 by 475,972. The other R+1 cases exceed 30M charged gas. All 12 R+1 programs execute with sufficient gas before applying candidate constraints. No cap waiver or global gas-packing maximum follows.

## Literal same-proposal results

IDs are `storage-key-shift-{mode}-k{K}-s{shift}-r{R}`. Actual PGU is raw SDK6.3 gas*10//191 with exact integer conversion. Node is the sum of literal block ceilings; complete proposal is separate. Positive gaps are max(actual−estimate,0), retaining the literal denominator. Full node/actual ratios are 0.87489710–1.15757542; complete-proposal/actual ratios are 1.05133257–1.63821362.

| Full mode | K | Shift | R | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive proposal gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| revert | 16 | 64 | 2095 | 29996035 | 887915020 | 778568660 | 935567874 | 109346360 | 0 |
| revert | 16 | 128 | 2095 | 29996035 | 889897409 | 778568660 | 935578130 | 111328749 | 0 |
| revert | 16 | 192 | 2095 | 29996035 | 887911536 | 778568660 | 935568130 | 109342876 | 0 |
| revert | 1024 | 64 | 5 | 29854915 | 515497261 | 591302654 | 844494636 | 0 | 0 |
| revert | 1024 | 128 | 5 | 29854915 | 516159198 | 591302654 | 844410908 | 0 | 0 |
| revert | 1024 | 192 | 5 | 29854915 | 515766234 | 591302654 | 844468748 | 0 | 0 |
| stop | 16 | 64 | 2108 | 29998187 | 681088671 | 763173261 | 920172476 | 0 | 0 |
| stop | 16 | 128 | 2108 | 29998187 | 681088972 | 763173261 | 920182732 | 0 | 0 |
| stop | 16 | 192 | 2108 | 29998187 | 681085660 | 763173261 | 920172732 | 0 | 0 |
| stop | 1024 | 64 | 12 | 28917783 | 583490614 | 675240221 | 928432204 | 0 | 0 |
| stop | 1024 | 128 | 12 | 28917783 | 583322872 | 675240221 | 928348476 | 0 | 0 |
| stop | 1024 | 192 | 12 | 28917783 | 583464931 | 675240221 | 928406316 | 0 | 0 |

All 12 R0 controls remain explicit below. Their own full signed envelopes and complete initial storage maps are retained. Complete-proposal gaps are zero for every control.

| Control mode | K | Shift | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| revert | 16 | 64 | 653955 | 165602809 | 36006827 | 193006041 | 129595982 |
| revert | 16 | 128 | 653955 | 165604929 | 36006827 | 193016297 | 129598102 |
| revert | 16 | 192 | 653955 | 165597273 | 36006827 | 193006297 | 129590446 |
| revert | 1024 | 64 | 25390275 | 409745569 | 517060583 | 770252565 | 0 |
| revert | 1024 | 128 | 25390275 | 409427461 | 517060583 | 770168837 | 0 |
| revert | 1024 | 192 | 25390275 | 409482381 | 517060583 | 770226677 | 0 |
| stop | 16 | 64 | 556619 | 165493600 | 16005495 | 173004710 | 149488105 |
| stop | 16 | 128 | 556619 | 165496983 | 16005495 | 173014966 | 149491488 |
| stop | 16 | 192 | 556619 | 165491681 | 16005495 | 173004966 | 149486186 |
| stop | 1024 | 64 | 20345675 | 401858929 | 497059251 | 750251234 | 0 |
| stop | 1024 | 128 | 20345675 | 401593528 | 497059251 | 750167506 | 0 |
| stop | 1024 | 192 | 20345675 | 401626295 | 497059251 | 750225346 | 0 |

## Matched diagnostics

The retained analysis contains 64 whole-proposal diagnostics: 12 full−own-R0 increments, 12 REVERT−STOP terminal contrasts, 16 adjacent-shift contrasts (64→128→192), and 24 new-shift−exact-old0 contrasts. Each retains actual PGU differences and exact pre-rounding node/proposal differences scaled by 10^18 units/PGU. Subtracting rounded ceilings does not replace those scaled differences.

Each diagnostic explicitly records same-R, same-K, same-charged-gas, same-signed-envelope and same-authenticated-selected-storage-support flags. Equality of selected support compares canonical **per-address storage-node hash sets/digests**, with underlying nodes retained; equal counts or full-input hashes do not establish equality. Account/global witness support is separate.

| Diagnostic group | Count | Same R | Same K | Same charged gas | Same signed envelope | Same selected storage support | Actual difference range PGU |
| --- | ---: | --- | --- | --- | --- | --- | ---: |
| Full−own-R0 | 12 | false | true | false | true | true | 105751692..724292480 |
| REVERT−STOP | 12 | true for R0; false for full | true | false | false | true | -67993353..208808437 |
| Adjacent shifts | 16 | true | true | true | true | false | -1985873..1982389 |
| New shift−old0 | 24 | true | true | false | false | false | -138443..2943703 |

Across all 64 rows, true flags total 46/64/16/28/24 respectively. Fixed mode/K/R shifts retain executed shape and gas while changing raw keys, code/trie hashes and witness bytes. Own-zero comparisons hold the signed envelope and selected storage support equal but change executed work and gas. Terminal/full comparisons can change R and refunds. Old0 comparisons include the added shift instructions/gas and changed envelopes/support. These contrasts do not allocate actual cost to SHL, storage, rollback or any isolated operation, and do not select tariffs. Absolute deficits and ordinary usability limits remain visible even when an increment is covered.

## Tools, qualification and retained evidence

See the [producer](../../bin/zkgas/bench/cases/storage_key_shift.py), [shared runtime](../../bin/zkgas/bench/cases/storage_dirty_keys.py), [usage](../../bin/zkgas/bench/README.md#high-limb-storage-key-distributions) and [binding specification](../superpowers/specs/2026-11-03-storage-key-shift.md). No price/refit, Rust/guest/host/GuestInput/SDK/ELF/pin change, guest build or actual retry occurred.

Retained implementation checks passed with managed Python and stdlib `-S`:

- `STORAGE_KEY_SHIFT_NATIVE_BIN="$NATIVE_BIN" python -S -m unittest discover -s bin/zkgas/bench/tests -p test_storage_key_shift.py -v`: eight tests passed with the frozen-native opt-in, including all 24 literal trajectories, all 16 default goldens, eight references, twelve R+1 programs and four native specimens.
- `PYTHONPATH=bin/zkgas/bench/tests python -S -m unittest test_storage_dirty_keys test_storage_mutation test_storage_root_journal test_storage_long_revert -v`: 32 tests total: 28 executed successfully and four optional-native tests intentionally skipped.
- Separate strict/SHL/catalog three-test, refined semantic-negative one-test and independently serialized golden one-test lanes passed. An intermediate test assertion counted unrelated equal-valued PUSH3 literals; the corrected assertion checks PUSH3 immediately before SHL. Its failure remains retained.

These are retained command results, not tests rerun for this record. The native opt-in selects the configured frozen binary. Independent source review passed. Independent all 24 qualification then checked 96 user signatures/receipts, complete original/final worlds and authenticated per-account roots, canonical conservative mutation witnesses, literal high-key event streams/gas/refunds/counters/markers, prefix/sentry, maximal R and sufficient-envelope R+1, and all 24 original real quotes before freeze. Boundary/overflow SHL cases, semantic negatives, native-accepted dirty9 mutants and attempted-counter controls are retained.

Independent raw24/diagnostic64 audit passed exact conversion, original input/public-output/quote joins, literal node sums versus complete proposals, eight exact reference bindings, five comparison flags, resources and guards without launching measurements. All 24 actual executions succeeded as unique attempts, sequentially bounded by 6GiB sampled process-tree PSS and 600s; peak sampled PSS was 4,576,620,544 bytes and maximum retained execution wall time was 26.443126s. Native was bounded at 180s and quotes at 1800s/max two concurrent. Policy retains a 256MiB free floor plus 320MiB reserve (576MiB total), 192MiB output and 16MiB logs.

A separate host-context receipt confirms all 96 unique exact owned shared-memory names absent. A private posthoc host assertion initially omitted `sem.sp1_` semaphore names; the failed assertion and corrected checker are retained. This checker correction required no actual rerun. The raw audit alone does not establish host namespace absence.

The ELF declared source revision is `4580faf7dc009714b310fd625b5810667019d683`; this is not a build attestation. These are execute-only observations, not a generated proof, GPU throughput result, production certification, natural historical admission result or global bound. The prior `ab932` CI run already failed at the prover SDK6.8.1 runner's missing `include_bytes!` binary and the retained RISC0 fingerprint check in xtask (run 41591818335, jobs 125770123207/125770123162). This Python/tests/docs increment does not repair those failures or establish current-head CI success.

| Identity | SHA256 / version |
| --- | --- |
| Producer source | `b2778fe39df1de18dece1d44284f6def3fa0237e` |
| Model | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Retained ELF / SDK | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` / `6.3.0` |
| Native executable | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Quote executable | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| SP1 executable | `1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b` |
| Frozen source inventory | `7430b1f3ee4cdf70f0825e97b4a6a22495a57f717557f83808d26daf6a834a20` |
| Frozen config | `8592d24875f4825faf1384cd96905253cf5dff35be1c3aa4ae0eec1f4c798f30` |
| Pre-actual policy | `90fb34bdb3cdabf0bc880813afbb30226d1be3386b149f61acddf8baf9defc58` |
| Campaign-derived input/quote index | `a43ac662dc463b3fc49da8c6661bf80d485bee85d9a35d629811304c1b2dbab2` |
| Independent qualification/support index | `f21abcb48c25df83fe8f1604dd0d3f8d431cf90b65f96ab6abfc6e33f70e0146` |
| Reference join | `49d60b5e254b5b3cb84942731642a4f5a28a749e580964900530ae310da26b0a` |
| Counter retention audit | `b0e69895459ffca9475726aee3eafaec6677be66398dfcac882de983bb31ccbf` |
| Actual analysis | `836417cd8fe10d679a47ba78deb22067088be62e10b6983dd8f6e9f3d79006db` |
| Gas-budget report | `5faf7a61da1bbf6ee8e548da3bfa540269b3679a07451e3cc405717c936fcfb7` |
| Independent raw24/diagnostic64 audit | `3f2a3868fc7ff294f7480d2fea89d5dafcc708e2be2e71ba6176e6916cade58a` |
| Independent final manifest | `3a9a137b2a63afaffd7d53e527639b89b60f94d4e333a60466f02b9037e863a9` |
| Independent closure | `9bc7f58ba677d66d6ef842302ff0892c001b8c02325803acff48b38e6a4dceda` |
| Independent final report | `837943f8e891e24dc95d32db3ecbca326af7d88ab694c532c8550557e53a66eb` |
| Host shared-memory receipt | `598788ecde5c30affc9f91f0b690e7f5fd9fece6d62123ec5c8fbb2d703ab3f0` |
| Seven-panel storage-cohort summary | `26df1ff9c643ec3422557eb0f706eb1e3b031afde7dcaa72cb7b5a01825fb3bc` |
| Measurement runtime summary | `b78d811343a1fe32a23151952544578d49be88e4815e8ad1322d2755c987824f` |
| Recovery receipt | `536d870ef4ff06a579e4f0f1b007089cdfca42a9f678a6893fd8d89da277555a` |
| Previous-head CI observation | `db18860063e6a3adcce279bc2dc656cd70436ca4e78e611d782aa7210c3a59b0` |

The independently rejoined seven-panel storage subset contains 172 distinct canonical inputs: 76 generated full selectors and 72 near-full inputs, 16 full node gaps with maximum 111,328,749 PGU, maximum all-point gap 164,072,990 PGU, no >2B/30M gap and no complete-proposal gap. Eight reused references are excluded. Source-analysis identities, identical model/ELF/SP1 identities and canonical-input deduplication are checked. The preceding 148/64/63 snapshot remains historical; maxima are not monotonic. This is not an all-research count, IID confidence bound or global coverage percentage.

Retained requests/canonical inputs, original quotes/invocations, all 24 successful actual attempts, selected-support joins, private helper/dependency identities, negative checks, audits and host receipts are recoverable. The verified local archive `storage-key-shift-evidence-20261103.tar.gz` is SHA256 `9f5b3411a19179d4eee061ea3994232ffdd08c278d4ce04b68ddfe3009a1a6ec`: 34,707,433 compressed bytes, 715 regular members carrying 98,178,098 payload bytes, 144 restorable duplicate aliases and four external runtime artifacts identified by hash. All streamed member hashes were verified without extraction. The eight old references retain their prior archive SHA256 `3d45e466a9610fa369ed6f8b0b70694599789777a15329c7148bbf3b35b258ff`; they are external prior evidence rather than newly measured archive inputs. This is private recovery evidence, not public Atlas fixture publication or a standalone auditor runner.

Remaining work includes many-key × nested/caught rollback, deliberate hash-path skew/depth and mutation topology, broader cache/lookup distributions, heterogeneous packing/order and an exact-production-program/natural-history lane. High raw limbs do not establish exploitable hash collisions or close those interactions. No merge, deployment or on-chain action is part of this panel.
