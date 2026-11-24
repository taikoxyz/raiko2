# Fixed-operand EXP density and popcount: finite 10-input panel

All ten original execute-only actuals completed once. Nine retain literal node deficits, including four of five near-full points. Maximum all-input and full shortfalls are 153,488,803 and 94,588,500 PGU. Complete-proposal deficits and actual-minus-node gaps >2B PGU within 30M charged block gas are zero. Maximum actual is separately 817,856,124 PGU. No price, model, Rust, guest, SDK, ELF, dependency pin, build or resource-cap change follows.

## Inputs, fixed operands and charged capacity

The finite matrix is dense2=2^16−1, sparse24=2^191, dense24=2^192−1, dense25=2^200−1 and dense32=2^256−1, each at own-zero/full. Every case uses the same odd full-width base `0x` followed by 32 `a5` bytes. Sparse24 and dense24 have equal bit length 192, exponent-byte count 24 and target EVM gas, with popcounts 1 and 192. These are selected values, not worst-case operands.

Two signed EIP-1559 user transactions with contiguous nonces 0/1 call two distinct tagged deployed code owners, each with empty persistent storage, empty calldata and zero value. Each 677-byte runtime retains exponent below base on the stack. A guarded warmup executes even at R0; each loop batch has 64 `DUP2,DUP2,EXP,POP` sites restoring the complete operand pair. Final operand, counter, owner-tag and empty-stack STOP guards reject mismatches. The body has no calls, deployment, storage, logs, refund or result chaining. Predeployment is outside the measured block and is not free over the lifecycle.

Independent actual-byte execution checks stack, results modulo 2^256, gas, counter, tags and terminal behavior. Its host-only body cache requires the complete decoded 64-site stack/gas/count invariant and direct R0/1/2 parity; real EVM/SP1 executes every target. Native status/gas/routes do not export per-EXP operands, results or stacks. Two-owner empty selected persistent support excludes account/code/global authentication.

EXP gas is exactly `10 + 50*L` for exponent-byte length L, with static 10 charged once. Each user executes `64R+1` targets and uses charged/gross gas `21195 + (10+50L) + R*(64*(18+50L)+55)`, with zero refund. Anchor costs 167,355. The common signed envelope within each own-zero/full pair is full user gross+1,000; fixture 15M, EIP-7825 16,777,216, 31M ordered header-prefix admission and 30M charged block budget are distinct constraints.

| Class | Bit length / popcount / L | Full R | Targets per user | Full charged block gas | Near-full ≥99% |
| --- | ---: | ---: | ---: | ---: | --- |
| dense2 | 16/16/2 | 1958 | 125313 | 29998977 | true |
| dense24 | 192/192/24 | 190 | 12161 | 29854825 | true |
| dense25 | 200/200/25 | 183 | 11713 | 29934027 | true |
| dense32 | 256/256/32 | 143 | 9153 | 29844567 | true |
| sparse24 | 192/1/24 | 190 | 12161 | 29854825 | true |

Full+1 fails the 30M charged budget for both owners in every class. These R values maximize uniform 64-site batches under the retained constraints; they do not establish unrestricted transaction packing.

## Literal whole-proposal observations

SDK 6.3.0 `ExecutionReport.gas` is normalized as `raw_gas * 10 // 191`. Raw Unzen scores are never PGU. NodeKnown is the literal block-known PGU sum; Complete is the separate complete-proposal quote. Signed gaps are actual minus the respective rounded literal estimate. N/P flags mean positive node/complete gaps, including controls. Canonical SHA256 binds bincode.

