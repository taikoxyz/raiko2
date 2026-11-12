# Full-width ADDMOD/MULMOD density: finite modular16 panel

All 16 original execute-only actuals completed once, without external failure or implicit retry. All eight full inputs are near-full and their literal node and complete-proposal quotes cover the measured actual PGU. Eight own-zero controls retain node deficits, maximum 153,485,120 PGU. No actual-minus-node gap exceeds 2B PGU within the frozen 30M charged-gas budget. The maximum actual is 2,375,928,982 PGU; that maximum is distinct from the maximum shortfall. The current ADDMOD 1100 / MULMOD 1600 floors, model and runtime artifacts remain unchanged.

All eight own-zero-to-full increments are covered by both exact scaled estimates. All 12 modulus-minus-limb1 contrasts remain uncovered, with maximum upward-rounded increment deficit 775,441,834 PGU. These signed whole-proposal differences remain visible even though full absolute quotes cover. Positive absolute margins neither prove nor exclude a startup-only explanation. No per-operation actual-cost, causal component or global bound follows.

## Inputs, byte intent and qualification

The 16 inputs are ADDMOD (0x08) / MULMOD (0x09) × modulus limb1 = 3, limb2 = 2^64+1, limb3 = 2^128+1, limb4 = 2^192+1 × own-zero/full. Unsigned a = 2^256−2 and b = 2^256−3 use an untruncated 257-bit sum or 512-bit product before reduction. In all eight domains the correct result differs from premature 256-bit wrapping before modulo. This selection does not establish worst operands or quotient-correction behavior.

One block has two EIP-1559 users from the same signer, nonces 0/1, two distinct authenticated code owners tagged 0/1, empty selected persistent storage and empty calldata. Each retains stack [m,b,a], executes one checked warmup even at R0, then repeats a fixed 64-site DUP3/DUP3/DUP3/target/POP body. Target operations per user are 64R+1. Counter, owner tag, all three original base words, empty final stack and STOP are guarded independently. Arithmetic operands repeat; tags do not make target inputs unique. There are no nested calls, logs, creates, refunds or REVERTs in the user kernel.

The public host interpreter caches only the decoded pure body after proving ordered operands/results, gas/counts and complete stack preservation. Direct/cache R0/1/2 comparisons for both owners in all eight domains and mutation negatives qualify that bounded shortcut; surrounding instructions execute normally. Actual guest execution runs the entire kernel. The separate canonical verifier derives intent from actual bytes and integer arithmetic without importing the producer or public VM as authority.

The retained source smoke ran exactly three native specimens once: two originals and an accepted MULMOD/limb1/zero modulus3→5 mutant with the same correct result 2. The unchanged original intent rejects that mutant, which is excluded from canonical inputs and actuals. Native exports gas/status/routes and full/minimized execution-result equality plus same-GuestInput stage binding; it does not export arithmetic operands, results or stacks. Canonical qualification then used 14 fresh guarded native runs plus two exact original positive bridges, 16 fresh original quotes and 32 independently authenticated signatures, including account/code/selected-empty-storage/witness/roots/receipt binding. All byte/math/counter/tag/gas/full+1 gates passed, as did 21 semantic and nine authentication negatives.

A private v1 negative locator stopped with StopIteration before external native or quote calls. Its width-independent decoded replacement passed all 21 negatives in v2; v3 passed 21 again with stronger decoded-width context. This is a retained private-checker correction, not a v2 failure, re-export, external retry or invented run.

## Literal whole-proposal observations

Raw SDK ExecutionReport.gas is normalized as raw_gas × 10 // 191. NodeKnown is the literal block-known PGU sum; Complete is the complete-proposal PGU quote. Raw Unzen scores are separate units. Signed gaps below are actual minus the respective literal quote; flags N/P are true exactly when that gap is positive, including controls. Input SHA256 binds canonical bincode.

