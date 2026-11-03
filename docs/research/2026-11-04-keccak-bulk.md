# Unique-input KECCAK bulk and padding edges

This separate panel measures 27 successful whole proposals: nine length/pattern domains × own-zero/single/full, with two authenticated tagged code owners and two contiguous signed users. All 27 actual executions completed once without failure or retry. All nine full points are near-full and their literal node estimates cover actual. Eighteen node gaps remain (nine controls and nine singles), maximum 153,504,011 PGU; complete-proposal gaps and positive node gaps >2B PGU within 30M charged block gas are zero. Maximum actual is 2,358,803,804 PGU, a different quantity from the maximum gap. Full node/actual ratios span 1.06114420994–1.20631123096.

All 27 own-R increments are covered by both exact scaled estimates. Six of 12 signed a5-minus-zero contrasts and six of 12 padding-edge contrasts remain uncovered; maximum upward-rounded deficits are 2,292 and 763,493 PGU. Absolute full coverage does not erase these incremental TODOs or the control/single gaps. Whole-proposal deficit positivity neither proves nor excludes startup-only cost or a carried baseline gap. No isolated per-hash actual PGU or causal cost decomposition is measured.

## Program and capacity contract

Domains are L0/zero and L135/136/137/65536 with zero/a5 tails. Empty hashing repeats the same empty message and follows the constant shortcut, with permutation feature zero; it is not a unique-input workload. For nonempty calls indexed i from zero, exact hashed bytes are `BE256((tag << 128) | i) || pattern * (L - 32)`, using distinct owner tags 0/1. Ethereum Keccak uses suffix 0x01 and rate 136; the existing frozen model already uses `L // 136 + 1` permutations for nonempty inputs, including the extra padding permutation at L136. No price correction is made here.

The full aligned memory extent is prepared once using logarithmic MCOPY even at R0. Only nonempty partial words retain 0xee padding outside hashed bytes. Counter, final digest, final nonce word and tail guards bind the intended loop. The public host reference uses lazy digest tokens with exact snapshot/materialization for every consumer and discards unused tokens on complete-word overwrite; it retains at most eight sampled events and one pending 64KiB message. Retained direct R0/1/2, feedback/snapshot and independent concrete Crypto checks validate that optimization. Actual EVM/SP1 execution evaluates every hash. Host optimization timing establishes no guest performance claim.

Own-zero controls retain both full signed transaction envelopes and preparation, with empty calldata and empty selected persistent storage at both authenticated owners. Equal envelopes/support do not establish equal code, GuestInput, account authentication or global witness work. The Anchor contributes 167,355 charged gas and whole-block hash work; quote aggregates do not establish user-only per-hash cost.

Every full point is maximal only for this uniform-R fixture. The 15M per-user fixture cap, EIP-7825 cap 16,777,216, 31M header ordered admission prefixes, 30M charged-block budget and 1000-gas continuation margin are separate constraints. Independent concrete full+1 checks execute both owners at sufficient gas; every full+1 violates only the charged-block budget among the retained six capacity constraints. No refund applies. This does not establish global packing optimality.

| Hashed L | Tail | Full R/user | Full charged gas | Full+1 R/user | Full+1 envelope/user | Full+1 charged gas | Fixture/EIP7825/header/repetition/runtime caps |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | --- |
| 0 | zero | 129523 | 29999957 | 129524 | 14917416 | 30000187 | all pass |
| 135 | a5 | 102724 | 29999821 | 102725 | 14917378 | 30000111 | all pass |
| 135 | zero | 102724 | 29999821 | 102725 | 14917378 | 30000111 | all pass |
| 136 | a5 | 102724 | 29999821 | 102725 | 14917378 | 30000111 | all pass |
| 136 | zero | 102724 | 29999821 | 102725 | 14917378 | 30000111 | all pass |
| 137 | a5 | 102724 | 29999821 | 102725 | 14917378 | 30000111 | all pass |
| 137 | zero | 102724 | 29999821 | 102725 | 14917378 | 30000111 | all pass |
| 65536 | a5 | 1199 | 29993361 | 1200 | 14926406 | 30018167 | all pass |
| 65536 | zero | 1199 | 29993361 | 1200 | 14926406 | 30018167 | all pass |

## Literal whole-proposal results

