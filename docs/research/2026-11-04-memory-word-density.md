# Dense in-bounds word-memory: finite 12-input panel

All twelve original execute-only actuals completed once: six own-zero controls, then six near-full points. Six literal node deficits remain, all controls; maximum shortfall is 153,488,487 PGU. Full node deficits, complete-proposal deficits and actual-minus-node gaps strictly >2B PGU within 30M charged block gas are zero. Maximum actual is separately 1,582,107,357 PGU. No price, model, Rust, guest, SDK, ELF, pin, build or resource-cap change follows.

## Matrix, memory intent and charged capacity

The finite matrix is MSTORE at relative offsets 0/1 in 256-byte/1MiB arenas, plus MLOAD at offset 1 in both arenas, each own-zero/full. Two signed EIP-1559 CALLs with contiguous nonces 0/1 visit two distinct tagged predeployed owners with empty persistent storage, calldata and value. There are no target child calls, creation, logs or refunds. Predeployment is outside the measured block and is not free over the lifecycle. The arena is runtime memory; canonical inputs are small (about 334kB), not megabyte initial-memory payloads.

Exact `MSTORE8(arena−1,0)` preallocates the arena without an extra word. Counter offset 64 and owner-tag offset 96 are disjoint from target bytes 0..32 and the final arena word. A is 32 `a5` bytes, B is 32 `5a` bytes. Store warmup writes and individually checks A then B; each 64-site batch alternates A/B with `DUP3,DUP2,MSTORE` / `DUP2,DUP2,MSTORE`, preserving `[A,B,offset]` and ending B. Load seeds B, checks one warmup load, then executes 64 `DUP1,MLOAD,POP` sites per batch preserving `[offset]`. Final target B, last-word zero, complete original operands/offset, counter, tag and empty-stack STOP guards all remain. INVALID rejects mismatches.

Bulk target counts are `64R+2` stores or `64R+1` loads per user. Setup, counter and guard occurrences of the same opcodes are counted separately; preallocation is another MSTORE8 event. Each target costs 3 EVM gas plus actual stack/control work. `C_mem(W)=3W+floor(W²/512)`, W=arena/32, is charged once at exact expansion. Anchor costs 167,355. Signed full-user gross+1,000 envelopes, the 15M fixture cap, EIP-7825 cap 16,777,216, 31M ordered header prefixes and 30M charged block budget are distinct constraints. Own-zero retains its full counterpart’s signed envelope.

| Operation / arena / offset | Full R | Targets per user | Full charged block gas | Near-full ≥99% |
| --- | ---: | ---: | ---: | --- |
| load / 1048576 / 1 | 22397 | 1433409 | 29998931 | true |
| load / 256 / 1 | 26269 | 1681217 | 29998915 | true |
| store / 1048576 / 0 | 20126 | 1288066 | 29999939 | true |
| store / 1048576 / 1 | 20126 | 1288066 | 29999939 | true |
| store / 256 / 0 | 23605 | 1510722 | 29999573 | true |
| store / 256 / 1 | 23605 | 1510722 | 29999573 | true |

Both-owner full+1 fails the charged block budget in all six groups. These are maximal uniform 64-site batches, not unrestricted transaction packing.

## Literal whole-proposal observations

SDK 6.3.0 `ExecutionReport.gas` normalizes as `raw_gas*10//191`; raw Unzen is never PGU. NodeKnown is literal block-known PGU, Complete is the separate complete-proposal quote, and signed gaps subtract those rounded literal estimates from same-proposal actual. Canonical SHA256 binds bincode.