| Operation | Modulus | Level | Charged EVM gas | Raw SDK gas | Actual PGU | NodeKnown PGU | Complete PGU | Signed node gap | Signed complete gap | N/P deficit | Canonical input SHA256 |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| ADDMOD | limb1 | zero | 209827 | 3081251542 | 161322070 | 7839504 | 162832591 | +153482566 | -1510521 | true/false | `82eafe5167bc1d08278ee622aadfc71d3efaa424b904631c2e8dc929a265e79f` |
| ADDMOD | limb1 | full | 29999525 | 20999880577 | 1099470187 | 2359652080 | 2514645167 | -1260181893 | -1415174980 | false/false | `c93802f1fefa4b7bc197aa53bd3f73cdedb414c6a3612d6f975017019e0a1e10` |
| ADDMOD | limb2 | zero | 209827 | 3081292932 | 161324237 | 7839504 | 162832591 | +153484733 | -1508354 | true/false | `883ed4f4fa71f422a7cd46229e79e5361e92cc88238d39b1e5943a520f699ce8` |
| ADDMOD | limb2 | full | 29999525 | 23883786673 | 1250460035 | 2359652080 | 2514645167 | -1109192045 | -1264185132 | false/false | `6effc13f10ac3dccb73db9eb83f2f6455126239abb560e1c2a37c2203dab5139` |
| ADDMOD | limb3 | zero | 209827 | 3081261704 | 161322602 | 7839504 | 162832591 | +153483098 | -1509989 | true/false | `5695b04ecc612496d7d84a5374ed7d06ce2f0d6f88a0fb4b89887d5573835018` |
| ADDMOD | limb3 | full | 29999525 | 31446830927 | 1646430938 | 2359652080 | 2514645167 | -713221142 | -868214229 | false/false | `e60e4855a8be16aa7ee03a237aca919c11729a59dce721f57a2f008c65de8299` |
| ADDMOD | limb4 | zero | 209827 | 3081257134 | 161322363 | 7839504 | 162832591 | +153482859 | -1510228 | true/false | `dd086c065d0a85e9f16ff0baec4ce6bd3d8c7651e1b54fafd382e6c91caf7500` |
| ADDMOD | limb4 | full | 29999525 | 28752543599 | 1505368774 | 2359652080 | 2514645167 | -854283306 | -1009276393 | false/false | `1d3970da420bbc14342c962c09662927ea0144c69123d14f8ba029803f7fac6c` |
| MULMOD | limb1 | zero | 209827 | 3081259902 | 161322507 | 7840504 | 162833591 | +153482003 | -1511084 | true/false | `f3d6e8cad6da43a5e3ba2082bb90a50d9c27f8c894367f3a9230df02e6f6ebe7` |
| MULMOD | limb1 | full | 29999525 | 30569304528 | 1600487148 | 3109669080 | 3264662167 | -1509181932 | -1664175019 | false/false | `6f2db656706c8371fce0ac73074264da4156f6dbb6d7a1f317fb186b7b6cf033` |
| MULMOD | limb2 | zero | 209827 | 3081267079 | 161322883 | 7840504 | 162833591 | +153482379 | -1510708 | true/false | `b6e1cf02bb4c83307a6df4c2ee7424faa98339c41c75bb5768732028348493c9` |
| MULMOD | limb2 | full | 29999525 | 34851260760 | 1824673338 | 3109669080 | 3264662167 | -1284995742 | -1439988829 | false/false | `b7b7288e39a08a1ffddc58000dd72f0532f5bc7a9ddec39c530695ee79898872` |
| MULMOD | limb3 | zero | 209827 | 3081319421 | 161325624 | 7840504 | 162833591 | +153485120 | -1507967 | true/false | `e4f9b30c7b3f188a1e843812f15182d9db38f20092736619ebfcdcdabca4e272` |
| MULMOD | limb3 | full | 29999525 | 45380243569 | 2375928982 | 3109669080 | 3264662167 | -733740098 | -888733185 | false/false | `ec82b55de9ea070c43be8ccadcd80e4d3a4dc07d83ffb927b44238df27137307` |
| MULMOD | limb4 | zero | 209827 | 3081317022 | 161325498 | 7840504 | 162833591 | +153484994 | -1508093 | true/false | `2d43016a83a812147f6f7e3b3d5b592729361bb38f38e3c4ca27d8473005a2ff` |
| MULMOD | limb4 | full | 29999525 | 42906076085 | 2246391418 | 3109669080 | 3264662167 | -863277662 | -1018270749 | false/false | `9e47a237645690f95466d0260b036b204a62fddd79f606b043afd3c8dbc4b56f` |

Full literal node/actual ratios range from 1.30882240318 to 2.14617195437; complete/actual ratios range from 1.37405713375 to 2.28714265901. Ratios describe these eight full observations, separately from absolute deficits and signed increments. MULMOD/limb3/full has the largest full actual/node ratio, 0.764045601277; controls rank separately and retain their absolute node deficits.

## Capacity from actual decoded bytes

Every runtime is 816 bytes. The canonical zero user gross/charged gas is 21,236 and each repetition adds 1,271: 64 pure sites cost 1,216, with the remaining loop overhead derived independently from bytes. Intrinsic gas is 21,000, expansion to two memory words costs 6 and refunds are zero. Both zero/full signed transaction objects use the same full envelope 14,917,085. The private preparatory layout with base 21,209 is not this canonical program.