Actual PGU is raw SDK gas ×10//191, with SDK 6.3.0 gas-estimator execution. Literal node values sum original block estimates; complete quotes include proposal work. Unzen raw scores are not PGU. Both literal underpricing flags are shown for every row, including controls and singles; signed actual-minus-estimate values retain negative margins. Near-full means charged gas ≥29.7M and ≤30M. No admission limit is selected. These are execute-only observations, not STARK proving, aggregation, final wrapping, GPU or production validation.

| Input (keccak-bulk-) | Charged gas | Raw SDK gas | Actual PGU | Literal node PGU | Complete quote PGU | Actual-node | Actual-complete | Node gap? | Complete gap? | Near-full? |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- | --- |
| l0-zero-n0 | 209667 | 3080459532 | 161280603 | 7776592 | 162769679 | 153504011 | -1489076 | yes | no | no |
| l0-zero-n1 | 209897 | 3080592434 | 161287561 | 7786642 | 162779729 | 153500919 | -1492168 | yes | no | no |
| l0-zero-n129523 | 29999957 | 20732740680 | 1085483805 | 1309431305 | 1464424392 | -223947500 | -378940587 | no | no | yes |
| l135-a5-n0 | 209861 | 3080595749 | 161287735 | 7800306 | 162793392 | 153487429 | -1505657 | yes | no | no |
| l135-a5-n1 | 210151 | 3080784376 | 161297611 | 7815651 | 162808737 | 153481960 | -1511126 | yes | no | no |
| l135-a5-n102724 | 29999821 | 26492351162 | 1387034092 | 1584080947 | 1739074034 | -197046855 | -352039942 | no | no | yes |
| l135-zero-n0 | 209861 | 3080622934 | 161289158 | 7800306 | 162793392 | 153488852 | -1504234 | yes | no | no |
| l135-zero-n1 | 210151 | 3080820220 | 161299487 | 7815651 | 162808737 | 153483836 | -1509250 | yes | no | no |
| l135-zero-n102724 | 29999821 | 26492307719 | 1387031817 | 1584080947 | 1739074034 | -197049130 | -352042217 | no | no | yes |
| l136-a5-n0 | 209861 | 3080598630 | 161287886 | 7800306 | 162793392 | 153487580 | -1505506 | yes | no | no |
| l136-a5-n1 | 210151 | 3080854783 | 161301297 | 7819945 | 162813031 | 153481352 | -1511734 | yes | no | no |
| l136-a5-n102724 | 29999821 | 33606532632 | 1759504326 | 2025197089 | 2180190176 | -265692763 | -420685850 | no | no | yes |
| l136-zero-n0 | 209861 | 3080558061 | 161285762 | 7800306 | 162793392 | 153485456 | -1507630 | yes | no | no |
| l136-zero-n1 | 210151 | 3080897216 | 161303519 | 7819945 | 162813031 | 153483574 | -1509512 | yes | no | no |
| l136-zero-n102724 | 29999821 | 33606488865 | 1759502034 | 2025197089 | 2180190176 | -265695055 | -420688142 | no | no | yes |
| l137-a5-n0 | 209861 | 3080601112 | 161288016 | 7800306 | 162793392 | 153487710 | -1505376 | yes | no | no |
| l137-a5-n1 | 210151 | 3080897498 | 161303533 | 7819945 | 162813031 | 153483588 | -1509498 | yes | no | no |
| l137-a5-n102724 | 29999821 | 33621072659 | 1760265584 | 2025197089 | 2180190176 | -264931505 | -419924592 | no | no | yes |
| l137-zero-n0 | 209861 | 3080600001 | 161287958 | 7800306 | 162793392 | 153487652 | -1505434 | yes | no | no |
| l137-zero-n1 | 210151 | 3080896584 | 161303486 | 7819945 | 162813031 | 153483541 | -1509545 | yes | no | no |
| l137-zero-n102724 | 29999821 | 33621071575 | 1760265527 | 2025197089 | 2180190176 | -264931562 | -419924649 | no | no | yes |
| l65536-a5-n0 | 250967 | 3082743786 | 161400198 | 8092902 | 163085989 | 153307296 | -1685791 | yes | no | no |
| l65536-a5-n1 | 275773 | 3117769184 | 163233988 | 10173751 | 165166838 | 153060237 | -1932850 | yes | no | no |
| l65536-a5-n1199 | 29993361 | 45053140215 | 2358803152 | 2503030999 | 2658024086 | -144227847 | -299220934 | no | no | yes |
| l65536-zero-n0 | 250967 | 3082786299 | 161402424 | 8092902 | 163085989 | 153309522 | -1683565 | yes | no | no |
| l65536-zero-n1 | 275773 | 3117769413 | 163234000 | 10173751 | 165166838 | 153060249 | -1932838 | yes | no | no |
| l65536-zero-n1199 | 29993361 | 45053152670 | 2358803804 | 2503030999 | 2658024086 | -144227195 | -299220282 | no | no | yes |