| Operation / arena / offset / level | Charged EVM gas | Raw SDK gas | Actual PGU | NodeKnown PGU | Complete PGU | Signed node gap | Signed complete gap | Canonical input SHA256 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| store / 256 / 0 / zero | 210063 | 3081209306 | 161319858 | 7848796 | 162841882 | +153471062 | -1522024 | `da190e2a1510fd6d3406ac6da01d3b8a5a9f32d09401a7f21991ce6332183357` |
| store / 256 / 0 / full | 29999573 | 23884137864 | 1250478422 | 1449503233 | 1604496319 | -199024811 | -354017897 | `827b5852fb8111ea864536a2baae7aef5ee19f1c30c75951013784574c18b74c` |
| store / 256 / 1 / zero | 210063 | 3081159523 | 161317252 | 7848796 | 162841882 | +153468456 | -1524630 | `fb66d3047f1efa7b0a7a9dfed1e9817278de11071c209a650cee9ffc8395e1e5` |
| store / 256 / 1 / full | 29999573 | 26997529473 | 1413483218 | 1449503233 | 1604496319 | -36020015 | -191013101 | `cd113333179d43da6319f93a8c17b3334eb41c26404fd95451694d9a16f8edf7` |
| store / 1048576 / 0 / zero | 4600927 | 3092594944 | 161915965 | 19793192 | 174786279 | +142122773 | -12870314 | `3e73cdd6a6658c9fae801c100b2d992588f8436c03f671942824e4dba26ef5f2` |
| store / 1048576 / 0 / full | 29999939 | 20882964366 | 1093348919 | 1248970790 | 1403963877 | -155621871 | -310614958 | `36a7bb9caa06137e4343e4dd85d6a71f6b56e2a531f03bb1a3290d6307b1aee1` |
| store / 1048576 / 1 / zero | 4600927 | 3092544607 | 161913330 | 19793192 | 174786279 | +142120138 | -12872949 | `a90e42989c8be8cd9f2f44f3e4f8ad84f12e3b8cf2b561427c293008ee519ca4` |
| store / 1048576 / 1 / full | 29999939 | 23537507951 | 1232330259 | 1248970790 | 1403963877 | -16640531 | -171633618 | `2d4ac38d8f0731e35c9e830977ed08f1b59e0327ef6bbd02d0a96485d2232f7a` |
| load / 256 / 1 / zero | 209869 | 3080923806 | 161304911 | 7816424 | 162809511 | +153488487 | -1504600 | `a4461b7e6da4980416e837de3f4102394313a70945dc621ff6c846d45a096851` |
| load / 256 / 1 / full | 29998915 | 30218250523 | 1582107357 | 1858421502 | 2013414589 | -276314145 | -431307232 | `ef6c16e2557a439ef9781c433de051aad27f5f99be7c25c0b129633c26f60cfa` |
| load / 1048576 / 1 / zero | 4600733 | 3092243307 | 161897555 | 19760821 | 174753908 | +142136734 | -12856353 | `1aa9db72e32f2ad5174afa8ff555f2a6ca0a7ebcb0fee772dae39bb5170d817c` |
| load / 1048576 / 1 / full | 29998931 | 26266263469 | 1375197040 | 1597590275 | 1752583362 | -222393235 | -377386322 | `cf11eb8a167e7f1b37ef8fc90aca3ee402eb4da03e6070dd8d9e87dfe1899ccc` |

Full node/actual ratios span 1.01350330472–1.17464942804; complete/actual ratios span 1.13513644773–1.28409499713. Small unaligned store full actual is 1,413,483,218 versus aligned 1,250,478,422 with shared node quote 1,449,503,233; large counterparts are 1,232,330,259 versus 1,093,348,919 with shared node quote 1,248,970,790. These whole-input differences are not isolated per-word allocation costs.

## Twenty signed exact-scaled diagnostics

Scale S=10^18. Signed deficit is `Δactual*S−Δestimate_scaled`; positive gap is `max(0,ceil(deficit/S))`. N/P means node/complete. Below, identical node/complete scaled deltas and deficits are shown once; every signed negative remains visible. Compact sample labels use operation/arena/offset/level.

Eight flags, in order: same operation / arena bytes / residue / repetitions / target count / charged gas / signed envelope / authenticated selected persistent support. True totals are 16/14/16/9/7/4/10/20. Support compares both authenticated owners mapped to empty sets; it excludes account/code/global witness work. Equal gas or signed transactions do not imply equal runtime or complete GuestInput.