| Class / level | Charged EVM gas | Raw SDK gas | Actual PGU | NodeKnown PGU | Complete PGU | Signed node gap | Signed complete gap | N/P deficit | Canonical input SHA256 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| dense2 / zero | 209965 | 3081012728 | 161309566 | 7820763 | 162813850 | +153488803 | -1504284 | true/false | `14166bad66a4d405917e2baec13a01aef5951e1388c739ed3bc99d7706201c24` |
| dense2 / full | 29998977 | 15304108531 | 801262226 | 779528584 | 934521671 | +21733642 | -133259445 | true/false | `fe55754fa1bd29a53821b7638623600aadfe8eab048643a52f429196dc0a2fb9` |
| sparse24 / zero | 212165 | 3081864848 | 161354180 | 7873693 | 162866779 | +153480487 | -1512599 | true/false | `064bfe63dc6763fc9b31e02d18d1e1ef3a1725563372721f6e37bbb4664f6ea8` |
| sparse24 / full | 29854825 | 15621051982 | 817856124 | 723267624 | 878260711 | +94588500 | -60404587 | true/false | `c39f9629db2fa8ac47975ccc8dd4edfd4aac4bce58b9dbe6f9c71100a3860864` |
| dense24 / zero | 212165 | 3081770921 | 161349262 | 7873693 | 162866779 | +153475569 | -1517517 | true/false | `be091a3d928c0f482d916629db7e1e22dd6d05b80796e99a1b1c3a94277a3d7b` |
| dense24 / full | 29854825 | 14452522278 | 756676559 | 723267624 | 878260711 | +33408935 | -121584152 | true/false | `2d0d84f9350dd2075b607e06ba492854ae596904aab42296398c2d16e91555b6` |
| dense25 / zero | 212265 | 3081858749 | 161353861 | 7876403 | 162869489 | +153477458 | -1515628 | true/false | `f64c30ae3cf2f11777e76b4d83393d03931ac3281195dd082145bb77a964523d` |
| dense25 / full | 29934027 | 14479521848 | 758090149 | 728652020 | 883645107 | +29438129 | -125554958 | true/false | `67fb6e23e5ce9b306ce07928933d9c0c93dab7c35050db9205f21d83f448c028` |
| dense32 / zero | 212965 | 3082130746 | 161368101 | 7904429 | 162897516 | +153463672 | -1529415 | true/false | `a24a4f33f658387d8e617b3895260f59e121752c1b67114de483041ea269de20` |
| dense32 / full | 29844567 | 14428735407 | 755431173 | 827634709 | 982627795 | -72203536 | -227196622 | false/false | `2ac2ccb118d37a86d33613936cfe7bd90588990312e6f6fa685b84884c03e7d8` |

Full node/actual ratios range from 0.884345794787 to 1.09557923816; complete/actual ratios range from 1.07385722895 to 1.30075092228. Four full node ratios are below one. Covered own increments do not erase absolute deficits. Sparse24 full actual (817,856,124) exceeds dense24 full (756,676,559) by 61,179,565 PGU despite identical node and complete quotes. Source multiplication count is not a monotonic PGU ranking.

## Thirteen signed exact-scaled diagnostics

Scale S=10^18. Signed deficit is `Δactual*S − Δestimate_scaled`; positive-gap ceiling is `max(0, ceil(deficit/S))`. Signed negative contrasts remain visible. These are five own-zero→full and eight exponent-class whole-input contrasts, not isolated EXP/bit/multiply costs. Full signed transactions and per-owner support maps remain in the independent raw audit.

Eight flags in order: same base / exponent / exponent-byte length / repetitions / target count / charged gas / signed envelope / authenticated selected persistent support. Their true totals are 13/5/7/5/5/2/7/13. Support compares both authenticated owners mapped to empty selected persistent sets; it excludes account/code/global work. Equal envelopes or gas do not imply equal code or complete GuestInput.

| Contrast (to minus from) | Δactual PGU | Δnode_scaled | Δcomplete_scaled | Signed node deficit_scaled | Signed complete deficit_scaled | Positive gap N/P PGU | Covered N/P | Eight flags |
| --- | ---: | ---: | ---: | ---: | ---: | --- | --- | --- |
| dense2 full−own zero | +639952660 | +771707820780013382312987896 | +771707820780013382312987896 | -131755160780013382312987896 | -131755160780013382312987896 | 0/0 | true/true | `11100011` |
| dense24 full−own zero | +595327297 | +715393931673354963071701720 | +715393931673354963071701720 | -120066634673354963071701720 | -120066634673354963071701720 | 0/0 | true/true | `11100011` |
| dense25 full−own zero | +596736288 | +720775617283391377143956940 | +720775617283391377143956940 | -124039329283391377143956940 | -124039329283391377143956940 | 0/0 | true/true | `11100011` |
| dense32 full−own zero | +594063072 | +819730279125038412301165996 | +819730279125038412301165996 | -225667207125038412301165996 | -225667207125038412301165996 | 0/0 | true/true | `11100011` |
| sparse24 full−own zero | +656501944 | +715393931673354963071701720 | +715393931673354963071701720 | -58891987673354963071701720 | -58891987673354963071701720 | 0/0 | true/true | `11100011` |
| zero dense24−sparse24 | -4918 | +0 | +0 | -4918000000000000000000 | -4918000000000000000000 | 0/0 | true/true | `10111111` |
| full dense24−sparse24 | -61179565 | +0 | +0 | -61179565000000000000000000 | -61179565000000000000000000 | 0/0 | true/true | `10111111` |
| zero dense25−dense24 | +4599 | +2709896187275490461978 | +2709896187275490461978 | +1889103812724509538022 | +1889103812724509538022 | 1890/1890 | false/false | `10011001` |
| full dense25−dense24 | +1413590 | +5384395506223689562717198 | +5384395506223689562717198 | -3970805506223689562717198 | -3970805506223689562717198 | 0/0 | true/true | `10000001` |
| zero dense32−dense25 | +14240 | +28026796859092021165028 | +28026796859092021165028 | -13786796859092021165028 | -13786796859092021165028 | 0/0 | true/true | `10011001` |
| full dense32−dense25 | -2658976 | +98982688638506127178374084 | +98982688638506127178374084 | -101641664638506127178374084 | -101641664638506127178374084 | 0/0 | true/true | `10000001` |
| zero dense24−dense2 | +39696 | +52929446331355945037184 | +52929446331355945037184 | -13233446331355945037184 | -13233446331355945037184 | 0/0 | true/true | `10011001` |
| full dense24−dense2 | -44585667 | -56260959660327063296248992 | -56260959660327063296248992 | +11675292660327063296248992 | +11675292660327063296248992 | 11675293/11675293 | false/false | `10000001` |