Full means maximum uniform R under all constraints; it is not global block packing. All eight fulls exceed the 99% near-full threshold. The fixture 15,000,000 transaction envelope, EIP-7825 envelope 16,777,216, header-prefix admission under the 31,000,000 header and whole-block charged budget 30,000,000 are distinct checks. The frozen Anchor costs 167,355. Both owners execute at full+1 under a diagnostic execution envelope; the policy envelope below is derived separately.

| Operation | Modulus | Full R | Targets/user | Full user gas | Full envelope | Full block gas | Full+1 R | Full+1 user gas (both owners) | Candidate envelope | Full+1 block gas | Limiting constraint |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ADDMOD | limb1 | 11719 | 750017 | 14916085 | 14917085 | 29999525 | 11720 | 14917356 | 14918356 | 30002067 | charged block >30M |
| ADDMOD | limb2 | 11719 | 750017 | 14916085 | 14917085 | 29999525 | 11720 | 14917356 | 14918356 | 30002067 | charged block >30M |
| ADDMOD | limb3 | 11719 | 750017 | 14916085 | 14917085 | 29999525 | 11720 | 14917356 | 14918356 | 30002067 | charged block >30M |
| ADDMOD | limb4 | 11719 | 750017 | 14916085 | 14917085 | 29999525 | 11720 | 14917356 | 14918356 | 30002067 | charged block >30M |
| MULMOD | limb1 | 11719 | 750017 | 14916085 | 14917085 | 29999525 | 11720 | 14917356 | 14918356 | 30002067 | charged block >30M |
| MULMOD | limb2 | 11719 | 750017 | 14916085 | 14917085 | 29999525 | 11720 | 14917356 | 14918356 | 30002067 | charged block >30M |
| MULMOD | limb3 | 11719 | 750017 | 14916085 | 14917085 | 29999525 | 11720 | 14917356 | 14918356 | 30002067 | charged block >30M |
| MULMOD | limb4 | 11719 | 750017 | 14916085 | 14917085 | 29999525 | 11720 | 14917356 | 14918356 | 30002067 | charged block >30M |

At full+1 both owners retain 750,081 targets, the correct result, counter 11,720, tag/base words and empty-stack STOP. All other capacity constraints pass while charged-block budget fails at 30,002,067. The header remaining before full users is 30,832,645 and 15,916,560, each covering the 14,917,085 envelope.

## Twenty signed exact-scaled diagnostics

The eight flags, in order, are same operation / operand a / operand b / modulus / R / charged gas / full signed transaction objects / authenticated selected two-owner storage support. Their true counts are 20/20/20/8/12/12/20/20. Each bit string below uses that order. Equal full transaction objects and selected empty storage support do not establish equal runtime, GuestInput, accounts, code or global witness; every pair differs in runtime and canonical input.

Scale S = 10^18. For every row the two exact estimate increments are identical, so Δestimate_scaled below represents both node and complete-proposal estimates. Signed deficit_scaled = Δactual×S−Δestimate_scaled; retaining this signed integer avoids rounding away negative or fractional results. Divide by S for the exact PGU deficit. Positive-gap ceiling is max(0, ceil(deficit_scaled/S)); covered requires deficit_scaled ≤ 0. These are whole-input contrasts, not isolated arithmetic-cost measurements.