| Input (keccak-bulk-) | GuestInput bytes | Canonical GuestInput SHA256 |
| --- | ---: | --- |
| l0-zero-n0 | 333769 | 112ea24aa045d9ff5c3c6253a8b121a7b2560f17b9fda75f7acc701d62088231 |
| l0-zero-n1 | 333769 | 4c914ca794aff5f42387184d766c00ac495076065a3b36ce402e6857cc7eb83c |
| l0-zero-n129523 | 333769 | f7256d487bc5ff3c041db7bc4edca98cf2d32bf0951552ef26afc61141cfb606 |
| l135-a5-n0 | 334011 | 27dc0ce15fdbf4d07d94f839ce9685549bf49ca96a7f313afbd32deef8b47b39 |
| l135-a5-n1 | 334011 | 4d1e89cd5aec821e87695916667456729200cb6d1962bcab23b44cb153d3d8e3 |
| l135-a5-n102724 | 334011 | 4ca2504eb76c9870b0d3e6594687304c0cce88ccb1c29b72ac840125bee47f41 |
| l135-zero-n0 | 334011 | aaea7e33908755280591705177b3f837e396d78d8cd252ebd2c7c2fc7a3c0b03 |
| l135-zero-n1 | 334011 | 999314ac74b08f91ce83c47b80a81e97709a8915af8e44f0eb982fe1946be862 |
| l135-zero-n102724 | 334011 | 2a79d08b21924134bfdf10dae9f00cc12d983b20c0a478527d4c1c145e7ecbf0 |
| l136-a5-n0 | 334011 | 3eaabcc9f4f658ad081e381a5c8c4486576d90161b090f6ab274761640a44932 |
| l136-a5-n1 | 334011 | 798e92caacdc0e4b112ab793b919da2ddf1d51c29aeecfdbcc46a8736962ebac |
| l136-a5-n102724 | 334011 | a58c1ee494036ea5f9b313f7f00dfd95260692dfaf5385a2cb4b62e037b56610 |
| l136-zero-n0 | 334011 | 79f93ee25744eb5cac53316a16eebb431d569f9b2168c3c99c50b2754d3ac564 |
| l136-zero-n1 | 334011 | 4c3f23f8750256b42e06b16b14a24bbe2020d7fc45bacf4554ea53434e430493 |
| l136-zero-n102724 | 334011 | 20aff0ff97c4b1a8a348f7b98fbb3084e4e05de6e9f060db50e568b6c33bf226 |
| l137-a5-n0 | 334011 | 755b3937955228227e51a6c2f3b723fcc3df6f5b6feca1bcf60e3cb87bf723df |
| l137-a5-n1 | 334011 | ad5f0a4e16baa6ad658f2e31e529afd92854e7c437c83de4e104c2e2399c90fd |
| l137-a5-n102724 | 334011 | a841e88c8ec14ec3eabb7990a5abadddbf6eaca70fbb6fb8bea9e0b4c549d929 |
| l137-zero-n0 | 334011 | a3ecc84cf2cc00aa87acce5a6dd3d100b203f43e64f2e58915aea88e612592a3 |
| l137-zero-n1 | 334011 | 37bff5efb676526678a349db6eeb80af5e23e9dcaa1d8b314952e995828523ad |
| l137-zero-n102724 | 334011 | b170271f69f9b0fab5097853677478d080eefc0d14dc9d55a38a1f447dbe9b90 |
| l65536-a5-n0 | 334143 | 9c2166d1e51f8f4dd883be667efd25f4b3f401e1c95614b5f2cca132a84f4e2b |
| l65536-a5-n1 | 334143 | e929d1ca9ee6b394cd756e147c34b312f2edf4be631aa9da141f8075cec1d265 |
| l65536-a5-n1199 | 334143 | 4c8a8a715e288c042e19f6a5729ce145c2eff6b6b1821a3a42f6b1b32db2ba15 |
| l65536-zero-n0 | 334143 | e4299b59c88ca18e61d7449c59c2992defc1f15eff0086af8618e8f77dec2e91 |
| l65536-zero-n1 | 334143 | 394c43861db2f19b8d373fc581611acd29b94774aace828370db144ed37ef7a0 |
| l65536-zero-n1199 | 334143 | e41897f4412573c4c55327729258cdc4bbda6544aa5abe34dd6098b02c486b8d |