| Contrast (to−from) | Δactual PGU | Δnode=Δcomplete_scaled | Signed deficit N=P_scaled | Positive gap N/P | Covered N/P | Eight flags |
| --- | ---: | ---: | ---: | --- | --- | --- |
| store/256/0/full−store/256/0/zero | +1089158564 | +1441654437004061576638316580 | -352495873004061576638316580 | 0/0 | true/true | `11100011` |
| store/256/1/full−store/256/1/zero | +1252165966 | +1441654437004061576638316580 | -189488471004061576638316580 | 0/0 | true/true | `11100011` |
| store/1048576/0/full−store/1048576/0/zero | +931432954 | +1229177597930258135624772696 | -297744643930258135624772696 | 0/0 | true/true | `11100011` |
| store/1048576/1/full−store/1048576/1/zero | +1070416929 | +1229177597930258135624772696 | -158760668930258135624772696 | 0/0 | true/true | `11100011` |
| load/256/1/full−load/256/1/zero | +1420802446 | +1850605077784095755262322628 | -429802631784095755262322628 | 0/0 | true/true | `11100011` |
| load/1048576/1/full−load/1048576/1/zero | +1213299485 | +1577829454000167217275504964 | -364529969000167217275504964 | 0/0 | true/true | `11100011` |
| store/256/1/zero−store/256/0/zero | -2606 | +0 | -2606000000000000000000 | 0/0 | true/true | `11011111` |
| store/256/1/full−store/256/0/full | +163004796 | +0 | +163004796000000000000000000 | 163004796/163004796 | false/false | `11011111` |
| store/1048576/1/zero−store/1048576/0/zero | -2635 | +0 | -2635000000000000000000 | 0/0 | true/true | `11011111` |
| store/1048576/1/full−store/1048576/0/full | +138981340 | +0 | +138981340000000000000000000 | 138981340/138981340 | false/false | `11011111` |
| store/1048576/0/zero−store/256/0/zero | +596107 | +11944396582402342447378598 | -11348289582402342447378598 | 0/0 | true/true | `10111001` |
| store/1048576/0/full−store/256/0/full | -157129503 | -200532442491401098566165286 | +43402939491401098566165286 | 43402940/43402940 | false/false | `10100001` |
| store/1048576/1/zero−store/256/1/zero | +596078 | +11944396582402342447378598 | -11348318582402342447378598 | 0/0 | true/true | `10111001` |
| store/1048576/1/full−store/256/1/full | -181152959 | -200532442491401098566165286 | +19379483491401098566165286 | 19379484/19379484 | false/false | `10100001` |
| load/1048576/1/zero−load/256/1/zero | +592644 | +11944396582402342447378598 | -11351752582402342447378598 | 0/0 | true/true | `10111001` |
| load/1048576/1/full−load/256/1/full | -206910317 | -260831227201526195539439066 | +53920910201526195539439066 | 53920911/53920911 | false/false | `10100001` |
| load/256/1/zero−store/256/1/zero | -12341 | -32371175719679314382174 | +20030175719679314382174 | 20031/20031 | false/false | `01110001` |
| load/256/1/full−store/256/1/full | +168624139 | +408918269604314499309623874 | -240294130604314499309623874 | 0/0 | true/true | `01100001` |
| load/1048576/1/zero−store/1048576/1/zero | -15775 | -32371175719679314382174 | +16596175719679314382174 | 16597/16597 | false/false | `01110001` |
| load/1048576/1/full−store/1048576/1/full | +142866781 | +348619484894189402336350094 | -205752703894189402336350094 | 0/0 | true/true | `01100001` |

All six own-zero→full increments are covered. Two of four residue contrasts remain uncovered (maximum upward-rounded gap 163,004,796 PGU), three of six arena contrasts remain uncovered (53,920,911), and two of four load-minus-store contrasts remain uncovered (20,031), identically for node/complete. Negative full arena actual deltas can coexist with larger negative quote deltas; negative zero-operation actual deltas can also be uncovered. Absolute control deficits and every incremental deficit remain research TODOs, without a startup-only, causal or global-bound claim.

## Source interpretation and observability