| Contrast (to minus from) | Δactual PGU | Δestimate_scaled (both) | Signed deficit_scaled (both) | Positive-gap ceiling PGU | Covered N/P | Eight flags |
| --- | ---: | ---: | ---: | ---: | --- | --- |
| ADDMOD limb1 full−own zero | +938148117 | +2351812576154650286743152012 | -1413664459154650286743152012 | 0 | true/true | `11110011` |
| ADDMOD limb2 full−own zero | +1089135798 | +2351812576154650286743152012 | -1262676778154650286743152012 | 0 | true/true | `11110011` |
| ADDMOD limb3 full−own zero | +1485108336 | +2351812576154650286743152012 | -866704240154650286743152012 | 0 | true/true | `11110011` |
| ADDMOD limb4 full−own zero | +1344046411 | +2351812576154650286743152012 | -1007766165154650286743152012 | 0 | true/true | `11110011` |
| MULMOD limb1 full−own zero | +1439164641 | +3101828576154650286743152012 | -1662663935154650286743152012 | 0 | true/true | `11110011` |
| MULMOD limb2 full−own zero | +1663350455 | +3101828576154650286743152012 | -1438478121154650286743152012 | 0 | true/true | `11110011` |
| MULMOD limb3 full−own zero | +2214603358 | +3101828576154650286743152012 | -887225218154650286743152012 | 0 | true/true | `11110011` |
| MULMOD limb4 full−own zero | +2085065920 | +3101828576154650286743152012 | -1016762656154650286743152012 | 0 | true/true | `11110011` |
| ADDMOD zero limb2−limb1 | +2167 | +0 | +2167000000000000000000 | 2167 | false/false | `11101111` |
| ADDMOD full limb2−limb1 | +150989848 | +0 | +150989848000000000000000000 | 150989848 | false/false | `11101111` |
| ADDMOD zero limb3−limb1 | +532 | +0 | +532000000000000000000 | 532 | false/false | `11101111` |
| ADDMOD full limb3−limb1 | +546960751 | +0 | +546960751000000000000000000 | 546960751 | false/false | `11101111` |
| ADDMOD zero limb4−limb1 | +293 | +0 | +293000000000000000000 | 293 | false/false | `11101111` |
| ADDMOD full limb4−limb1 | +405898587 | +0 | +405898587000000000000000000 | 405898587 | false/false | `11101111` |
| MULMOD zero limb2−limb1 | +376 | +0 | +376000000000000000000 | 376 | false/false | `11101111` |
| MULMOD full limb2−limb1 | +224186190 | +0 | +224186190000000000000000000 | 224186190 | false/false | `11101111` |
| MULMOD zero limb3−limb1 | +3117 | +0 | +3117000000000000000000 | 3117 | false/false | `11101111` |
| MULMOD full limb3−limb1 | +775441834 | +0 | +775441834000000000000000000 | 775441834 | false/false | `11101111` |
| MULMOD zero limb4−limb1 | +2991 | +0 | +2991000000000000000000 | 2991 | false/false | `11101111` |
| MULMOD full limb4−limb1 | +645904270 | +0 | +645904270000000000000000000 | 645904270 | false/false | `11101111` |

All 12 class differences are positive with zero estimate increments. The eight repetition increments have positive estimate deltas that cover the corresponding actual deltas. Neither observation selects a fixed cause or a new price. Absolute control gaps remain TODOs; this panel does not certify ordinary-transaction usability, cache/IID behavior or production admission.

## Frozen identities and retained evidence

Producer/source revision is `4de2816dd42a456e0128354e28c784b63d874de0`; the [binding spec](../superpowers/specs/2026-11-04-modular-density.md), [implementation plan](../superpowers/plans/2026-11-04-modular-density.md), [maintained tool usage](../../bin/zkgas/bench/README.md) and [coverage inventory](zkgas-workload-coverage.md) define the source and scope. The source-only independent review passed specification and quality with no findings. The final stronger closure matches all 181 inventory files to both current files and committed blobs. Native build declarations of 81 source files (66 Rust) and 70 compiled workspace inputs are separate counts, not a fresh build attestation.

| Identity | SHA256 / value |
| --- | --- |
| Model ID | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| SDK | `6.3.0` |
| Guest source revision | `4580faf7dc009714b310fd625b5810667019d683` |
| elf artifact | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| model artifact | `4cb6c65c6ad28e25e1f30639803958f9555e43d51e6d35d2f1945b0757ce1ac7` |
| native artifact | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| program_manifest artifact | `8d5b4a482d26e3bdca94c5e66b9de7b33dc57bcfbaa9b9c24140bf030c1f93a9` |
| quote artifact | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| sp1 artifact | `1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b` |
| Native build | `1e1a7f4dbd141ce97359114b737d744ee71eb37b78e974038179eedf25cd8023` |
| Source review | `b441b0c4fb3465ada9688a63011b747537f85866678c05b9cbf3fd76664ded33` |
| Source inventory | `311d510b957fd871c589b8c494250114089ebdbcd927e62d07c53fc64f5ce9ce` |
| Policy freeze | `1af2c7b5566cb3ec3daf5efef610c0502610a314595f2da7edde369cd3493d42` |
| Config | `5e6f1c6f13f51b58204d91a76366e2488717db82dc78f1617df9cc9a1f3b9406` |
| Qualified input index | `0ce1d734bc616c160e34ba7cd401a2eee9071df2f9ddb06e163602fd32c40c96` |
| Original qualification manifest (430 unchanged) | `77cc1f9f9786d2657847cb7b2cb569d2a6fe3c45756e957261182b1eee1c72be` |
| Final qualification manifest (436 files) | `5090048084d9c9735e29bb13af6681ca742b02f21e0800c3e5dfc683406acae8` |
| Analysis | `e7a922f0e24336cf19bf808e14886f12d5d211aba0ce90a49100173afa6ab2b0` |
| Independent raw16/exact20 audit | `921ad22cd74d535d0105858155a2d43e5ce132267337b4d8c3253e0110f255c4` |
| Raw final closure | `587155d1e8d343551effcaa2e89b63d6bbb11974aa269aade9fdf2868d672e1a` |
| Committed source closure | `1a6f3381ee862aaae58b8a6c2674707cdc61ef64f813969502bf465dd948f19f` |
| Independent real-host audit | `26bd27fc568e1d246659ed354e5ffedb30e3f50354ee14736887dcc0cd0e2c0f` |
| Real-host receipt | `83201d2c8453cfd4258fc2275de41c0c4c16001719dd191b1a095d6911f17ffe` |
| Measurement order | `4b507ca8cf0be26786876911175e80798ba583e903b7e63cb68672b37d9d4388` |

