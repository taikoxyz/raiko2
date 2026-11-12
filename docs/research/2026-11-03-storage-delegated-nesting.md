# Many-key storage through shared-code DELEGATECALL nesting

All 16 new execute-only observations completed once: eight own-R0 controls and eight generated full points. Six literal node gaps remain: four K16 controls and two full caught-REVERT/K16 points, at depth1/depth4 gaps of 108,753,117/109,050,081 PGU. Maximum all-point gap is 149,088,122 PGU; maximum actual is 886,973,305 PGU. No complete-proposal gap or >2B actual-minus-node gap at <=30M charged gas appears. Prices remain unchanged; smaller deficits remain explicit TODOs.

This panel carries the unchanged [many-key leaf](2026-11-03-storage-dirty-keys.md) through F4 with one shared leaf code account and four distinct top storage owners at wrapper depths 1/4. Sixteen exact old [CALL observations](2026-11-03-storage-caught-nesting.md) are read-only references, without rerun or recount. The [coverage inventory](zkgas-workload-coverage.md) records this finite interaction and remaining work.

## Programs, contexts and finite gas limits

The 16 inputs cross leaf STOP/REVERT, K=16/1024, wrapper depth=1/4 and own-R0/generated-full. Four signed users visit disjoint guarded wrapper chains with empty calldata. All chains share one leaf code address at BASE and only that account carries the leaf bytecode; configured chain accounts total 4*depth+1. Exactly four top wrappers own populated storage. Intermediate wrappers and the shared code account have explicit empty maps, while their account/code/global authentication remains required. The unused primary is STOP. Four user receipts and Anchor succeed even when leaf REVERT is caught.

Each top owner has the identical raw map: phase slot 0 contains 65537, body keys 65537..65536+K contain 1. Leaf code reads phase, seeds all K keys 1→5, performs R warm dirty 5→7→5 pairs per key in batches of 64 plus tail, then cleans 5→1. It preserves round/marker checks and 192-byte memory. Own-R0 includes seed/cleanup and shares its full point's exact signed envelope. All complete maps and owner-specific roots restore; fees, nonces and Anchor still change whole-world state.

The initial disjoint-range idea was corrected before measurements: keying those ranges by shared code would collapse only phase0, leaving 4K+1 pairs. Identical maps make the stronger test explicit. Correct logical resources are 4(K+1) storage-address/slot accesses and 4K writes; code-keyed K+1/K collapse is rejected. Physical node-hash/root deduplication is allowed; it cannot erase the four authenticated logical contexts. Read-only phase0 is excluded from literal writes.