The unchanged memory repeat constants already price ordinary work: MLOAD 330.480754086…, MSTORE 246.250401477… and MSTORE8 138.752283445… PGU, including ordinary 64-PGU dispatch before additional growth. `f_mem` is extra growth accounting, not the sole memory price. The selected b76 MLOAD path uses 32 byte loads without an aligned-LD fast branch. MSTORE uses alignment-sensitive 32-byte memcpy. Inspected TLSF backing payloads are at least 16-aligned with root checkpoint zero, supporting selected relative-offset 0/1 source classes. This does not observe actual per-occurrence pointers/branches, prove future allocator/ELF behavior, imply an 8× PGU multiplier or turn source branch counts into PGU.

Independent actual-byte execution proves full stack and exact 32-byte read/write footprint transitions, including alternating A/B writes and final B. Its stateful body cache validates entry stack and relevant memory every iteration; outer control/counter/tag/guards execute directly. Direct R0/1/2 checks for both owners and memory-feedback/offset/value/omission/alias mutants exercise that boundary. Actual EVM/SP1 executes every target. Native gas/routes/receipts and full/minimized execution-result replay bind whole inputs; native does not directly export final memory or per-word values.

## Qualification and frozen evidence

Producer source is `e258b1ef24aa99f060b51ab7d79134fa6b2c5e5a`, parent `d9c028610a4138e34ebe1f2c615a8156e48a2b1e`. The [binding spec](../superpowers/specs/2026-11-04-memory-word-density.md), [plan](../superpowers/plans/2026-11-04-memory-word-density.md), [tool usage](../../bin/zkgas/bench/README.md) and [coverage inventory](zkgas-workload-coverage.md) define scope. Fresh independent source/spec/quality review passed. Current and committed inventories match 190 scoped files: 124 Python, 47 Rust and 19 others. Native’s 81 declared files/66 declared Rust/70 compiled source entries are separate inventories. Five stable helpers remain unchanged.

Focused source checks ran six tests: five passed, one optional-native test skipped, in 12.648 seconds. Separately exactly three native invocations completed once in 26.926 seconds: store/1MiB/offset1/full and load/256/offset1/zero originals, plus one store-full owner mutant changing A from a5 to aa in exactly three decoded initial/warmup-A-guard/final-A-guard contexts. Native accepts its internally consistent changed intent with unchanged gas/routes/final B; original intent rejects it. Original request/intent are preserved, and the mutant is excluded from canonical actuals.

Qualification retains one canonical 12-input export, ten fresh native calls plus two exact original positive bridges, twelve fresh original quotes and 24 signature authentications. Independent byte/stack/memory/gas/root/account/code/empty-storage/witness checks, 12 semantic and nine authentication negatives, 36 direct/cache pairs (72 executions), and 24 both-owner full+1 cases passed. Feature-error count is zero and counters retain all attempted work. Every original quote path/hash/bytes/canonical identity/original-manifest membership is explicitly indexed before freeze. All twelve actuals completed once without external failure or retry.

| Identity | SHA256 / value |
| --- | --- |
| Model ID | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| SDK / guest source | `6.3.0` / `4580faf7dc009714b310fd625b5810667019d683` |
| elf artifact | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| model artifact | `4cb6c65c6ad28e25e1f30639803958f9555e43d51e6d35d2f1945b0757ce1ac7` |
| native artifact | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| program_manifest artifact | `8d5b4a482d26e3bdca94c5e66b9de7b33dc57bcfbaa9b9c24140bf030c1f93a9` |
| quote artifact | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| sp1 artifact | `1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b` |
| Source inventory | `b3622a808329e63118671ee92055ff4c58c632f93f1d3809cc3168165b93f2de` |
| Source native3 summary | `6750fde3113ba46de3fb0f8782b6fb84a95a97a0b6c100e7c316ea07cf37bd0c` |
| Policy freeze | `baeb3ea4161b824c69529a2abc7cadab219af286391a84a8ffdc4b09b54e5a43` |
| Config | `fec05e65a068bc7ae852dbb97daf68f578f213c8adf39627b66d385d855fe01d` |
| Independent qualified input index | `bbf9f9ad725cc74849a89570a6aacadf5916e2233b539d8cf7c69e0831241584` |
| Root qualified input index | `4b3951b730769e510dea708c1b1b0f6c2089ab981639aefda48f644e0ffcd484` |
| Original qualification manifest (354 immutable) | `47601b5800afe5c68cf04da2e006a3647be16036f45260ae88a0dedadac04dc1` |
| Qualification verification | `657a21f80eb78b549ae2684bd2ea83eedbad4e1064feb3bb70d2002062463eb8` |
| Analysis | `80f0d790bc6884e1b7cad7b84583ad4964a121094990c85c6ad46847f9faffc0` |
| Independent raw12/exact20/eight-flag audit | `eaf47d8acf4780ef457a3989b42060420eb51d6cb2fed0e99c1a3e2e3931fbbc` |
| Independent raw final closure | `a0339d51efb2f0a7f61663cf5cccb2e0174399f12675ce421660650b7fda656f` |
| Independent committed source closure | `a6a5531eb8174c0f3806e84d33f7534f64859b32dafe57436306930d1260b913` |
| Root real-host receipt | `4de6c171e82f07f35d183a3e67ff1bf0b91a5d652bb0bc947832d7ad64553284` |
| Independent real-host audit | `9fc77050863420c9a63c9aae7ad546a302fd9885143dfe5afbaae3f62d6669ee` |