The original 430 qualification files remain unchanged; six later closure files produce the final 436-file manifest. The planned execution order is all eight controls then all eight fulls. Runtime tools, source, inputs, original quotes and order were frozen before actuals. All 16 raw reports bind the canonical input, native/quote public values and frozen program, with no retry or relabelled reference measurement.

Actual guard limits were 6,144 MiB sampled process-tree PSS, 600 seconds, 192 MiB output and 16 MiB log, with 256 MiB runtime disk plus 320 MiB retention reserve (576 MiB preflight). Native cap was 180 seconds and quote diagnostic cap 1,800 seconds. Peak actual sampled PSS was 4,544,562,176 bytes and maximum wall time 69.956023927778 seconds. Sampling is not a continuous peak guarantee.

All 64 original owned shared-memory/semaphore resource names are unique. The corrected authoritative receipt and independent audit match every original recorded device to real-host device 27 in mount namespace mnt:[4026531841] and lstat the exact names absent. A sandbox-private absence receipt is preserved as preliminary and explicitly rejected as host authority. No unrelated resource was deleted; the independent host check performed no deletions or external executable calls.

The private recovery archive `modular-density-evidence-20261104.tar.gz` is SHA256 `cf231a48f1f3814be6ca58e76bf8dfac8e6dc4b2e59d9a4388d8515513e4f17e`, 2,520,124 compressed bytes, 697 regular members, 232 restorable duplicate aliases and four external runtime artifacts. Root streamed and verified every member hash without extraction. Executables remain external by hash/alias. This private archive is not published standalone Atlas recovery; immutable data fixtures and maintained tooling retain separate destinations.

Prior nine cohorts remain separate frozen identities, without old reruns or an inflated union:

| Prior cohort | Inputs | Frozen context SHA256 |
| --- | ---: | --- |
| division32 | 32 | `e5647210f90d673dd6bc0a0d934a7a0a8c9ffd33c1438c227705316d6e092b68` |
| keccak27 | 27 | `b702bc9104a4bfc5ba1b4b64ea372d25c07387635ad97aebb687eae053f9d644` |
| linear_sha18 | 18 | `286e6711f3455495ebc7662696bbdd53e174b70d856260475c53cfca248bb21c` |
| log30 | 30 | `2d9057d738d544694cc419acf6aaf240e56bdd6ea8d5e40d13634d87897cba56` |
| mcopy12 | 12 | `d197e92a897d4d3ef9936c7ad519c399a695e940bf04fee0110bc514249d9380` |
| pairing18 | 18 | `088eeabd4a6373c81ba82d648b79a16ca25b48633ae4c0dce69f2cfbb120518f` |
| persistent236 | 236 | `99668c6585bbd517f665c030ac3b830874ec4bb5d9c828ebb759a4859c0db8ff` |
| sha18 | 18 | `0bad7d639c22add65c3ca5b5dbdd089f670154a64e8954bfa78b715c968b2a17` |
| transient8 | 8 | `c322c0957334d51882668febbfa7e2c090319f440653523ac7b541ba966ac8c0` |

This documentation pass independently re-read raw SP1 reports, reconstructed all literal16 rows and exact20 diagnostics/eight flags, checked eight capacity/full+1 receipts, hashes, links and portable paths using a private retained-data checker. It did not rerun producer functional tests, native, quote, SP1, prior cohorts or builds. Source tests and external receipts above describe retained runs, not new executions.

The evidence is finite execute-only SDK6.3/experimental-ELF evidence. It does not establish STARK proving, aggregation, final SNARK/PLONK wrapping, proof/GPU performance, current production parity, arbitrary operand/modulus/control-flow coverage, per-PC charging, natural historical usability or a global component bound. Other arithmetic domains, mixed-family order/composition and exact production promotion remain open. No price fit, model selection, guest/SDK/ELF/dependency pin or deployment change is authorized by these results.