All five own increments are covered by both exact estimates. Two class contrasts remain uncovered by both: zero dense25−dense24 (1,890 PGU ceiling) and full dense24−dense2 (11,675,293 PGU ceiling). The latter has negative actual delta −44,585,667 PGU and a larger negative quote delta; uncovered does not mean every actual delta is positive. Neither margins nor controls establish a startup-only cause, per-event PGU or a causal/global bound.

## Source interpretation

The selected ruint 1.17.2 `wrapping_pow` source performs h squarings plus popcount result multiplications and has no base-0/1 fast exit. This source work count is neither PGU nor the node selector. The unchanged c893 evaluator uses the maximum of its curve and `24*rawgas`, then ordinary 64-PGU dispatch: L≤24 remains on the floor here and L≥25 enters the quadratic regime. Historical formal calibration used base 2 and minimum single-bit exponents, not these dense exponents; old points are not recertified. Selected source resolution and declared ELF provenance do not attest a reproducible build.

The retained b76 ELF independently shows three EXP zero-comparison call sites: gas checking, initial power condition and post-shift condition. They call a 48-byte bytewise `memcmp` against 32 zero bytes, reading low-address bytes until a difference. Little-endian sparse24 has more low-address zero bytes than dense24. Closed path arithmetic gives 2,456 versus 225 examined bytes per EXP, despite equal 192 power-loop iterations. Across 24,322 user EXP events it predicts 108,524,764 additional sparse LBU and BNE instructions each. Dense instead performs 191 additional result products per event. Whole-input counters reconcile as follows; these are instruction counts, never PGU.

| Whole-input opcode | Sparse24 full | Dense24 full | Predicted signed difference | Observed signed difference / residual |
| --- | ---: | ---: | ---: | ---: |
| BNE (sparse−dense) | 120490270 | 11965506 | +108524764 | +108524764 / 0 |
| LBU (sparse−dense) | 133318105 | 24793308 | +108524764 | +108524797 / +33 |
| MUL (dense−sparse) | 43269757 | 89724774 | +46455020 | +46455017 / −3 |
| MULHU (dense−sparse) | 19662645 | 47535657 | +27873012 | +27873012 / 0 |

Both syscall maps are identical. The retained-source/ELF path and whole-input aggregate pattern support this mechanism and show why h+popcount is an insufficient ranking heuristic. They do not allocate all 61,179,565 PGU to this function or establish isolated per-bit/per-EXP costs or a causal bound. The read-only source/binary report `exp-sparse-condition-preflight-20261104.md`, SHA256 `7f2eccd1084fcdd6966ade60ae3cf5392a938d5cab897d6ea837b526e7dc9a00`, is retained in the private archive; no execution or build was repeated.

## Qualification and frozen evidence

Producer source is `be1bd0b065ae107478015ab7b7d252d7e14cbc1c`, parent `21352765f7f830f6926b56dc32c327290fada624`. The [binding spec](../superpowers/specs/2026-11-04-exp-density.md), [plan](../superpowers/plans/2026-11-04-exp-density.md), [tool usage](../../bin/zkgas/bench/README.md) and [coverage inventory](zkgas-workload-coverage.md) define scope. Independent source/spec/quality review passed. Current and committed source closure checks 187 tracked files: 121 Python, 47 Rust and 19 others. Native declared 81/66 Rust/compiled 70 are a separate inventory, not an alternative 187-file taxonomy.

Six focused source test groups passed; optional native was skipped in that suite. Separately, exactly three native invocations ran once in 8.364 seconds: dense32 zero and dense24 full originals, plus a dense32-zero one-owner exponent mutant accepted by native but rejected against unchanged original intent. The mutant changes precisely two decoded exponent contexts to `(2^256−1)−2^254`, preserving bit length, gas and exact modular result; it is excluded from canonical actuals. A private count label was corrected before qualification, preserving original source and native receipts without reruns.

