# Empty-storage warm-read catalogs: finite four-input baseline

All four original execute-only SP1 actuals completed once: two own-zero controls and two maximal single-user points. Two absolute literal node deficits remain, 148,762,567 PGU (ordered zero) and 148,810,164 PGU (mixed zero). Full-point node deficits, complete-proposal deficits and actual-minus-node gaps strictly >2B PGU within 30M charged block gas are zero. Two small mixed-minus-ordered incremental deficits also remain: 47,597/47,542 PGU at zero/full. This finite baseline leaves deliberate map-distribution risk OPEN. No price/model/Rust/native/guest/SDK/ELF/pin/build/cap change follows.

## Catalog intent and single-user capacity

The [binding spec](../superpowers/specs/2026-11-04-warm-read-catalogs.md) defines two deterministic 512-key catalogs, each own-zero/full. Every raw U256 key has low 192 bits zero and eight nonzero high-64 bytes; all keys are distinct, with `0x1122334455667788 << 192` last. Ordered scans valid integers from `0x0101010101010101`; mixed uses the specified SplitMix64 recurrence from state `0x243f6a8885a308d3`, advancing even on rejection. Neither recipe selects buckets, h2, seeds or PGU. Calldata is 16,384 bytes (12,288 zero/4,096 nonzero), ordinary intrinsic gas 135,688. EIP-7623 floor 307,720 is separate and inactive; it is not added to intrinsic.

One signed EIP-1559 CALL, nonce zero, visits existing primary `0x6666666666666666666666666666666666666666` with authenticated EMPTY persistent storage and no extra prestate contracts. User work has no SSTORE, creation, logs, child/precompile calls or refund. The compiler-free runtime depends only on R, not the catalog. It guards calldata length, all 512 cold SLOADs returning zero, then one warm query read and 64 warm zero reads per batch. Memory offsets 0/64/96 hold seed counter/repetition counter/sentinel, with 128-byte peak. Final original query, seed count 512, repeat count R, sentinel and empty-stack STOP are guarded; mismatch executes INVALID. Setup, loop and guard work remain separate from target reads.

| Selector (both catalogs) | R | Bulk reads | Query reads | All user SLOADs | User gross=charged gas | Charged block gas | Signed user gas limit | Near-full ≥99% of 30M |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| own-zero | 0 | 0 | 1 | 513 | 1258351 | 1425706 | 14999051 | false |
| full | 2028 | 129792 | 129793 | 130305 | 14998051 | 15165406 | 14999051 | false |

Anchor gas is 167,355. Full+1 needs a 15,005,826 envelope and fails only the 15M transaction fixture cap; the full envelope is gross+1,000 = 14,999,051. EIP-7825's 16,777,216 cap, 31M header prefix and 30M charged block budget are separate constraints. Own-zero retains its full counterpart's exact signed user transaction while runtime R guards differ. Full user occupancy is 14,998,051/14,999,051, but the block occupies only 15,165,406/30,000,000 = 50.5513533%. This is maximal uniform single-user R under the fixture envelope, not near-30M packing or a 30M capacity certification.

## Literal whole-proposal observations

SDK 6.3.0 `ExecutionReport.gas` normalizes as `raw_gas*10//191`. Raw Unzen scores are never PGU. NodeKnown is literal block-known PGU; Complete is the separate complete-proposal quote. Signed gaps below subtract rounded literal estimates from same-proposal actual. Canonical SHA256 binds bincode (349,441 bytes each).

| Catalog / level | Charged block gas | Raw SDK gas | Actual PGU | NodeKnown PGU | Complete PGU | Signed node gap | Signed complete gap | Canonical input SHA256 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| ordered / zero | 1425706 | 3239265915 | 169595074 | 20832507 | 175905410 | +148762567 | -6310336 | `8bdd684a8339f4cac180bc6ec94ac95b7826b6da666e9cb7a10264bf6776c9aa` |
| ordered / full | 15165406 | 6078532964 | 318247799 | 571333777 | 726406679 | -253085978 | -408158880 | `c8a4178039a892ea2ded046a9360a3d55462f3132ffaf949481be45ad541ac5d` |
| mixed / zero | 1425706 | 3240175021 | 169642671 | 20832507 | 175905410 | +148810164 | -6262739 | `03668edc6432b1020942a54a9491d918aa6a626b0fcf59e7d274e34171737ef2` |
| mixed / full | 15165406 | 6079441029 | 318295341 | 571333777 | 726406679 | -253038436 | -408111338 | `be91ee6b75a6ca4c414bda7da5ec453012ecbae40751465fe4924b9cb277d1ab` |

Full node/actual ratios are 1.79498001826–1.79524816447; complete/actual ratios are 2.28217817049–2.28251909764. Maximum actual is 318,295,341 PGU. These are whole-input observations; the positive control deficits do not isolate startup cost.

