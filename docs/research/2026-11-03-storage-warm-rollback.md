# Caller-prewarmed storage and caught ancestor rollback

All 32 new execute-only observations completed once without setup failure, actual failure or retry: 16 own-R0 controls and 16 generated full points. All 32 have literal node gaps, including all 16 full points; maximum full/all-point gaps are 109,265,103/128,975,075 PGU. Maximum actual is 887,059,584 PGU. No absolute complete-proposal gap or >2B actual-minus-node gap at <=30M charged gas appears. Smaller gaps remain explicit low-priority TODOs; no price change is selected.

This panel extends the selected [shared-code DELEGATECALL interaction](2026-11-03-storage-delegated-nesting.md) with caller prewarming and caught ancestor rollback at depths 2/4. It requires no old actual references or reruns. The [coverage inventory](zkgas-workload-coverage.md) retains the finite scope and remaining work.

## Programs, checkpoints and finite gas limits

The 32 inputs cross exit placement leaf/ancestor, K16/256, depth 2/4, prewarm false/true and own-R0/generated-full. Four successful signed users visit disjoint DELEGATECALL chains sharing one leaf code account. Exactly four TOP wrappers own populated storage, each with the identical raw map: phase 0=65537 and body keys 65537..65536+K=1. Intermediate wrappers and the shared code account have empty maps; their account/code/global authentication remains required. There are 4*depth+1 configured accounts.

The unchanged leaf reads phase, seeds 1→5, performs R warm dirty 5→7→5 pairs per key in 64-round batches plus tail, then cleans 5→1. TOP optionally reads and guards every phase/body value before calling and always postchecks every value after caught rollback. TOP uses 32 memory bytes and leaf 192 in separate frames, with no TOP writes. K256 replaces K1024 because four K1024 seed/cleanup controls plus mandatory cold postchecks would exceed 30M even at R0; no infeasible control or waived constraint is included.

In leaf mode, leaf REVERT is caught by the innermost wrapper. In ancestor mode, leaf STOP commits to its immediate caller, that ancestor REVERTs, and its caller catches failure. Each user has one REVERT, depth STOPs and depth F4s; all user receipts succeed. Leaf preterminal refund is 2800K. Leaf mode discards it immediately; ancestor mode retains it only until ancestor REVERT, where the ancestor preterminal counter is 2800K. Every TOP preterminal/effective/applied refund and global effective refund is 0. Complete storage maps restore; fees, nonces and Anchor still change whole-world state.

Without TOP prewarming, leaf initial reads cost 2100 and TOP postreads are cold 2100 after either rollback. With prewarming, leaf initial reads and TOP postreads cost 100: parent warmth precedes the reverting checkpoint. Successful leaf STOP warmth must disappear on ancestor REVERT. Independent per-frame checkpoints restore state/refund/account warmth/owner-slot warmth while retaining attempted reads, writes, opcodes and frames. Logical resources remain 4(K+1) address-slot accesses and 4K writes; read-only phase 0 is excluded from writes. Observed system additions of +4 access/write resources and 12363 dispatch are allowances, not independent proof of Anchor opcode pricing. Aggregate counter agreement does not establish per-opcode occurrence charging or actual-PGU allocation.