## Signed diagnostic boundary

The 51 signed whole-input diagnostics comprise nine each of own-zero→single, own-zero→full and own-single→full, 12 a5-minus-zero tails, and 12 padding edges (135→136 and 136→137 × two patterns × three levels). Each retains exact signed scaled estimate differences divided by 10^18, signed actual differences, and upward-rounded positive deficits; negative differences are not clamped. All 27 repetition actual differences are positive and covered. Tail actual differences split six positive/six negative; padding differences split ten positive/two negative. Both estimates leave six tail and six padding deficits. The retained machine-readable analysis and independent raw audit preserve every pair and rational value.

| Equality flag | True / 51 | Meaning |
| --- | ---: | --- |
| R | 24 | Equal repetitions per user |
| Hashed L | 39 | Hashed message length, not GuestInput size/content |
| Tail pattern | 39 | Selected zero/a5 tail |
| Memory words | 51 | Allocated word count |
| Permutations/call feature | 45 | Frozen model feature, including empty shortcut |
| Charged gas | 24 | Whole-block charged gas |
| Full signed envelope | 51 | Both signed user transactions |
| Owner-selected storage support | 51 | Both authenticated owners mapped to empty selected storage sets |

Equal memory words do not imply equal padding or permutation feature: 135→136 crosses a Keccak rate boundary within the same EVM word count. Equal gas/envelopes for 136→137 do not imply equal messages or complete witness work. Empty selected support includes two owners; it excludes account/code/global authentication. These comparisons do not isolate opcode cost, certify a component bound, exhaust cache effects or establish IID confidence/global coverage.

## Qualification and retained evidence

The final producer source is `891c3ba87d08bb4ea958ea03598570b7d564ffe3`. Source review passed before qualification. Source closure binds 178 current files and exact committed blobs. The source inventory is `e04e33c016dac90a74b59b9021767809a300324a342f1702ba2d7b6d16970571`; source closure is `11277a59a48f3e3684e581ea754f6a50f36203d8ff25ee161873d61d8a6cebd2`.

The author native3 comprises two original positives and one native-successful prefix-bit80 mutant that fails original intent and is excluded. Canonical qualification uses 25 fresh native invocations plus two exact original positive bridges, 27 fresh original quotes and 54 signatures. Original bridges retain original source/command/timing evidence. Independent decoded byte intent, gas/counter retention, domain/semantic/authentication negatives, both-owner full+1, account/code/storage witness closure and full/minimized native execution-result equality bind all canonical inputs. Native exports gas/status/routes rather than per-call message bytes, digest or final memory; private oracle commitments are not protocol roots. Whole-GuestInput/native/quote/public-output binding does not replace that missing native observability.

All actuals use 600s/6144MiB process-tree sampled-PSS limits; observed maximum elapsed time is 34.9937918112s and peak sampled PSS is 4,544,495,616 bytes. Qualification native/quote caps are 180s/1800s with 6144MiB sampled PSS. Retained limits also distinguish the 256MiB runtime disk floor, 320MiB reserve, 192MiB output and 16MiB logs. There were no actual retries or measurement reruns.

The first private shared-memory absence receipt inspected a sandbox mount (device69, namespace mnt4026532513), not the actual runtime mount, and is preserved as `host-shared-memory-audit.sandbox-original.json`. It is excluded as host-cleanup authority. The corrected read-only receipt binds device27/namespace mnt4026531841 to all 108 original runtime resource names; the independent real-host audit checks each name with lstat and finds all absent. Correction records and the original script are retained. No resource deletion, source/price change or measurement rerun accompanied this private harness correction.