## Four signed exact-scaled diagnostics

Scale S=10^18. Signed deficit is `Δactual*S−Δestimate_scaled`; positive gap is `max(0,ceil(deficit/S))`. Node and complete scaled deltas coincide here and are shown once; signed negatives are retained.

Eight flags, in order: same runtime SHA256 / key catalog / query key / repetitions / charged gas / signed envelope / authenticated selected storage support / calldata shape (length/zero/nonzero). True totals are 2/2/4/2/2/2/4/4. Support is exactly one authenticated primary owner mapped to an empty set, excluding logical raw-key equality and account/code/global work.

| Contrast (to−from) | Δactual PGU | Δnode=Δcomplete_scaled | Signed deficit N=P_scaled | Positive gap N/P | Covered N/P | Eight flags |
| --- | ---: | ---: | ---: | --- | --- | --- |
| mixed full−own-zero | +148652670 | +550501269191330363907037752 | -401848599191330363907037752 | 0/0 | true/true | `01100111` |
| ordered full−own-zero | +148652725 | +550501269191330363907037752 | -401848544191330363907037752 | 0/0 | true/true | `01100111` |
| mixed−ordered / zero | +47597 | +0 | +47597000000000000000000 | 47597/47597 | false/false | `10111011` |
| mixed−ordered / full | +47542 | +0 | +47542000000000000000000 | 47542/47542 | false/false | `10111011` |

Both own-zero→full increments are covered; both catalog increments remain uncovered by both estimate differences. Equal runtime and calldata shape across catalogs do not establish equal complete input, signed transaction, numeric seed or map layout. Sender recovery, signatures and earlier input-dependent allocation may affect the guest map seed. No seed/layout observation or selection knob was implemented. These four diagnostics establish neither per-lookup costs, collisions, causal effects, startup-only explanations nor a global hash-table bound. Deliberate key-distribution investigation remains OPEN and unimplemented; a future bounded host observer requires separate design, implementation, independent review/behavior verification and build-feasibility checks. Custom targeted data must form a separate development-probe cohort rather than relabeling this primary holdout.

## Qualification and frozen evidence

Producer source is `ccad5d955dcf9a51d7db1ce7bb6243a75ff08424`, base `54d23bd7631a190fe9c3465208413361fb485e89`. The [plan](../superpowers/plans/2026-11-04-warm-read-catalogs.md), [maintained usage](../../bin/zkgas/bench/README.md), [producer](../../bin/zkgas/bench/cases/warm_read_catalogs.py), [focused harness](../../bin/zkgas/bench/tests/test_warm_read_catalogs.py) and [independent byte VM](../../bin/zkgas/bench/tests/warm_read_catalogs_vm.py) retain the reusable path. Optional custom JSON is bounded at 128KiB before parsing and requires exactly schema_version=1 and 512 canonical constrained keys; validation precedes output creation. Its full-calldata-SHA identity and development_probe status differ from the four original unfitted holdouts. No actual was used for fitting. Five stable helpers are unchanged.

Fresh independent source/spec/quality review passed without source findings. Current and committed scoped inventories match 193 files (127 Python, 47 Rust, 19 other); native's 81 declared files/66 declared Rust/70 compiled workspace entries are separate inventories. Retained focused source evidence is three functional passes with one optional-native skip; the native-enabled run passed all four methods. Exactly three source-native calls ran: ordered/full and mixed/zero positive originals plus an accepted order mutant swapping the first two nonquery calldata keys. The mutant preserves distinct keys, final query, shape, gas, code and zero-read result, but fails original retained catalog-order intent and is excluded from canonical actuals. Native success alone is insufficient intent evidence.

Qualification exported canonical four once, ran two fresh native positives plus two exact original positive bridges, four fresh original quotes and four signature authentications. Explicit original quote index/path/hash/bytes/canonical/manifest-membership checks preceded freeze. Independent byte/calldata/warmset/zero-value/stack/counter/gas-floor checks, six direct/cache R0/1/2 pairs (12 interpretations), 14 semantic negatives, eight authentication negatives and four full+1 checks passed. Complete genesis/root/account/code checks bind initial/final account counts four/five, three required codes and the primary EMPTY storage root. An omitted extra-contract report or zero initialized-storage list does not mean an absent primary owner. Normal feature-error count is zero; exposed counters retain attempted work. Native does not export a per-read key/value trace. The body cache proves all 64 decoded sites and complete stack/warmset/zero-value/read-footprint invariants, checking entry query/warmness/value each iteration; outer control executes directly and real EVM/SP1 executes every read.