Every wrapper emits six F4 operands, requests 12M gas with empty input/output ranges, checks child status and zero RETURNDATASIZE, then STOPs. Innermost expected status is 1/0 for STOP/REVERT; outer status is 1. Ordinary pre-F4 gas is 18, cold target gas 2600 and post-F4 gas 46, adding 2664 gross per wrapper. With C=leaf gross−21000, user gross=21000+C+2664*depth; no repeated child intrinsic is charged. Leaf refund before terminal is 2800K: STOP propagates it and caught REVERT discards it. Apply min(propagated refund,gross//5) once at the transaction.

The envelope recurrence is T0=C+2301; Tj=2618+max(ceil(64*T(j−1)/63),T(j−1)+46); signed user envelope=21000+Tdepth. Independent frame checks reconstruct actual min(12M,available−available//64) forwarding, requirements, returned gas and retained post-F4 gas. The bounded interpreter carries code_address and storage_address independently through every actual F1/F4 byte. Repeated shared code across transactions does not share warmth. This is a fixture interpreter, not a general EVM.

Constraints include a 31M header, <=30M charged block gas including Anchor 167355, <=15M/user and EIP-7825, every ordered admission prefix, SSTORE gas-left >2300, <=8192 selected targets and <=24576 code bytes. Full means maximal uniform R for this fixed program; sufficient-envelope R+1 executes before checking candidate constraints. It is not a global packing maximum.

| Leaf | K | Depth | Full R | Gross/user | Applied refund/user | Charged/user | Signed envelope/user | Charged block gas |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| revert | 1024 | 1 | 5 | 7406122 | 0 | 7406122 | 7525596 | 29791843 |
| revert | 1024 | 4 | 5 | 7414114 | 0 | 7414114 | 7896640 | 29823811 |
| revert | 16 | 1 | 2094 | 7456058 | 0 | 7456058 | 7576325 | 29991587 |
| revert | 16 | 4 | 2092 | 7457074 | 0 | 7457074 | 7942394 | 29995651 |
| stop | 1024 | 1 | 12 | 8968740 | 1793748 | 7174992 | 9113017 | 28867323 |
| stop | 1024 | 4 | 11 | 8753500 | 1750700 | 7002800 | 9323114 | 28178555 |
| stop | 16 | 1 | 2107 | 7501396 | 44800 | 7456596 | 7622382 | 29993739 |
| stop | 16 | 4 | 2105 | 7502412 | 44800 | 7457612 | 7990680 | 29997803 |

Six full points exceed 99% charged occupancy. STOP/K1024/depth1 charges 28,867,323 and depth4 charges 28,178,555; next R fails an admission prefix despite charging below 30M. These are prefix-limited full selectors, not near-full or arbitrary-capacity observations.

## Literal same-proposal results

IDs below abbreviate `storage-delegated-nesting-{leaf}-k{K}-d{depth}-r{R}`. Actual is whole-proposal SDK6.3 `ExecutionReport.gas * 10 // 191`. Literal node PGU is the sum of per-block known-PGU ceilings; these proposals each have one block. Complete-proposal estimates remain separate. Raw Unzen gas scalars are not actual PGU. Positive gaps are max(actual−estimate,0). Full node/actual ratios are 0.87704526–1.26544881; full complete-proposal/actual ratios are 1.05353142–1.73794138.

| Leaf | K | Depth | R | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive proposal gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| revert | 1024 | 1 | 0 | 25327203 | 361059377 | 513788214 | 738767373 | 0 | 0 |
| revert | 1024 | 1 | 5 | 29791843 | 467800269 | 588030285 | 813009444 | 0 | 0 |
| revert | 1024 | 4 | 0 | 25359171 | 361651268 | 514138906 | 739552536 | 0 | 0 |
| revert | 1024 | 4 | 5 | 29823811 | 468365590 | 588380977 | 813794607 | 0 | 0 |
| revert | 16 | 1 | 0 | 663459 | 164989645 | 36012658 | 192246713 | 128976987 | 0 |
| revert | 16 | 1 | 2094 | 29991587 | 886973305 | 778220188 | 934454243 | 108753117 | 0 |
| revert | 16 | 4 | 0 | 695427 | 165604740 | 36363349 | 193031876 | 129241391 | 0 |
| revert | 16 | 4 | 2092 | 29995651 | 886912356 | 777862275 | 934530801 | 109050081 | 0 |
| stop | 1024 | 1 | 0 | 20295215 | 350284727 | 493786883 | 718766041 | 0 | 0 |
| stop | 1024 | 1 | 12 | 28867323 | 533962845 | 671967853 | 896947012 | 0 | 0 |
| stop | 1024 | 4 | 0 | 20320791 | 350857488 | 494137574 | 719551205 | 0 | 0 |
| stop | 1024 | 4 | 11 | 28178555 | 519554898 | 657470130 | 882883761 | 0 | 0 |
| stop | 16 | 1 | 0 | 564219 | 164840485 | 16011326 | 172245381 | 148829159 | 0 |
| stop | 16 | 1 | 2107 | 29993739 | 680228837 | 762824790 | 919058844 | 0 | 0 |
| stop | 16 | 4 | 0 | 589795 | 165450139 | 16362017 | 173030544 | 149088122 | 0 |
| stop | 16 | 4 | 2105 | 29997803 | 680498314 | 762466876 | 919135403 | 0 | 0 |

## Matched diagnostics and attempted work

The retained 40 whole-proposal contrasts comprise eight full−own-R0, eight leaf REVERT−STOP, eight depth4−depth1 and 16 new−matched-old-CALL comparisons. They retain actual differences and exact pre-rounding node/proposal differences scaled by 10^18 units/PGU. Subtracting rounded ceilings does not replace those scaled differences. The five equality columns count true flags.

| Diagnostic | Count | Same R | Same K | Same charged gas | Same signed envelope | Same selected storage support | Actual difference range PGU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Full−own-R0 | 8 | 0 | 8 | 0 | 8 | 8 | 106714322..721983660 |
| Leaf REVERT−STOP | 8 | 4 | 8 | 0 | 0 | 8 | -66162576..206744468 |
| Depth4−depth1 | 8 | 5 | 8 | 0 | 0 | 8 | -14407947..615095 |
| New−old CALL | 16 | 16 | 16 | 0 | 0 | 0 | -49637216..-843559 |

True flags total 25/40/0/8/24 respectively. Selected-storage equality compares canonical per-address authenticated node-hash sets/digests with underlying nodes retained. All 16 old-CALL flags are false: old support names four leaf owners, while new support names four top wrappers. Physical hash/root matches alone cannot identify these account domains. New common raw maps also differ from old CALL's disjoint ranges. Added/different account/code/global support and these map/context changes prevent isolated attribution to an F4 price.

Independent attempted counters retain completed child work through caught REVERT, including F4 and all wrapper STOPs, leaf STOP/REVERT, dispatch and distinct resources. The four logical owner contexts are checked against the rejected code-address collapse: literal user accesses are 4(K+1), writes are 4K, and observed system additions are +4 access/write resources in node/full selectors. Phase0 is read-only and never counted as a user write. Empty intermediates/code owners still have account/code/global proofs. Aggregate/unique counters do not prove per-opcode occurrence charging.

## Tools, qualification and retained evidence

See the [producer](../../bin/zkgas/bench/cases/storage_delegated_nesting.py), [shared frame interpreter](../../bin/zkgas/bench/tests/storage_caught_vm.py), [usage](../../bin/zkgas/bench/README.md#shared-code-delegated-storage-calls) and [binding specification](../superpowers/specs/2026-11-03-storage-delegated-nesting.md). Keyword-only `call_kind` extends the common helpers; default CALL preserves all 16 prior requests, runtime identities and full core metadata byte-for-byte. Wrapper assembly selects the opcode before emission. There is no bytecode patching or duplicated leaf code.

Retained managed-Python `-S` checks below are original results, not reruns for this record. `PYTHON_BIN` names the existing managed interpreter and `NATIVE_BIN` the frozen native executable.

- `"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests -p test_storage_delegated_nesting.py -v`: final eight tests, seven executed successfully and one optional native skip; expected RED failures were retained before implementation. Literal all16 trajectories, contexts/maps, EIP150/refunds, boundaries, immutable prior CALL identities and negative metadata/bytecode/observation cases passed.
- `PYTHONPATH=bin/zkgas/bench/tests "$PYTHON_BIN" -S -m unittest test_storage_caught_nesting test_storage_dirty_keys test_storage_key_shift test_storage_root_journal -v`: 33 tests, 29 executed successfully and four optional native skips.
- `STORAGE_DELEGATED_NATIVE_BIN="$NATIVE_BIN" STORAGE_DELEGATED_NATIVE_OUTPUT="$OUTPUT_DIR" PYTHONPATH=bin/zkgas/bench/tests "$PYTHON_BIN" -S -m unittest test_storage_delegated_nesting.StorageDelegatedNestingTests.test_frozen_native_positive_and_accepted_operand_mutants -v`: one test passed with four sequential frozen native invocations, two positives and two dirty7→9→5 operand mutants. Native accepts both mutants; independent literal intent rejects them. This separate smoke lane does not replace all16 qualification.

Independent review of the complete seven-file source change passed specification and quality review without material findings. All 16 original native and 16 real quotes, 64 signatures, receipts, whole-world authentication, owner-specific complete roots, canonical mutation witnesses, all-frame counters and F4 EIP150/refund gates passed before freezing one final source head. Every new invocation binds ef2689b0; there is no source-transition bridge or invocation/source race in this round. Old CALL reference provenance retains its original f8f/e818 identities.

Independent raw16/diagnostic40/cohort204 audit passed. It reconstructed PGU from original raw SDK6.3 reports, checked 40 exact contrasts and their equality markers, canonical deduplication, frozen identities and all 204 cohort rows. Focused final checks and closure passed; no measurements were rerun or failed actuals removed.

The independent retained commands were `"$PYTHON_BIN" actual_audit.py --root "$CAMPAIGN" --qualification "$QUALIFICATION" --freeze-sha256 ed7cc7d09f80804c9ab9a9599fc3947ee889ffcfad2658e34c33e0d2a28763f5 --config-sha256 23021f1f14d1a511c3e19653d40864144af41ced6c70ab100c39fd56f80dbd77 --qualification-sha256 3a71e96ad13721eb711d2301c8080f3713ce8a6fdbedeec64722939162ceee83 --final`, `"$PYTHON_BIN" focused_final_checks.py` and `"$PYTHON_BIN" finish_actual_audit.py`. All exited 0; exact original argv and stdout are retained. These audit commands launch no native, quote or SP1 execution.

The nine-panel finite storage subset contains 204 distinct canonical inputs, 92 generated full selectors and 84 near-full points. It has 20 full node gaps, maximum full gap 111,328,749 PGU and maximum all-point gap 164,072,990 PGU; high-priority and complete-proposal gaps remain zero. Reused references are excluded. Historical 188/84/78 counts remain unchanged. These are selected finite panels, not IID samples, a global bound or a coverage percentage.

All 16 actuals completed once with EXIT0. Maximum wall time was 26.501565493643284 seconds and sampled process-tree PSS 4,571,193,344 bytes. The unchanged limits are 180 seconds per native invocation, 1800 seconds per quote with at most two concurrent quotes, and 600 seconds/6GiB, 16MiB logs and 192MiB output per SP1 actual. Actuals run sequentially. Disk preflight requires 576MiB: a 256MiB free floor plus a 320MiB retention reserve. The host receipt checks all 64 uniquely owned shared-memory/semaphore objects absent after completion; it does not claim host-wide cleanup.

| Identity | SHA256 / version |
| --- | --- |
| Final producer source | `ef2689b00faab762d6acfad215cd6ee2af49531d` |
| Model | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Retained ELF / SDK | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` / `6.3.0` |
| Native executable | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Quote executable | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| SP1 executable | `1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b` |
| Source file manifest | `bfc01bc33b12f38bea7f5595bd8f7e28defb60e5ed91bc2c6e30e23c3516229b` |
| Frozen config | `23021f1f14d1a511c3e19653d40864144af41ced6c70ab100c39fd56f80dbd77` |
| Pre-actual policy | `ed7cc7d09f80804c9ab9a9599fc3947ee889ffcfad2658e34c33e0d2a28763f5` |
| Campaign input/quote index | `f5de25b51e0d270ccc1d77eee72bc70617867b47118446b0388d350b85a3258e` |
| Independent qualification | `3a71e96ad13721eb711d2301c8080f3713ce8a6fdbedeec64722939162ceee83` |
| Reference join | `1852546adc11570cb2a98560a4000fa46f69510fb7b267189683bc2fb3f25877` |
| Frame semantics audit | `86fbf2165238b22a0d27cc7d7349e579792799da76ca6f9bb5477b6c9d417b15` |
| Counter retention audit | `e6769335ad62ca1dcade4685e2097e64d9f6ed0a4a1d82d919af9d7bfab4b48a` |
| Native qualification | `237a56bfb7e6534f524ac5e64e7c27c1a85599536698aa6f811c8c7d8d225509` |
| Actual analysis | `4b5d4c8f6b644bc1f264cd5ffed105f11c2ccb06569cdd19eeaa94dc54a5c127` |
| Gas-budget report | `e95710e8e8500bc39aa65168012dd02b0bc5d34ecc335bc98f37a6db4541f306` |
| Independent raw audit | `01cbb80c9de6d0e5dc232ecdb59988eb31bd8f482398e9bc0ae1b13ccfd1a9e6` |
| Independent focused checks | `5bb08dbd60bd4e04d9336304ff42eab0bf6090aa3aaeb4a6df09f7d22fb51309` |
| Independent closure | `6dcf5502a54260e8c7f3f98c48650c9e791d9c8a388c0983ee10d930180c0962` |
| Independent final report | `ba1c96901012f0bcdec905e2f8d1f37fc1768d6828d121f2fdfbe2bea2c85b16` |
| Independent final 256-file manifest | `67dcc6ccd47b03eec067e00d45345703509ce5d4211db266a20bd27345e649d3` |
| Previous-head CI observation | `5121f0d1476cfebfd27d639620a38c124c6a26b7beb7a7feac990fa05494abd0` |
| Nine-panel storage summary | `a03fe6fcd9fe6c0af8fd99230d21be7b9061d607eb6166cef58c79bc43bf45f7` |
| Runtime summary | `90bbb8ab918d03a11a5c8a28bf3370665ffa81e5a36ab6983b239a95784b1821` |
| Recovery receipt | `7455a473326d0fc6a6b2aca331a2ac86c9ab6bac70817b9a945c4b5c534dba17` |
| Host shared-memory receipt | `0d3404baf6b493ec271804f5bc039a899e67f7aa7929033115debb00c0dbb0d7` |

The ELF declares source revision `4580faf7dc009714b310fd625b5810667019d683`; this is not a build attestation. Model/native/quote/SDK/ELF identities remain frozen. No Rust, guest, SDK, ELF or pin change/build, price fit or pricing change occurred. These runs execute the guest without generating a proof; they establish no GPU throughput, production parity or natural-history admission result.

The previous published 4f head CI run 41609980273 failed at the SDK6.8.1 nested runner's missing binary and the retained RISC0 fingerprint check. Primary logs are retained privately under the `storage-delegated-pr296-ci-failed-20261103.log` basename. This increment does not repair those failures or establish current/new-head CI success.

The verified private archive `storage-delegated-nesting-evidence-20261103.tar.gz` is SHA256 `3193c4e9984fa365c55a1e00a63e2c9427dc8d589a613fcfac118c953dfede51`: 20,057,321 compressed bytes, 511 regular members carrying 68,334,777 payload bytes, 96 restorable duplicate aliases and four external runtime artifacts identified by hash. All streamed member hashes were verified without extraction.

Retained evidence includes original requests/canonical inputs, native diagnostics, original quotes/invocations, all 16 successful actuals, exact old reference joins, source/tools/config/policy/counter/runtime/host identities and audit records. Four runtime artifacts and 16 old CALL inputs remain external; the old CALL archive SHA256 is `09f9b680e0b7d3b4424132121c33af57316cfb257285b58303886cd715710b6d`. This is not a standalone auditor runner or public immutable Atlas fixture.

Remaining work includes caller prewarming, ancestor rollback, wider graphs/exceptional exits, deliberate hashed-path skew/depth and mutation topology, cache/lookup distributions, heterogeneous packing/order and exact-production-program/natural-history promotion. This panel samples shared-code storage ownership through successful ancestors and caught leaf rollback. No merge, deployment or on-chain action is part of it.