| Identity / retained basename | SHA256 |
| --- | --- |
| Frozen elf | b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034 |
| Frozen model | 4cb6c65c6ad28e25e1f30639803958f9555e43d51e6d35d2f1945b0757ce1ac7 |
| Frozen native | ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec |
| Frozen program_manifest | 8d5b4a482d26e3bdca94c5e66b9de7b33dc57bcfbaa9b9c24140bf030c1f93a9 |
| Frozen quote | c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e |
| Frozen sp1 | 1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b |
| Semantic model ID | c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef |
| analysis.json | b702bc9104a4bfc5ba1b4b64ea372d25c07387635ad97aebb687eae053f9d644 |
| gas-budget-report.json | 9da34247b05cc04ad5784de55f6edf62232924356815b6308069537d5d6b2845 |
| config.json | 229327199493d3ee54d588131ca2c671c6b0688703d4dacb4404f2696d31f24c |
| policy-freeze.json | 6fa4e15a3e78dee2f2c9158b9c8e99a86fc81db9e6c45a1f4eaf9c0ca8d41e4c |
| qualified-inputs-index.json | 92831ec37087243ac6c72e73cddcc816c1b551345371036a46d0cd894709a6f7 |
| verification.json | fa4d4ae9ea225a73e5d13ca4ff07a557fc7407edd6d7914a2cd23fc28028463b |
| artifact-manifest.json | 3d12092e1010695d939fbe2ed49fadf295714495d7cb3bf19ac9783fb4bf3a84 |
| final-artifact-manifest.json | 6c352569c7c31de1a5bc1ddc00a045ea9de07c669ba887f9b3a147728fe6f5bc |
| actual-audit.json | 0343d3c6afc355aeacb92d8dd6989db84ebc8b9d5660e227c187d90204bebd23 |
| raw-final-closure.json | e2c6fe17df595a86b6f10b58bc6ee77ec0a3b9a5a4eae978dfbcf452fd70a5a0 |
| raw-strong-closure.json | 69b8a07f5c0e83a6528496e8c749cb20c0741a5fec6ddd6d691e9f738e4f59b5 |
| host-shared-memory-audit.json | a15e8c14a507c7ea85f5673ac064839ea4968a8fe3b9246b8886b493b685f1bc |
| host-context-independent-audit.json | 63237de443f7f0cbdad691f26254ecdcf60e672bfada61e2bf49b027f49d56da |
| keccak-bulk-evidence-20261104.json | 8436a9e0a426c99bd7f4aadc6e4b975765492418f5b407d98b0afa7f24d70357 |

The original qualification manifest has 577 members, independently verified unchanged; the final manifest has 584. The verified private recovery archive `keccak-bulk-evidence-20261104.tar.gz` is SHA256 `75345379c7eab5558b0be5b8d2397c116c34d0a9b4714b9c49d35769adcd01b2`, 3,452,054 compressed bytes, 1002 regular members, 451 restorable duplicate aliases and four external runtime artifacts. Its receipt verifies streamed member hashes without extraction. Basenames and hash scopes identify private retained evidence, not publicly shipped artifacts; runtime artifacts require external hash-matching recovery. This is not a standalone public Atlas replay bundle.

The documentation check independently parses original raw27 SDK reports, original quotes, native capacity receipts, canonical input bytes and all 51 exact scaled diagnostics, recalculating both literal flags, thresholds, nine full+1 capacities and eight equality totals. It does not rerun native, quotes or SP1. It also rehashes all 178 current source files and exact committed blobs, all original577/final584 manifest members and the compressed archive. Source, tool/model/SDK/ELF and input identities remain frozen. No Rust build is claimed for this documentation increment.

Prior [persistent236](2026-11-03-storage-warm-rollback.md), [transient8](2026-11-03-transient-storage.md), [pairing18](2026-11-03-bn254-pairing-dense.md), [defaultSHA18](2026-11-04-sha256-bulk.md), [linearSHA18](2026-11-04-sha256-linear-regime.md), [LOG30](2026-11-04-log-receipt-bulk.md), [MCOPY12](2026-11-04-mcopy-bulk.md) and [division32](2026-11-04-division-density.md) remain unchanged, separately counted cohorts. No new union, repricing, protocol/guest/SDK/ELF/pin change or natural-usability/production certification is asserted. Broader lengths/messages, failures, nested work, heterogeneous packing/order and cache interactions remain open alongside the retained incremental deficits. This increment remains draft/do-not-merge; no merge or deployment is authorized by this record.
