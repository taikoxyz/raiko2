# Preallocated bulk MCOPY overlap and relative-residue observations

Twelve new finite inputs retain 11 positive actual-minus-node gaps, including three of four near-full inputs. Maximum full gap is 58,014,088 PGU; maximum across all inputs is 153,288,636 PGU. All four zero controls and all four single points have node gaps. No complete-proposal quote gap or >2B-PGU gap within 30M charged gas appears. These lower-priority observations do not warrant repricing in this increment; the gaps remain explicit TODOs. All12 own-R increments are covered despite the absolute node deficits at near-full gas. The panel does not isolate the source of these absolute deficits and neither proves nor excludes a startup-only cause.

All 12 actual executions completed once, sequentially four zero → four single → four full, without failure, retry or unmeasured case. The dataset is separate from persistent236, transient8, pairing18, defaultSHA18, linearSHA18 and LOG30. No old measurements were rerun, counted as new, relabelled or pooled into a new union.

## Program and finite workload

The [binding specification](../superpowers/specs/2026-11-04-mcopy-bulk.md), [implementation plan](../superpowers/plans/2026-11-04-mcopy-bulk.md) and [producer usage](../../bin/zkgas/bench/README.md#preallocated-bulk-mcopy-layout-probes) describe the reusable compiler-free producer and independent byte interpreter. Corpus `unzen-mcopy-bulk-20261104` uses fixed L=65536, two signed user transactions from one signer with contiguous nonces, and two distinct authenticated code owners tagged0/1, each with empty selected persistent storage. Empty calldata, successful STOP and refund0 apply throughout; no CALL, precompile, CREATE, storage access or REVERT occurs in the user runtimes.

S/D/R/L below mean same-residue-disjoint/different-residue-disjoint/overlap-right/overlap-left. Relative EVM offsets are observable; these names do not assert guest heap base alignment.

| Layout | Source | Destination | Payload extent | Memory words |
| --- | --- | --- | --- | --- |
| same-residue-disjoint | 64 | 65600 | 131072 | 4098 |
| different-residue-disjoint | 64 | 65601 | 131073 | 4099 |
| overlap-right | 64 | 65 | 65537 | 2051 |
| overlap-left | 65 | 64 | 65537 | 2051 |

Counter0 and protected word32 are separate from payload64. A32 is `bytes(i|(tag<<7) for i in range(32))`; B32 is its inverse. Preparation preallocates once with0xee padding, seeds source A and disjoint destination B through MSTORE/logarithmic MCOPY even at R0, and writes the D gap at65600 as0xcd. Overlap seeds A plus its first byte. Loop copies use snapshot semantics in both overlap directions; disjoint copies still execute when their destination bytes already match. Every preparation copy is distinguished from loop copies. Exact final EVM guards check counter plus sorted unique protected/source/destination head, midpoint `base+L//2`, tail `base+L-32`, final aligned word and D's aligned gap word65600. Independent interpretation checks all prepared/final bytes, copy operands/order/content commitments and both owner trajectories. Native exports no final memory: debug final-memory/sequence hashes are intent evidence, not native exports or protocol commitments.

The frozen RV64 little-endian ELF routes MCOPY through `SharedMemory::copy`/memmove, with aligned and unaligned word paths and overlap branches. The assumption that different residues necessarily select an8× byte loop was rejected during source preflight. This source observation chooses probes; it does not attribute actual PGU to a path. The frozen model retains48 PGU/copy-word plus existing dispatch/memory/gas/floor composition. Aggregate quote selectors do not prove per-copy occurrence charging.

## Capacity and gas checks

Full is the largest fitting uniform R for this fixture, not globally optimal transaction packing. Complete gas includes21000 intrinsic, ordinary opcodes including MSTORE8, every preparation/workload MCOPY at `3+3*ceil(n/32)`, and once-only memory cost `3*w+w*w//512`. Each loop bulk copy costs6147 gas before ordinary loop overhead. Refund is0. Full+1 independently executes with sufficient interpreter gas for both owners and fails charged-block sizing in every layout. The separate15M fixture cap,16777216 EIP-7825 cap, ordered31M header admission prefixes and30M charged block cap were checked. Anchor succeeds at167355 gas. All four full selectors meet the derived29.7M near-full criterion.

Zero/single retain their layout's full signed gas envelope; the1000 surplus is fixture continuation margin, not EIP-150 or a storage sentry. Gas-only million-count sizing uses private fixed-width placeholders without simulating bulk memory; selected requests contain exact guards.

| Layout | Full R | User gross=charged | Signed envelope | Charged block | Runtime bytes | Admission remaining before users |
| --- | --- | --- | --- | --- | --- | --- |
| S | 2388 | 14910804 | 14911804 | 29988963 | 908 | 30832645,15921841 |
| D | 2388 | 14910888 | 14911888 | 29989131 | 1003 | 30832645,15921757 |
| R | 2394 | 14911100 | 14912100 | 29989555 | 778 | 30832645,15921545 |
| L | 2394 | 14911100 | 14912100 | 29989555 | 778 | 30832645,15921545 |

## Literal actual and estimates

SDK6.3.0 local execute-only uses `evm-gas-only` and normalized actual PGU `raw_sdk_gas*10//191`. There is one block per proposal: literal node per-block estimate and its literal sum are identical. Complete-proposal quotes remain separate; neither node admissibility at a selected PGU limit nor proof/GPU throughput was measured. Signed gaps below are actual minus literal estimate, retaining surplus as negative rather than hiding it with a zero clamp.

| Case | R | Charged block gas | Raw SDK gas | Actual PGU | Node literal=sum | Proposal quote | Actual−node | Actual−proposal |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| S/zero | 0 | 325227 | 3085222828 | 161529990 | 8522727 | 163515814 | 153007263 | -1985824 |
| S/single | 1 | 337649 | 3086487029 | 161596179 | 8726283 | 163719370 | 152869896 | -2123191 |
| S/full | 2388 | 29988963 | 5962699084 | 312183198 | 494614780 | 649607867 | -182431582 | -337424669 |
| D/zero | 0 | 325395 | 3085582979 | 161548847 | 8542944 | 163536031 | 153005903 | -1987184 |
| D/single | 1 | 337817 | 3088650863 | 161709469 | 8746501 | 163739587 | 152962968 | -2030118 |
| D/full | 2388 | 29989131 | 10436723289 | 546425303 | 494634998 | 649628084 | 51790305 | -103202781 |
| R/zero | 0 | 251287 | 3083194005 | 161423769 | 8135133 | 163128219 | 153288636 | -1704450 |
| R/single | 1 | 263709 | 3086309549 | 161586887 | 8338689 | 163331775 | 153248198 | -1744888 |
| R/full | 2394 | 29989555 | 10571135873 | 553462611 | 495448523 | 650441609 | 58014088 | -96978998 |
| L/zero | 0 | 251287 | 3083193677 | 161423752 | 8135133 | 163128219 | 153288619 | -1704467 |
| L/single | 1 | 263709 | 3086256697 | 161584120 | 8338689 | 163331775 | 153245431 | -1747655 |
| L/full | 2394 | 29989555 | 10450315467 | 547136935 | 495448523 | 650441609 | 51688412 | -103304674 |

Canonical whole-GuestInput SHA256 identities (case labels retain the table meanings):

| Case | Canonical input SHA256 |
| --- | --- |
| S/zero | 8debd5d77c09cbd197b8b9162f69ec47e102c6c097987ccb222469a6c8ae85bd |
| S/single | aeef693e7b8498ee6efb54d04cc166fca22eb64f6adc9cfe80141d7b482e5221 |
| S/full | cae0aaefd56d1869984c81b219f96b45ea4d7505e63d6dc6325f2482da9d82da |
| D/zero | ba6f4ca8359f31666a00466390eb0a14c3aca5f50ba2795c8e40805b3da54a39 |
| D/single | 00f131ea83bc0ce916752ada1378a53bb22f261e1223f95ae9df513dd1cdb367 |
| D/full | 231e757e67ec712fe81188d2d26cefcfb3d32b5956a457aae799fa755b8f9b10 |
| R/zero | 7bc54bd47c8c9267492b645a06ddfd283e23e5bddd80df001f9c9de2108c1e61 |
| R/single | 8fd970aa9a24ad4e7401729f530ba591df666f6569c0aa997d3fb2c49edd2402 |
| R/full | 02d300f0f6e15c36d5900d422fe5d733c7eadd2322744b4252903b26234360a5 |
| L/zero | 0c3794e933a8cfdbf89763b48651cfcd52a22f57ff15a18f8b2875c516483e35 |
| L/single | 31c6f6b2729924a7bfc86eb9154862899a6d9276d387f375ee55618e70c3fc14 |
| L/full | 21be9b47c3ee0b88546a6bd9d822cabfdc7604a9875f76489610c45408c2beec |

Full node/actual ratios span0.895179752260–1.58437348060; complete-proposal/actual ratios span1.17522231145–2.08085467495. These synthetic observations do not establish natural transaction admission usability. All actuals are whole-proposal measurements; there is no actual per-copy PGU export.

## Signed increment diagnostics

The following21 comparisons retain exact scaled node/proposal deltas, each divided by10^18 PGU. Node and proposal scaled differences are equal for every pair here, although subtracting independently rounded literal quotes can differ by1. Both coverage decisions and positive-gap ceilings use the exact scaled values: `delta_actual*10^18 <= delta_scaled`, and `max(0,ceil((delta_actual*10^18-delta_scaled)/10^18))`. Negative actual/estimated deltas stay signed.

Flags, in order, are equal R / copied L / payload extent / source offset / destination offset / charged gas / full signed user envelope / authenticated two-owner selected persistent support;1 means equal and0 means different. Copied L equality does not mean equal selected GuestInput byte length or contents. Every pair has different runtime and complete selected-input bytes. Support equality compares two owner-qualified empty sets, excluding authenticated account/code/global witness work. Own-R controls include preparation and do not subtract a universal startup constant.

| From→to | Actual Δ | Node scaled Δ | Proposal scaled Δ | Both cover | Positive gap ceil | Flags |
| --- | --- | --- | --- | --- | --- | --- |
| S/zero→S/single | 66189 | 203556136160124714886010 | 203556136160124714886010 | 1 | 0 | 01111011 |
| S/zero→S/full | 150653208 | 486092053150377819147791880 | 486092053150377819147791880 | 1 | 0 | 01111011 |
| S/single→S/full | 150587019 | 485888497014217694432905870 | 485888497014217694432905870 | 1 | 0 | 01111011 |
| D/zero→D/single | 160622 | 203556136160124714886010 | 203556136160124714886010 | 1 | 0 | 01111011 |
| D/zero→D/full | 384876456 | 486092053150377819147791880 | 486092053150377819147791880 | 1 | 0 | 01111011 |
| D/single→D/full | 384715834 | 485888497014217694432905870 | 485888497014217694432905870 | 1 | 0 | 01111011 |
| R/zero→R/single | 163118 | 203556136160124714886010 | 203556136160124714886010 | 1 | 0 | 01111011 |
| R/zero→R/full | 392038842 | 487313389967338567437107940 | 487313389967338567437107940 | 1 | 0 | 01111011 |
| R/single→R/full | 391875724 | 487109833831178442722221930 | 487109833831178442722221930 | 1 | 0 | 01111011 |
| L/zero→L/single | 160368 | 203556136160124714886010 | 203556136160124714886010 | 1 | 0 | 01111011 |
| L/zero→L/full | 385713183 | 487313389967338567437107940 | 487313389967338567437107940 | 1 | 0 | 01111011 |
| L/single→L/full | 385552815 | 487109833831178442722221930 | 487109833831178442722221930 | 1 | 0 | 01111011 |
| S/zero→D/zero | 18857 | 20217562007270185373674 | 20217562007270185373674 | 1 | 0 | 11010001 |
| S/single→D/single | 113290 | 20217562007270185373674 | 20217562007270185373674 | 0 | 93073 | 11010001 |
| S/full→D/full | 234242105 | 20217562007270185373674 | 20217562007270185373674 | 0 | 234221888 | 11010001 |
| S/zero→R/zero | -106221 | -387594229529693734454270 | -387594229529693734454270 | 0 | 281374 | 11010001 |
| S/single→R/single | -9292 | -387594229529693734454270 | -387594229529693734454270 | 0 | 378303 | 11010001 |
| S/full→R/full | 241279413 | 833742587431054554861790 | 833742587431054554861790 | 0 | 240445671 | 01010001 |
| S/zero→L/zero | -106238 | -387594229529693734454270 | -387594229529693734454270 | 0 | 281357 | 11000001 |
| S/single→L/single | -12059 | -387594229529693734454270 | -387594229529693734454270 | 0 | 375536 | 11000001 |
| S/full→L/full | 234953737 | 833742587431054554861790 | 833742587431054554861790 | 0 | 234119995 | 01000001 |

All12 own-R increments are covered by both exact scaled estimates. Eight of nine signed other-layout-minus-S contrasts are uncovered by both; maximum upward-rounded increment gap240,445,671 PGU. True flag totals are7/21/12/18/12/0/12/21. Full overlap comparisons change R as well as offsets/extent/gas/envelopes. These diagnostics do not isolate MCOPY cost, heap alignment, a component bound, a tariff defect or a global maximum. Three positive full absolute node gaps and the signed increment deficits remain visible despite absolute complete-proposal coverage.

## Qualification and retained evidence

Source implementation is frozen at `ae4686f9144da0978ff14789ad199a90147fda94`. Independent source spec/quality review passed before canonical release. Author focused tests passed after a retained RED and one corrected oracle preparation-store check; optional native smoke executed exactly three specimens once. Two originals were exact positives (S/single and R/full); the third was L/single changed to selfcopy with recomputed successful guards. Native accepted that mutant but independent original intent rejected it for both tags; it is excluded from canonical inputs and actual sampling. Canonical qualification used10 fresh native calls plus two exact source/request/runtime/GuestInput-positive bridges,12 fresh original quotes and24 authenticated EIP-1559 signatures. All receipts/status/gas/routes, account/code/empty-root/witness/trie and native full/minimized execution-result bindings passed. Independent checks covered24 private synthetic positive trajectories,20 semantic and7 strict-domain negatives,40 canonical-byte negatives,8 producer-domain negatives and38 additional invalid domains; all19532 preparation+loop copies were retained across24 owner trajectories. Both-owner full+1 and no-memory-simulation sizing gates passed.

Provenance corrections remain retained: historical authentication preparation first looked for a later script in the original historical manifest, then correctly pinned that script through the historical final manifest. A private semantic adapter omitted the producer cases import directory and failed after40 byte negatives; only that private check reran after fixing its import. Root requested `expected_last_input`→`final_memory_sha256`; original scripts/summary were copied before renaming24 keys with identical digest values, without repeating native/quote/SP1. Raw closure first stopped on an omitted controller source receipt; root supplied it, then read-only closure resumed. The successful raw audit was not repeated. These were private bookkeeping/import/receipt corrections; no source or measurement was rewritten.

Independent raw audit recomputed all12 results and21 diagnostics from raw reports before reconciling controller analysis. Closure verified172 current source files and committed blobs,292 unchanged original qualification files, a separate298-file final manifest, exact sequential one-attempt order, canonical gzip recovery, native/quote/program/public-output copies and all48 owned resource names/cleanup receipts. A fresh host receipt confirms all48 absent in its observed mount namespace; this is a finite host observation, not general cleanup certification. Peak sampled actual process-tree PSS was2530789376 bytes; maximum actual wall time16.3966871984303s, within6GiB/600s. Native180s/quote1800s,192MiB output,16MiB log and576MiB preflight (256MiB runtime floor plus320MiB reserve) guards were retained. Author native3 retains its stricter576MiB runtime disk floor unchanged.

The identities below are SHA256 unless labeled commit/model ID. R denotes the controller evidence root and Q the independent qualification root; these are private retained evidence labels, not portable paths or public download links.

| Evidence | Identity |
| --- | --- |
| producer | 5d6549950c68f93431cdd09804124d1f1eb05e4f3f73c88294388724b1e7ff68 |
| independent author VM | a5d22363e7587b97bf213d62dcd5ec1c1e7580c60769217a47188b090baf0405 |
| focused tests | aa708e7959c98dd9b8619846754c0698a203c146560b50887dd17819af317043 |
| R/config.json | f096f6b158a9594e005bc54fb0d4e54438da3fe869ce656a57070279352c1008 |
| R/policy-freeze.json | 98054e6fd69f99f181a797b6c5528325593a7e5c17976f88dcbeab77700b8f20 |
| R/source-files-sha256.json | d96dff198e2f7e0b3e1a0cae8c21396ea954fd0402642704eb2f806646d9e8bf |
| R/source-closure.json | 4c641cd448e8c9036e4918899dd46e48c5c0785a3ad209d42bebcfd761fe344d |
| R/analysis.json | d197e92a897d4d3ef9936c7ad519c399a695e940bf04fee0110bc514249d9380 |
| R/gas-budget-report.json | 04b3e2ef02a065f0d7b613a9fae72a21f1353d2fb94725576921fd03789f389a |
| R/host-shared-memory-audit.json | c6274a1b3e29c5092f04969a2320255ff9a0bf777b078e063aa320ad1b4090bc |
| R/prior-contexts.json | 0e916fa9972d800ee13dfa9cabaaaecb2029f72ea94505f92615e9ec6185a887 |
| Q/artifact-manifest.json (original292) | bda616e085fc71ade7b277dc39c7509a604b797f7ec07e6be4f66dde9759af0c |
| Q/final-artifact-manifest.json (final298) | 39ef581b624d209f3d648ccab6c168eb11439396779a81f99030399e310a9fa4 |
| Q/verification.json | 8742b9c09e78d9078fee814e8ffe55120a2a088b18c26d8c28f21fa7eb574b66 |
| Q/qualified-inputs-index.json | 45db78247a394752e2667c75a2b882cf29edab0c0f1ac4ad46ad9cb8d14c85b1 |
| R/qualified-inputs-index.json | 82ac8c46735219b89048dce217d49b95a4f54c24a36d6e56ad436500e79b1142 |
| Q/actual-audit.json | c1ea7f5c27a33d58f47a1c4246f09a507191260430549ed5447d14cf871572fa |
| Q/raw-final-closure.json | 5496f0328781ab9cf2746c9d1a44b6c52423d97ada076d175f6d1496dfdcb93c |
| Q/counter-retention-audit.json | 4a44cfd018dca075f51df6fe2e14aeef955f83d99ae4c7da6bc55d663b4fa35d |
| Q/adapter-naming-correction.json | dc25b6949c1de01416377525099e0b79072e1a6357f6bda26e15543b44085cc0 |
| Q/qualification-adapter-corrections.json | 8dd72fdc2690f6fc87efe38c1446ecc164ac1429818ae5a455a989b4546d5b95 |
| archive receipt | 3988316a04ca1796acd0096e7d04901f350bdded424ef6c414d6a19a61d42b0b |
| model ID | c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef |
| elf artifact | b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034 |
| model artifact | 4cb6c65c6ad28e25e1f30639803958f9555e43d51e6d35d2f1945b0757ce1ac7 |
| native artifact | ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec |
| program_manifest artifact | 8d5b4a482d26e3bdca94c5e66b9de7b33dc57bcfbaa9b9c24140bf030c1f93a9 |
| quote artifact | c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e |
| sp1 artifact | 1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b |
| guest source commit | 4580faf7dc009714b310fd625b5810667019d683 |

Verified private archive `mcopy-bulk-evidence-20261104.tar.gz` is SHA256 `7fbcdcd5af1a30b9957a3dd25859bcf01391742f0b6d9c98cd68be48e1dd6706`,2529817 compressed bytes,499 regular members and30302064 payload bytes;172 duplicate aliases are restorable. Every streamed member hash was verified without extraction. Four large runtime artifacts are external and identified by hash, so this is a private recovery bundle with external dependencies, not a self-contained public reproducibility package. Original292 and final298 inventories retain separate identities and include unchanged original evidence; final closure does not rewrite qualification history.

No Rust, guest, ELF, SDK, dependency pin, model or price was changed or fitted. Execution and byte-intent evidence establish only these selected successful layouts and lengths. Arbitrary lengths/residues, heap placement, memory allocation/growth, return-data copies, nesting/rollback, heterogeneous packing/order and cache effects remain open. Native final-memory exports, isolated actual per-copy PGU, global bounds, IID confidence, proofs/GPU timing, exact current production parity and ordinary historical usability are not established. Subsequent families retain independent evidence identities and the frozen >2B/30M priority rule.