Final qualification manifest has 364 files, SHA256 `92f7ea45133ce1d4e0db4d35662cf8435d0c3e421675ff4ccf5d991cdb647c6b`; all 354 original members remain unchanged. Root current source closure is SHA256 `8e4645b2dcbc9da35976520df82d660acf93b88722f96c0e7b8c4734ab511598`. Independent raw/final/source/host closure all passed on the first try without correction or retry. Native, quote, authentication and read-only closure commands also passed on their first attempts for this cohort. The read-only preflight `memory-word-density-preflight-20261104.md` is SHA256 `5af16d2c9f20867f0db2d20e66dcac1b77c28b00e8eede192806717baa4b04c5`; source resolution is not reproducible ELF-build attestation or live enforcement evidence.

The private recovery archive `memory-word-density-evidence-20261104.tar.gz` is SHA256 `737abe2b9daff8b59b52e060eda277a622d1869afd597692c9b2a9fa7d0439b0`, 2,249,385 compressed bytes, 573 regular members, 22,430,648 payload bytes and 220 restorable duplicate aliases. Root streamed and hash-verified every member without extraction. Four runtime artifacts remain external by identity; this is not a published standalone Atlas fixture.

Limits remain 6,144MiB sampled process-tree PSS, 600-second actual, 180-second native, 1,800-second quote, 16MiB log, 192MiB growing stage, 32/128/64MiB request/canonical-input/benchmark, and 256MiB disk plus 320MiB retention reserve. Peak sampled PSS is 4,544,769,024 bytes; maximum actual wall time is 41.039633424952626 seconds. Sampling is not a continuous peak guarantee. The first authoritative host check and independent audit match all 48 original unique runtime names to device 27 and mount namespace 4026531841, then check exact-name lstat absence without deletion. There is no sandbox false-absence history in this panel.

Prior twelve cohorts remain separate, immutable and read-only, including EXP10 analysis `0fa9f59ef898dac311b1e91ebc59d6714e59d77c644227e18a7663c809212571`. No old actual is rerun or joined into a new union. Documentation checks independently recompute retained raw hashes, normalization, literal signed gaps, exact-scaled diagnostics and flags; relative links/path hygiene and scoped diff checks run without workload, native, quote, SP1 or build execution. Pre-increment CI run 41741718783 at parent `d9c028610a4138e34ebe1f2c615a8156e48a2b1e` passed model, macOS, formatting, clippy, foundation, pipeline and runtime. Prover job 126284675550 lacked its SDK6.8.1 native binary; xtask job 126284675443 retained a stale RISC0 fingerprint. Root checked their logs. No CI rerun, build or new-head broad-green claim follows.

These finite execute-only SDK6.3/experimental-ELF observations establish no STARK proof, aggregation, final PLONK/SNARK wrapping, proof/GPU performance, production parity, natural usability, isolated word/memcpy PGU or global price bound. Broader memory values, residues, allocator/nesting histories, growth/repetition mixes and heterogeneous composition remain open. Six absolute node deficits and seven signed incremental deficits remain explicit; no tariff refit is selected.
