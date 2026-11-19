# Many-key warm dirty storage

All 16 new execute-only observations completed once, with no setup or SP1 failure: eight own-R0 controls and eight generated full points. Nine literal node gaps remain: six controls and full REVERT/K1, K16 and K256 (110,649,336 / 109,404,126 / 81,737,769 PGU). Maximum all-point gap is 153,248,333 PGU; complete-proposal gaps and >2B actual-minus-node gaps at <=30M charged gas are zero. Maximum actual is 893,287,462 PGU. Prices stay unchanged; these lower-priority deficits remain explicit TODOs.

This panel combines wider storage footprints with repeated warm dirty pairs after the [few-long-root-REVERT panel](2026-11-03-storage-long-revert.md). Its finite evidence does not close nested/caught rollback or general key/cache/hash distributions. See the [coverage inventory](zkgas-workload-coverage.md).

## Programs and gas limits

The 16 inputs cross STOP/root REVERT, K=1/16/256/1024 keys per account and own-R0/generated-full levels. Four signed top-level CALL users each visit a distinct predeployed contract once with shared runtime and empty calldata; the unused primary is STOP. For account i, B_i=(i+1)*65536, read-only raw phase slot 0 contains B_i+1, and raw body slots B_i+1..B_i+K contain 1. All others are absent; initialized entries total 4(K+1). Own-R0/full retain identical complete maps and the full point's signed envelope.

The body reads phase0, seeds **all** K keys 1->5 with checks, then visits each key for R warm dirty 5->7->5 pairs in 64-pair batches plus exact tail, then cleans **all** K keys 5->1 with checks. Executed round counters build marker K*R, with final phase/marker checks; memory remains six words (192 bytes). There is no nested frame, CALL wrapper or per-pair SLOAD. R0 still performs seed/cleanup and is not empty. Complete contract maps and authenticated roots restore under both terminals; nonces, fees and Anchor change whole-world state. STOP users succeed; REVERT users fail as intended while Anchor succeeds.