Canonical qualification retained eight fresh native calls plus two exact original positive bridges, ten fresh original quotes and twenty signature authentications. Independent byte/root/account/code/witness checks, nine semantic and nine authentication negatives, thirty direct/cache checks and twenty both-owner full+1 checks passed. Every original quote hash/canonical binding/manifest membership is explicit in the qualified-input index before freeze. All five zero actuals then all five full actuals ran once, without external failure or retry.

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
| Source inventory | `1e0cc1d18b0b1fb9590bbbdd2ce42cf1f0d03553bece369b87729e9e522e8b5c` |
| Source native3 summary | `3a9be4f1eabb3c7417020b3cfc983c14031394170ac1c1a44c0bea450e16b266` |
| Policy freeze | `733cd072fb455fd838e028d8fd1bd3b81b89552cb907c3643285c79c6b9c70a0` |
| Config | `bf6515b0f44c537fd9b7511e0a1a11e994c5e42bf99383ec978df7c84ad0bb71` |
| Independent qualified input index | `2e9467f8bafd42ba02956315ba292b59fb3bd7ac0fcabbe45cf560f2dfb497db` |
| Root qualified input index | `978033d3468724996794162da8a65bf097c1c75c38e276f579c691326a946817` |
| Root source closure | `7d8e01f97cfcdcfdf727ac0d47ab21355883424591c7d1fd95f1d78d023d03a6` |
| Root real-host receipt | `29b5cdc804349613e0dc5ddbf99ea4f06bd3dc72fccf422ebc2ef22b6e01b4c3` |
| Original qualification manifest (316 unchanged) | `d059b4064a44f1c5dd41c53fb6e6772cf5ab1ad8254514206963d4620740f839` |
| Analysis | `0fa9f59ef898dac311b1e91ebc59d6714e59d77c644227e18a7663c809212571` |
| Final qualification manifest (329 files) | `bf9dc331512696f40f37a8d4ed1630e936e879ba315e420397fb62e2f547c605` |
| Independent raw10/exact13/eight-flag audit | `5736add57b60f6503b10696385f0f0b51d658c1b38ebf95e7c84eb28939363d0` |
| Independent raw final closure | `4e4dca2df2546d27c9b65d969bbdacdbfeacd7c5ccbee0445424c693c8f8dc01` |
| Independent committed source closure | `94c3b1d2c63b52c737e1441d6d8b5f1b6dd9ed1e9b753218927c35990fc36776` |
| Independent real-host audit | `940d40a4030d377bad5d49588f228244dfe121cbf4d11eb00bbe72d9fb730183` |

The original 316 qualification members remain immutable; final closure has 329 members. A private final provenance adapter expected `cold12` instead of the original `coldread12` context key. The preserved traceback and old script plus additive wrapper resolve only that alias to the same pinned context; no raw/source/host/workload execution was repeated.

The private recovery archive `exp-density-evidence-20261104.tar.gz` is SHA256 `9b7378519b575553d41ed242d07a9607c3cf7e499f67ea227711fdcfd9132455`, 2,012,049 compressed bytes, 510 regular members, 19,773,035 payload bytes and 182 restorable duplicate aliases. Root streamed and verified every member hash without extraction. Four runtime artifacts remain external by identity; this is not a published standalone Atlas fixture.

Limits remain 6,144 MiB sampled process-tree PSS, 600-second actual, 180-second native, 1,800-second quote, 16 MiB log, 192 MiB growing stage, 32/128/64 MiB uncompressed request/native-input/benchmark, and 256 MiB disk plus 320 MiB retention reserve. Peak sampled PSS is 3,482,109,952 bytes and maximum actual wall time 23.11618898063898 seconds. Sampling is not a continuous peak guarantee.

The first authoritative real-host check and independent audit match all 40 unique original runtime-owned names to device 27 and mount namespace 4026531841, then verify exact-name lstat absence without deletion. This panel has no sandbox false-proof correction history. Prior eleven cohorts remain unchanged and separately counted, including coldread12 analysis `d9e51162c70255b1114bf2d94749cf611cef1bd2ff6583ed303e2c2d4713bcb7`; no old actual is rerun or joined into a new union.

This documentation pass independently recomputes all ten raw hashes/normalized actuals, literal signed rows, thirteen exact-scaled diagnostics and eight flags from retained data. Arithmetic/hash checks, relative links, path hygiene and `git diff --check` are checked without producer/native/quote/SP1/build execution. Pre-increment CI run 41736454292 at parent `21352765f7f830f6926b56dc32c327290fada624` passed model, macOS, formatting, clippy, foundation, pipeline and runtime; prover lacked the SDK6.8.1 runner and xtask had a stale RISC0 fingerprint. No new-head broad-green claim, CI rerun or build follows.

These finite execute-only SDK6.3/experimental-ELF observations establish no STARK proof, aggregation, final PLONK/SNARK wrapping, proof/GPU performance, production parity, reproducible build, global price sufficiency or natural-transaction usability. Nine absolute node deficits and two signed class deficits remain explicit research TODOs.