The six external qualifier stages (two native/four quote) and four actuals succeeded once without retry. Private pre-integration field mapping failed once, then was corrected. A pure-negative adapter first mutated an already-zero leading immediate byte, producing a no-op and a failed check; an additive corrected script mutates the actual tail byte. Both scripts and failure are retained, with no workload rerun. Independent raw/final/source/host late gates all passed first try. Root metadata readback later used wrong independent host/final-manifest filenames; corrected reads only are recorded in `root-readback-filename-correction.json`, with no artifact/data changes. This does not assert every controller command succeeded first try.

| Identity | SHA256 / value |
| --- | --- |
| Model ID | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| SDK / guest source | `6.3.0 / 4580faf7dc009714b310fd625b5810667019d683` |
| elf artifact | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| model artifact | `4cb6c65c6ad28e25e1f30639803958f9555e43d51e6d35d2f1945b0757ce1ac7` |
| native artifact | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| program_manifest artifact | `8d5b4a482d26e3bdca94c5e66b9de7b33dc57bcfbaa9b9c24140bf030c1f93a9` |
| quote artifact | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| sp1 artifact | `1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b` |
| Source inventory | `a698a646c16f15cb21821eb0968a7fa0fe01a7cf68f344c60fd9ecb5e1e86fb3` |
| Source review | `f64b40d22e308e4e064f5134bad5edd29628a55035fd6e7ed84bbc3bd8fbd330` |
| Original qualification manifest (214 immutable) | `18f823ea16eb505099427d3df44388d29f00bd6c3a2f05f364bc318f167d0528` |
| Final qualification manifest (225) | `5a7a8b9f6b1fc20fe35a0481fe4645331fb5fc8b12acc81b97ff10970c3a35db` |
| Qualification verification | `20b42066c02223e2e22cad9038f70f0ad73531a539380a90a4d7f270c6400557` |
| Analysis | `4458230e615b21e420795cdee85546f6091d984cf0833b031d8b0b97606fdeab` |
| Gas-budget report | `e2b98927dc2b9a9df3502d2ae2859fcaa9c4c7bab914cb08a71ee6240303e2ad` |
| Independent raw4/exact4/eight-flag audit | `4ca2805659e1c21a06d4600552b1d060510f64d5a3f3226d2c8d0121dde0954a` |
| Independent raw final closure | `70e518194ca03ad322cac17eae8208828746998008c9da1c08e84704825140c3` |
| Independent committed source closure | `6e0a254addacbbb020e9e5216c96eb875704431f72f449df105a0a995cbeb745` |
| Independent real-host audit | `9ad287b43b1d2ddcfb9e8493bbe0f9b5cf147638142d825f81549dea3ca70c80` |

The final 225-member qualification manifest preserves all 214 original members unchanged. The first authoritative real-host receipt and independent audit bind all 16 unique runtime names to the recorded device/mount namespace and exact-name lstat absence, without deletion; sandbox absence is not accepted as host evidence. The private recovery archive `warm-read-catalogs-evidence-20261104.tar.gz` has SHA256 `8f22707b9500f9c4cea3ed19012944b8838f860fd73efc5caa5d60124b60d497`, 1,861,851 compressed bytes, 327 regular members, 15,439,369 payload bytes and 68 restorable duplicate aliases. Every member was streamed/hash-verified without extraction. Four runtime artifacts remain external by identity; this is private recovery evidence, not a published standalone Atlas fixture or reproducible deployment.

Limits remained 6,144MiB sampled process-tree PSS, 600-second actual, 180-second native, 1,800-second quote, 16MiB log, 192MiB growing stage, 32/128/64MiB request/canonical/benchmark and 256MiB disk plus 320MiB retention reserve. Peak sampled PSS was 2,367,831,040 bytes; maximum actual wall time 11.029781185090542 seconds. Sampling does not guarantee a continuous peak. Resource failures would remain unmeasured rather than receiving invented PGU; none occurred in these four actuals.

Thirteen prior contexts remain separate and read-only: persistent236, transient8, pairing18, defaultSHA18, linearSHA18, LOG30, MCOPY12, division32, KECCAK27, modular16, coldread12, EXP10 and wordmemory12. Word-memory analysis retains SHA256 `80f0d790bc6884e1b7cad7b84583ad4964a121094990c85c6ad46847f9faffc0`; no old actual was rerun or new union asserted. No proof, GPU, exact production parity, IID confidence, global packing or natural-transaction usability certification follows.

Prehead `54d23bd7` CI run `41747457529` passed model/macOS/fmt/clippy/foundation/pipeline/runtime, while prover failed for the missing SDK 6.8.1 embedded native runner binary and xtask rejected a stale RISC0 source fingerprint. These remain prehead limitations; no new-head all-green or whole-PR readiness claim is made. This documentation step required no Rust test or guest build and repeated no workload, quote, native, SP1 or source test. The held draft PR296 remains do-not-merge; deployment and merge are outside this panel.