Storage gas/user is 2200+K*(5600+200R), plus intrinsic 21000, ordinary executed-opcode gas and memory expansion 18. Preterminal refund is 2800K; STOP applies min(refund,gross//5), while REVERT discards effective/applied refund. Literal user access pairs are 4(K+1), write pairs 4K: phase0 never becomes a write. Attempted work survives rollback. Original node and complete-proposal selectors match these sets plus fixed **observed** system allowances (+4 access/+4 write, 12363 ordinary dispatch); those allowances are not independently derived Anchor opcode totals. Aggregate/unique collection conformance is not proof of per-opcode occurrence charging.

Every block retains a 31M header, <=30M charged gas including Anchor 167355, <=15M/user and EIP-7825 caps, gross+2301 full envelope, every ordered admission prefix/SSTORE sentry, <=8192 targets and supported runtime size. Full means maximal uniform R for this finite program. Seven full points are near-full (>=29.7M); STOP/K1024 R12 charges 28,858,799 (96.195997%). R13 charges only 29,573,143 but fails its own last gross-envelope prefix by 413305 gas. Sufficient-envelope R+1 execution precedes applying candidate constraints. No cap waiver, 30M-header substitution or arbitrary block-maximality claim follows.

| Mode | K | Full R | Gross/user | Applied refund/user | Charged/user | Full signed envelope/user |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| revert | 1 | 33942 | 7458113 | 0 | 7458113 | 7460414 |
| revert | 16 | 2095 | 7456882 | 0 | 7456882 | 7459183 |
| revert | 256 | 104 | 7407554 | 0 | 7407554 | 7409855 |
| revert | 1024 | 5 | 7403458 | 0 | 7403458 | 7405759 |
| stop | 1 | 33955 | 7460941 | 2800 | 7458141 | 7463242 |
| stop | 16 | 2108 | 7502220 | 44800 | 7457420 | 7504521 |
| stop | 256 | 117 | 8133052 | 716800 | 7416252 | 8135353 |
| stop | 1024 | 12 | 8966076 | 1793215 | 7172861 | 8968377 |

## Literal same-proposal results

IDs are `storage-dirty-keys-{mode}-k{K}-r{R}`. Actual PGU is raw SDK6.3 gas*10//191, using exact integer conversion. Node is the sum of literal block ceilings; complete proposal is separate. Positive gaps are max(actual-estimate,0); the literal denominator is retained. Full node/actual ratios are 0.87613244–1.15601617 and complete-proposal/actual ratios 1.05039351–1.63910465.

| Mode | K | R | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive proposal gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| revert | 1 | 0 | 285783 | 162090781 | 28844076 | 184509307 | 133246705 | 0 |
| revert | 1 | 33942 | 29999807 | 893287462 | 782638126 | 938303356 | 110649336 | 0 |
| revert | 16 | 0 | 652803 | 165611098 | 35953832 | 192995814 | 129657266 | 0 |
| revert | 16 | 2095 | 29994883 | 887919791 | 778515665 | 935557648 | 109404126 | 0 |
| revert | 256 | 0 | 6525123 | 222376049 | 149709917 | 329524715 | 72666132 | 0 |
| revert | 256 | 104 | 29797571 | 790955485 | 709217716 | 889032515 | 81737769 | 0 |
| revert | 1024 | 0 | 25316547 | 407382747 | 513729388 | 766971835 | 0 | 0 |
| revert | 1024 | 5 | 29781187 | 513215495 | 587971459 | 841213906 | 0 | 0 |
| stop | 1 | 0 | 274559 | 162091078 | 8842745 | 164507975 | 153248333 | 0 |
| stop | 1 | 33955 | 29999919 | 681044637 | 762940265 | 918605495 | 0 | 0 |
| stop | 16 | 0 | 555695 | 165488857 | 15952500 | 172994483 | 149536357 | 0 |
| stop | 16 | 2108 | 29997035 | 681224103 | 763120266 | 920162249 | 0 | 0 |
| stop | 256 | 0 | 5253551 | 220534102 | 129708585 | 309523384 | 90825517 | 0 |
| stop | 256 | 117 | 29832363 | 672198070 | 762661713 | 942476512 | 0 | 0 |
| stop | 1024 | 0 | 20286691 | 399555646 | 493728056 | 746970503 | 0 | 0 |
| stop | 1024 | 12 | 28858799 | 581228051 | 671909027 | 925151474 | 0 | 0 |

## Matched diagnostics

All 28 diagnostics below retain exact pre-rounding scaled differences. Scaled quotes use 10^18 units per PGU; no rounded-ceiling subtraction replaces them. Own-zero rows are full−own-R0; terminal rows are REVERT−STOP at fixed K/level; adjacent rows are larger−smaller K at fixed mode/level. Mode/K comparisons can change R, gas, refunds, envelopes and witness support. These are whole-proposal diagnostics, not isolated tariffs, per-operation actual allocation or a denominator replacement.

| Own-zero mode/K | Actual increment PGU | Node increment scaled | Proposal increment scaled | Node/proposal increment covers actual |
| --- | ---: | ---: | ---: | --- |
| revert/K1 | 731196681 | 753794049293251171853304373 | 753794049293251171853304373 | True/True |
| revert/K1024 | 105832748 | 74242071036301499003494400 | 74242071036301499003494400 | False/False |
| revert/K16 | 722308693 | 742561833130677165197219737 | 742561833130677165197219737 | True/True |
| revert/K256 | 568579436 | 559507799720146509764321497 | 559507799720146509764321497 | False/False |
| stop/K1 | 518953559 | 754097520106885686473970685 | 754097520106885686473970685 | True/True |
| stop/K1024 | 181672405 | 178180970487123597608386560 | 178180970487123597608386560 | False/False |
| stop/K16 | 515735246 | 747167766148829399127880729 | 747167766148829399127880729 | True/True |
| stop/K256 | 451663968 | 632953128010582252654897369 | 632953128010582252654897369 | True/True |

| Terminal REVERT−STOP / level | Actual difference PGU | Node difference scaled | Proposal difference scaled | Same R / same charged gas |
| --- | ---: | ---: | ---: | --- |
| K1 / zero | -297 | 20001331754694528277460392 | 20001331754694528277460392 | True/False |
| K1 / full | 212242825 | 19697860941060013656794080 | 19697860941060013656794080 | False/False |
| K1024 / zero | 7827101 | 20001331754694528277460392 | 20001331754694528277460392 | True/False |
| K1024 / full | -68012556 | -83937567696127570327431768 | -83937567696127570327431768 | False/False |
| K16 / zero | 122241 | 20001331754694528277460392 | 20001331754694528277460392 | True/False |
| K16 / full | 206695688 | 15395398736542294346799400 | 15395398736542294346799400 | False/False |
| K256 / zero | 1841947 | 20001331754694528277460392 | 20001331754694528277460392 | True/False |
| K256 / full | 118757415 | -53443996535741214613115480 | -53443996535741214613115480 | False/False |

| Adjacent K / level | Actual difference PGU | Node difference scaled | Proposal difference scaled | Same R / same charged gas |
| --- | ---: | ---: | ---: | --- |
| revert K1→16 / zero | 3520317 | 7109755305141449073731340 | 8486507305141449073731340 | True/False |
| revert K16→256 / zero | 56764951 | 113756084882263185179701440 | 136528900882263185179701440 | True/False |
| revert K256→1024 / zero | 185006698 | 364019471623242192575044608 | 437447119623242192575044608 | True/False |
| revert K1→16 / full | -5367671 | -4122460857432557582353296 | -2745708857432557582353296 | False/False |
| revert K16→256 / full | -96964306 | -69297948528267470253196800 | -46525132528267470253196800 | False/False |
| revert K256→1024 / full | -277739990 | -121246257060602818185782489 | -47818609060602818185782489 | False/False |
| stop K1→16 / zero | 3397779 | 7109755305141449073731340 | 8486507305141449073731340 | True/False |
| stop K16→256 / zero | 55045245 | 113756084882263185179701440 | 136528900882263185179701440 | True/False |
| stop K256→1024 / zero | 179021544 | 364019471623242192575044608 | 437447119623242192575044608 | True/False |
| stop K1→16 / full | 179466 | 180001347085161727641384 | 1556753347085161727641384 | False/False |
| stop K16→256 / full | -9026033 | -458553255983961293281920 | 22314262744016038706718080 | False/False |
| stop K256→1024 / full | -90970019 | -90752685900216462471466201 | -17325037900216462471466201 | False/False |

Absolute gaps and uncovered increments remain visible. Differences between programs do not identify which opcode or terminal caused the actual difference. Ordinary usability also remains a constraint: conservative quotes can reject useful work despite covering its actual. Next priorities couple many-key warm work with nested/caught rollback, then broader key/cache/hash-path distributions and mixed workloads; this root-only panel supplies no nested-closure claim.

## Tools, qualification and retained evidence

See the [producer](../../bin/zkgas/bench/cases/storage_dirty_keys.py), [tool README](../../bin/zkgas/bench/README.md#many-key-warm-dirty-writes) and [binding specification](../superpowers/specs/2026-11-03-storage-dirty-keys.md). Model, native/quote executables, host/guest/GuestInput, dependencies, pins, SDK and ELF remain unchanged. No price fitting, Rust/guest build or measurement retry occurred.

Retained source checks passed: focused seven tests (six executed, one optional-native skip), separate one-test native lane with four specimens, and old mutation 6 / journal 11 / long 8 lanes (one optional-native skip each). Independent source review passed. Independent all 16 qualification then checked 64 signed users, receipts/refunds/gas/prefix/sentry, actual byte trajectories/counters/marker, complete original/final world maps and authenticated per-account roots, canonical conservative mutation witnesses, original real quotes and attempted counters before freeze. Boundary 0/1/63/64/65 and native-accepted dirty9 semantic negatives are retained. A private auditor initially misselected PUSH1 instead of actual PUSH3 in a post16 mutant; the failed check and correction remain retained. Canonical native, quote and SP1 measurements were never rerun.

All 16 sequential SP1 executions used 6GiB sampled process-tree PSS and 600s bounds; native was bounded at 180s and quotes at 1800s/max 2 concurrent. Policy binds a 256MiB free floor plus 320MiB retention reserve (576MiB total), 192MiB output and 16MiB logs. Peak sampled PSS was 4571155456 bytes. Independent raw16/diagnostic28 final audit passed exact SDK conversion, artifact/input/public-output/original-quote joins, raw literal sums/full totals, resources and guards. This auditor launched no measurements. There were 16 successful unique attempts and 64 unique owned shared-memory names.

The ELF declared source revision is `4580faf7dc009714b310fd625b5810667019d683`; this is not a build attestation. These are execute-only finite observations, not a genuine generated proof, GPU throughput measurement, global bound, production SDK/ELF certification or natural historical admission result.

| Identity | SHA256 / version |
| --- | --- |
| Producer source | `0f99d9ea82af14e22cdbdbb96f45f615a34a210f` |
| Model | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Retained ELF / SDK | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` / `6.3.0` |
| Native executable | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Quote executable | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| Frozen config | `fe337833f60b7479a8cf91fe662b5442955c65861404834cdb28eb001ee9e96d` |
| Pre-actual policy | `b80cd1077f9475751d93ea2fe3080a50c5fb1acf75f769a60e11d1cb0757bb60` |
| Input/quote qualification | `a9130c2874051c1fb9a105d529ec477b0f911ef0d395fa0c84519de4005e771b` |
| Counter retention audit | `440f185f6e12a905f862450cca40bd89323f91555cfc0c924bd49b4de4c92418` |
| Actual analysis | `61f24d10dc5f7c374c56ab5ded92746adec32723e20f4b5a3b34462c81db0250` |
| Gas-budget report | `09f26224e79c9b153fb16fe351f81ecc62946032215d0931406c6157ad25a0c4` |
| Independent raw final audit | `53d8ad4e4f616479e593e1f346e9ea4af2dd83b6d8932bf2a1ccf232306b27df` |
| Independent diagnostic closure | `413f336dd208e41a951d045c470914d2261a3b7e356eaee29fb9c4e22aa1cc3d` |
| Independent final report | `a1e5b03965ecb4395ab343809a7ef5284552eb32caea45ae3ecfcdbfe771eb74` |
| Independent 244-file manifest | `c811a7ae90a6fde9ba93fd0dd493e16a19a22bce325dfbe3964dc9b8d6c28dd6` |
| Host shared-memory receipt | `aff604fa532b1ed2fd8819ac41c2158a51266f2c4b726b4b532e7247c442e2c8` |
| Six-panel storage-cohort summary | `6e63e389a51e57135490c8845bd626f4a3d02499f5363024610a9c5ad9557453` |

A separate host-context receipt confirms all 64 exact owned shared-memory names absent; the raw actual audit alone does not establish host namespace absence. The independently rejoined six-panel storage subset has 148 distinct canonical inputs, 64 generated full selectors but 63 near-full inputs, 13 full node gaps with maximum 110,649,336 PGU, maximum all-point gap 164,072,990 PGU, no >2B/30M gap and no complete-proposal gap. All six source-analysis hashes, config model/ELF/SP1 identities and canonical-ID deduplication were checked. Reused references are excluded. The preceding 132/56 snapshot stays historical; neither cohort is an all-research count, IID confidence bound or global coverage percentage.

Retained raw requests/canonical inputs, original quotes and invocation provenance, native diagnostics, all 16 successful attempts, controls, private helper versions/dependencies, source/artifact identities, failed private-auditor check and correction, final audits and cleanup receipts are recoverable. The verified local archive is SHA256 `3d45e466a9610fa369ed6f8b0b70694599789777a15329c7148bbf3b35b258ff`, 9115097 compressed bytes, 494 regular members carrying 32762988 payload bytes, 96 restorable duplicate aliases and four external runtime artifacts identified by hash. Streamed member hashes were verified without extraction; archive hash/size were separately checked. This is private recovery evidence, not Atlas fixture publication or a standalone auditor runner. No merge, deployment or on-chain action is part of this panel.