F4 ordinary pre/cold-target/post gas is 18/2600/46. Ancestor mode moves the 6-gas empty REVERT from leaf to wrapper. Gross/charged user gas and aggregate opcode maps agree for exit pairs at fixed K/depth/prewarm/R. The common signed envelope is the maximum of both mode requirements at their common maximal uniform full R; own-R0 retains that full envelope. With leaf execution C, Tleaf=C+2301 and Ti=pre_i+max(ceil(64*Tchild/63),Tchild+post_i), including TOP traversal fees/memory in their actual regions. Actual forwarding uses min(12M,available−available//64), with returned-gas/post checks.

Constraints include 31M header and 30M charged block gas including Anchor 167355, the fixture's 15M/user cap separately from EIP-7825's 16,777,216 protocol cap, every ordered admission prefix, the fixture's 12M child request, SSTORE gas-left>2300, 8192 selected targets and 24576 code bytes. Sufficient-envelope R+1 executes before candidate constraints fail. Full means maximal uniform R for this fixed program, not a global packing maximum. All 16 full points exceed 99% charged occupancy.

| K | Depth | Prewarm | Common full R | Gross/charged user | Signed envelope/user | Charged block gas |
| ---: | ---: | :---: | ---: | ---: | ---: | ---: |
| 16 | 2 | false | 2083 | 7457570 | 7659274 | 29997635 |
| 16 | 2 | true | 2091 | 7454687 | 7690198 | 29986103 |
| 16 | 4 | false | 2081 | 7455922 | 7901532 | 29991043 |
| 16 | 4 | true | 2090 | 7456527 | 7935969 | 29993463 |
| 256 | 2 | false | 94 | 7417378 | 7528058 | 29836867 |
| 256 | 2 | true | 103 | 7454223 | 7626097 | 29984247 |
| 256 | 4 | false | 94 | 7422706 | 7755461 | 29858179 |
| 256 | 4 | true | 102 | 7403743 | 7797285 | 29782327 |

## Literal same-proposal results

Actual is whole-proposal SDK6.3 `ExecutionReport.gas *10//191`. Literal node PGU sums per-block known-PGU ceilings; each input has one block. Complete-proposal estimates remain separate. Raw SDK gas is not PGU. Positive gaps mean max(actual−estimate,0). Full node/actual ratios are 0.87636954–0.92279915; full complete-proposal/actual ratios are 1.05352988–1.14962505.

IDs abbreviate `storage-warm-rollback-{exit}-k{K}-d{depth}-w{0|1}-r{R}`. All complete-proposal positive gaps are 0.

| Exit | K | Depth | Warm | Full R | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| ancestor | 16 | 2 | 0 | 2083 | 884082355 | 775013798 | 931407180 | 109068557 |
| ancestor | 16 | 2 | 1 | 2091 | 886995391 | 778506064 | 934899447 | 108489327 |
| ancestor | 16 | 4 | 0 | 2081 | 883799037 | 774538987 | 931207514 | 109260050 |
| ancestor | 16 | 4 | 1 | 2090 | 886767329 | 778385555 | 935054082 | 108381774 |
| ancestor | 256 | 2 | 0 | 94 | 725477674 | 661199131 | 834002402 | 64278543 |
| ancestor | 256 | 2 | 1 | 103 | 781996971 | 721624499 | 894427769 | 60372472 |
| ancestor | 256 | 4 | 0 | 94 | 725902218 | 661432925 | 834511340 | 64469293 |
| ancestor | 256 | 4 | 1 | 102 | 777017462 | 716208652 | 889287067 | 60808810 |
| leaf | 16 | 2 | 0 | 2083 | 884079256 | 775013798 | 931407180 | 109065458 |
| leaf | 16 | 2 | 1 | 2091 | 886998511 | 778506064 | 934899447 | 108492447 |
| leaf | 16 | 4 | 0 | 2081 | 883804090 | 774538987 | 931207514 | 109265103 |
| leaf | 16 | 4 | 1 | 2090 | 887059584 | 778385555 | 935054082 | 108674029 |
| leaf | 256 | 2 | 0 | 94 | 725478478 | 661199131 | 834002402 | 64279347 |
| leaf | 256 | 2 | 1 | 103 | 781995189 | 721624499 | 894427769 | 60370690 |
| leaf | 256 | 4 | 0 | 94 | 725898712 | 661432925 | 834511340 | 64465787 |
| leaf | 256 | 4 | 1 | 102 | 777018449 | 716208652 | 889287067 | 60809797 |

All 16 controls retain seed/cleanup and the full signed envelope. Their absolute gaps remain visible:

| Exit | K | Depth | Warm | R0 actual PGU | R0 node PGU | R0 complete proposal PGU | R0 positive node gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| ancestor | 16 | 2 | 0 | 165516561 | 36703595 | 193096978 | 128812966 |
| ancestor | 16 | 2 | 1 | 165719971 | 37361441 | 193754824 | 128358530 |
| ancestor | 16 | 4 | 0 | 165908464 | 36937389 | 193605916 | 128971075 |
| ancestor | 16 | 4 | 1 | 166110629 | 37595235 | 194263762 | 128515394 |
| ancestor | 256 | 2 | 0 | 216073095 | 158187738 | 330991009 | 57885357 |
| ancestor | 256 | 2 | 1 | 219070142 | 167766339 | 340569610 | 51303803 |
| ancestor | 256 | 4 | 0 | 216454991 | 158421532 | 331499947 | 58033459 |
| ancestor | 256 | 4 | 1 | 219480213 | 168000134 | 341078548 | 51480079 |
| leaf | 16 | 2 | 0 | 165522678 | 36703595 | 193096978 | 128819083 |
| leaf | 16 | 2 | 1 | 165721831 | 37361441 | 193754824 | 128360390 |
| leaf | 16 | 4 | 0 | 165912464 | 36937389 | 193605916 | 128975075 |
| leaf | 16 | 4 | 1 | 166109589 | 37595235 | 194263762 | 128514354 |
| leaf | 256 | 2 | 0 | 216074860 | 158187738 | 330991009 | 57887122 |
| leaf | 256 | 2 | 1 | 219069859 | 167766339 | 340569610 | 51303520 |
| leaf | 256 | 4 | 0 | 216452813 | 158421532 | 331499947 | 58031281 |
| leaf | 256 | 4 | 1 | 219476073 | 168000134 | 341078548 | 51475939 |

## Matched diagnostics and remaining increments

The 64 whole-proposal contrasts comprise16 each full−own-R0, ancestor−leaf, warm−cold and depth 4−depth 2. They retain exact pre-rounding node/proposal differences scaled by 10^18 units/PGU, rather than substituting differences of rounded ceilings. The five equality columns count true flags.

| Diagnostic | Count | Same R | Same K | Same charged gas | Same signed envelope | Same selected storage support | Actual difference range PGU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Full−own-R0 | 16 | 0 | 16 | 0 | 16 | 16 | 509403618..721276680 |
| Ancestor−leaf | 16 | 16 | 16 | 16 | 16 | 16 | -292255..4140 |
| Warm−cold | 16 | 8 | 16 | 0 | 0 | 16 | 197125..56519297 |
| Depth4−depth2 | 16 | 10 | 16 | 0 | 0 | 16 | -4979509..424544 |

True flags total 34/64/16/32/64 respectively. Selected-storage equality compares canonical per-owner authenticated storage-node hash sets/digests for exactly four TOP owners, excluding global/account/code support. Equal physical roots/hashes do not erase owner identity. Exit pairs share R, gas, aggregate opcode maps and signed envelopes, but code/account/input bytes differ; these are not isolated physical-operation actual costs. Warmth and depth contrasts also change program/support structure and sometimes R.

Only eight of 16 own-R0→full increments are covered by node and complete-proposal increments. All eight K256 increments are uncovered on both surfaces; the maximum positive exact scaled increment gap, rounded upward, is 9,333,858 PGU. This low-priority incremental TODO remains even though every absolute complete-proposal quote covers its actual. The absolute node deficits therefore must not all be attributed to shared startup. No new tariff is fitted.

## Tools, qualification and retained evidence

See the [producer](../../bin/zkgas/bench/cases/storage_warm_rollback.py), [separate frame oracle](../../bin/zkgas/bench/tests/storage_warm_rollback_vm.py), [usage](../../bin/zkgas/bench/README.md) and [binding specification](../superpowers/specs/2026-11-03-storage-warm-rollback.md). The common call emitter and keyword-only `initial_warm_keys` preserve all 32 prior CALL/F4 request, runtime and complete core-metadata identities. The existing leaf and prior CALL/F4 frame oracle remain unchanged.

Original managed-Python checks passed: native-enabled `-S -m unittest discover -s bin/zkgas/bench/tests -p test_storage_warm_rollback.py -v` ran eight tests without skips, including two full native positives and two native-accepted dirty9 mutants rejected by literal intent. `PYTHONPATH=bin/zkgas/bench/tests "$PYTHON_BIN" -S -m unittest test_storage_caught_nesting test_storage_delegated_nesting test_storage_dirty_keys test_storage_mutation test_storage_root_journal test_gas_budget_journal -v` passed 45 tests with six optional skips. Final `test_storage_warm_rollback test_storage_key_shift -v` passed 16 tests with two optional native skips. These are retained original results, not measurement reruns for documentation.

Independent source review passed before all 32 original native/32 original real quotes and 128 signatures were qualified. Whole-world/account/code/owner-root/witness authentication, actual frame bytes and warm trajectories, EIP150/sentry/prefix/caps, independently solved full R, R0/1/63/64/65 boundaries and sufficient-gas R+1 passed. The independent gate rejected38 semantic negatives, four counter-drop/collapse controls and two extra native-accepted dirty9 mutants. Attempted leaf STOP/work survives ancestor rollback. All invocations bind the single final source below; postactual closure verifies 157 frozen tracked source files unchanged.

Retained qualification commands were `qualify.py --work WORKTREE --source-commit 387345bab9dde72d54300a443864d894907f711e --phase native` and `--phase quote`, `warm_semantics_audit.py`, `default_golden_check.py`, `counter_retention_audit.py` and `close_gate.py`. All exited0. Independent final `actual_audit.py --root CAMPAIGN --qualification QUALIFICATION --freeze-sha256 6f0ab173b616944dff8097cda0472769f3965d7ff0ce35e52dc6703659539aaf --config-sha256 57aa7693c9f2b20d4e3663382fe40391dfea920f88ca45deb52edfe96df7211c --qualification-sha256 069cb0a2da5d8abf513bc6656990f94bbbe706a401113b3ba30760c2779dd1e7 --final`, `focused_final_checks.py` and `finish_actual_audit.py` exited0. Exact argv/stdout are retained privately. The focused audit adapted an old reference-only schema assertion and optional runtime sidecar handling, then repeated read-only calculations; these are not setup/actual failures or retries. No native/quote/VM/SP1 execution was repeated during raw audit.

The independent raw 32/diagnostic 64/cohort236/host 128 audit passed. It reconstructs SDK integer conversion, canonical/public-output/native/quote identities, exact ceilings/scaled contrasts and all five flags. The ten-panel persistent-storage subset has 236 distinct canonical inputs,108 generated full selectors and 100 near-full observations,36 full node gaps, maximum full/all-point gaps 111,328,749/164,072,990 PGU and zero high-priority/complete-proposal gaps. The old204/92/84 snapshot remains historical; reused references are excluded. This is neither a count of the full research study nor IID/global coverage, a confidence bound or a guaranteed maximum. A separate transient-storage pilot is private preparation, not measured or counted here.

All 32 actuals completed once with EXIT0. Maximum wall time was 26.79144994355738 seconds and peak sampled process-tree PSS 4,576,067,584 bytes. Limits remain native 180s, quote 1800s with at most two concurrent quotes, actual sequential 600s/6GiB, logs 16MiB and output 192MiB. Disk preflight 576MiB is a256MiB free floor plus320MiB retention reserve, not a shared-memory budget. The root's read-only host receipt verifies all 128 uniquely owned shared-memory/semaphore objects absent; the independent audit authenticates that receipt, without claiming host-wide cleanup.

| Identity | SHA256 / version |
| --- | --- |
| Final producer source | `387345bab9dde72d54300a443864d894907f711e` |
| Model ID | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Model file | `4cb6c65c6ad28e25e1f30639803958f9555e43d51e6d35d2f1945b0757ce1ac7` |
| Retained ELF / SDK | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034 / 6.3.0` |
| Native executable | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Quote executable | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| SP1 executable | `1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b` |
| Source file inventory | `4e51f7eff0b062bccf32eb0f5069087b094d6814930ee62831886efbada2fd41` |
| Frozen config | `57aa7693c9f2b20d4e3663382fe40391dfea920f88ca45deb52edfe96df7211c` |
| Pre-actual policy | `6f0ab173b616944dff8097cda0472769f3965d7ff0ce35e52dc6703659539aaf` |
| Campaign input/quote index | `bd84a2d70c46891f616947860c0355ad89796b39fbc26f77e1ba37e1717af98f` |
| Independent qualification | `069cb0a2da5d8abf513bc6656990f94bbbe706a401113b3ba30760c2779dd1e7` |
| Original qualification manifest (441 entries) | `142cf364e07e9e3d1d17c2912f7b4b537858c4a464a16a45071bfd038d883fd2` |
| Warm semantics audit | `e186f3e608e5c44d90dd7024961405e506dc6ca4ba648dc50703c02f169f9c03` |
| Counter retention audit | `127c38f9409cfbe8bd532ac5cec0d095d777c9e50470998409d0fb97958021e3` |
| Actual analysis | `689022c0cb5ca619d4b7f555413c9caba72da4789385b712e2ce11845fccbcf6` |
| Gas budget report | `47b3564e75aee1a67a95bd6e80151dc64ff7c0cfb7d467357f00c6d420f5c1b6` |
| Independent raw audit | `9594ed4b5ccfb743e6b41b0133a464916565b13a6d1eff96f9fdf16b4031e1cf` |
| Independent focused checks | `2b0ae82d109baebce7b15947963eab287399b2fce95c15dc1ad819c0534bb0a4` |
| Independent closure | `404b7cbf275c088c27ca53a687069aa0edae2849dbf3a1d1c30fe10fa26a5da2` |
| Independent final report | `5faf65c7dee872c76ae8f53b8ce97451d2ebe05ab815e064d0e638190d83082f` |
| Independent final manifest (456 entries) | `ccca0852114a1a4237e6588c2e7acf10a262bb04b87e12479ce1eb54993a9d2a` |
| Ten-panel storage summary | `99668c6585bbd517f665c030ac3b830874ec4bb5d9c828ebb759a4859c0db8ff` |
| Host shared-memory receipt | `41a0733324cbd7f331c348a3c83660c1b17cd2f310ccf430ae14447b81277a72` |
| Postactual source closure | `701efe5036bd5c5abbff14649877ec69581f5b7189b04a32c908251424e2bdea` |

The ELF declares source revision `4580faf7dc009714b310fd625b5810667019d683`; that binding is not a build attestation or exact-production parity. Model/native/quote/SDK/ELF remain frozen. No Rust, guest, SDK, ELF or pin changes/builds, price fit or pricing change occurred. Execute-only runs generate no proof and establish no GPU throughput or natural-history usability result.

The completed prehead CI run 41626592496 still failed at the existing SDK6.8.1 nested runner's missing binary and stale RISC0 fingerprint. This record does not repair those failures or claim new-head CI success; no guest build was run.

The verified private archive `storage-warm-rollback-evidence-20261103.tar.gz` has SHA256 `4841cacaefb54e96de655410e2b0042728f30ab715f58eaf1e20cef927f454a8`: 7,048,579 compressed bytes, 932 regular members, 90,882,799 payload bytes, 192 restorable duplicate aliases and four external runtime artifacts identified by hash. Every streamed member hash was verified without extraction. It retains original inputs/native/quote/actual reports, source/tools/config/policy/counters, failed read-only audit diagnostics and recovery/host identities. This is private retained evidence, not a public immutable Atlas fixture or standalone auditor runner. No old actual reference is a dependency of this new panel; the historical cohort is a read-only join of its original analyses.

Remaining work includes wider call graphs and exceptional exits, deliberate hashed-path skew/depth and mutation topology, internal lookup/cache distributions, transient storage, heterogeneous packing/order and exact-production-program/natural-history promotion. The panel samples caller prewarming and ancestor rollback only in the selected F4 depth 2/4 graphs. No merge, deployment or on-chain action is part of it.
