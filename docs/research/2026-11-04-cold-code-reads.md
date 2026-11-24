# Honest cold code reads: finite 12-input pilot

All 12 original execute-only actuals completed once. Ten inputs retain literal node deficits, including four of the six selected points. Maximum all-input and selected node shortfalls are 154,197,096 and 139,817,596 PGU. Complete-proposal deficits and actual-minus-node gaps >2B PGU within 30M charged gas are both zero. Maximum actual is 904,063,041 PGU, distinct from the maximum shortfall. No price, model, Rust, guest, SDK, ELF, dependency pin, build or resource-cap change follows.

The initial missing-node-code hypothesis is falsified for this retained source graph: selected EXTCODESIZE already contributes every materialized target body to the node's Shadow collector at transaction finalization. The instruction observer alone does not describe the complete collector. These source-backed observations do not attest a live deployment or admission enforcement.

## Inputs and honest witness

The finite matrix is footprint K=16/128/512 × BALANCE (0x31)/EXTCODESIZE (0x3b) × own-zero/selected. Every prestate retains K target contracts, even at zero; selected visits every target once, zero visits none. Each block has one signed EIP-1559 user transaction, nonce 0, empty calldata and zero value, executing one primary program. Anchor is separate. Targets have zero balances and empty persistent storage, with unique 24,576-byte legacy code: PUSH32(tag=i+1), STOP, then 24,542 JUMPDEST bytes. Consecutive target addresses start at `0x4000000000000000000000000000000000010000` and are disjoint from primary, sender, system and precompiles. Predeployment occurs outside the measured block and is not free over the lifecycle.

The compiler-free loop derives BASE+index, checks each BALANCE result 0 or EXTCODESIZE result 24,576, increments its memory counter and separately checks final counter and empty-stack STOP. Invalid guards fail. Targets never execute a frame; there is no target CALL/DELEGATECALL, CREATE, LOG, persistent write or refund. Witness preloading does not prewarm EIP-2929 access. The byte-derived gas uses the frozen pre-Amsterdam 2,600-gas cold touch, not a future-fork gas schedule.

Authenticated minimality requires exactly K target code bodies for size-selected, and zero unused target bodies for BALANCE and own-zero, plus independently identified program/system codes. Configured account commitments still exist in all cases. This removes unused code witness without equating account/code/global support. Independent qualification binds signed transactions, configured genesis, visited accounts, target content/hash/tag order, roots, witness, gas, counter and receipts; the public producer/VM is not its authority. Native status/gas/routes and full/minimized output equality do not directly export per-read values, order or target-frame events.

Source qualification retained five passing focused test groups and exactly three native invocations: size-K16 selected and balance-K128 selected originals, plus a size-K16 same-length wrong-tail mutant accepted by native but rejected against original content intent. The mutant is excluded. An accidentally started broad discovery suite was stopped while in an old KECCAK test (exit 130); no broad-suite pass or rerun is claimed. Canonical qualification uses ten fresh native calls plus two exact original positive bridges, twelve fresh original quotes, twelve signature authentications, seventeen byte/intent negatives and nine authentication negatives. A separate supported JSON missing-required-code quote fails with expected exit 2 and `proposal_quote`; it is not one of the twelve original quotes and has no actual PGU.

## Literal whole-proposal observations

SDK 6.3.0 `ExecutionReport.gas` is normalized as `raw_gas * 10 // 191`. Raw Unzen scores are separate units. NodeKnown is the literal block-known PGU sum; Complete is the complete-proposal PGU quote. Signed gaps are actual minus the respective rounded literal estimate. N/P flags are true exactly for positive node/complete gaps, including controls. Canonical input SHA256 binds bincode.

| Operation | K | Visits | Charged EVM gas | Raw SDK gas | Actual PGU | NodeKnown PGU | Complete PGU | Signed node gap | Signed complete gap | N/P deficit | Canonical input SHA256 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| BALANCE | 16 | 0 | 188422 | 3078172267 | 161160851 | 7506475 | 163135098 | +153654376 | -1974247 | true/false | `07324d2556d81df34039948378f1e16db00ad48dd6fd4764b73762bf06b09d5c` |
| BALANCE | 16 | 16 | 231446 | 3087365064 | 161642149 | 21824553 | 177779672 | +139817596 | -16137523 | true/false | `887dca4506539b3b8b316671d0149753aae1afc604596738c86851136d4fd973` |
| BALANCE | 128 | 0 | 188422 | 3082482188 | 161386501 | 7506731 | 163267890 | +153879770 | -1881389 | true/false | `f8ddc19f4dd0dd060fba4a3ce92c36686c5fc9c2343a01fc6aae95e191384754` |
| BALANCE | 128 | 128 | 532614 | 3172693433 | 166109603 | 122051357 | 281748436 | +44058246 | -115638833 | true/false | `bea1b87f727ada334a7b4c8d5ca6fa7b72e80cea703346b7c7a49e304665cca1` |
| BALANCE | 512 | 0 | 188422 | 3088543096 | 161703827 | 7506731 | 164071682 | +154197096 | -2367855 | true/false | `f31ab399786e3469f586d79ed90b4c57a14c8c179f54c1d4a45cd7fda5993732` |
| BALANCE | 512 | 512 | 1565190 | 3473907467 | 181879972 | 465685237 | 640176683 | -283805265 | -458296711 | false/false | `ee7d591e956580750b9d21851699cbc72ad6de700d058f43077c85b26c08a564` |
| SIZE | 16 | 0 | 188422 | 3078191359 | 161161851 | 7506475 | 163135098 | +153655376 | -1973247 | true/false | `4abd4ac4bf8d5c438a5bc1e6105cf5ed73923a2169ebc1236f9888788d6ed021` |
| SIZE | 16 | 16 | 231446 | 3500384842 | 183266222 | 48997900 | 204953019 | +134268322 | -21686797 | true/false | `e31fb4b3ff7edb6e2461b483d3a52ea008fa579afcfdfa8412ba7ede8d8e6875` |
| SIZE | 128 | 0 | 188422 | 3082490261 | 161386924 | 7506731 | 163267890 | +153880193 | -1880966 | true/false | `4d4ea15c7bc5dfd1577038d49858f2d6b7a4bd1c48ad3ec4e272ca6f2413892b` |
| SIZE | 128 | 128 | 532614 | 6521042562 | 341415840 | 339438129 | 499135208 | +1977711 | -157719368 | true/false | `2ebaeb4f64b0a7401914a1287e849fa70d721952df3465e0d4a762cf0b384649` |
| SIZE | 512 | 0 | 188422 | 3088482212 | 161700639 | 7506731 | 164071682 | +154193908 | -2371043 | true/false | `79ef52b4f9e74474ac64b433ee6fafe8ac418c43b0ff617e59535118eac6ed24` |
| SIZE | 512 | 512 | 1565190 | 17267604087 | 904063041 | 1335232323 | 1509723770 | -431169282 | -605660729 | false/false | `56501ade875d28e39c2609cc411fc3271190669372fbb7dbb3b23b0dc7c0bac4` |

Selected node/actual ratios range from 0.135017711253 to 2.56039866226; selected complete/actual ratios range from 1.09983487042 to 3.51977557485. Four selected node ratios are below one. Covered increments do not erase absolute deficits.

## Unsaturated footprint and collector source correction

This is a selected-footprint pilot. All block charged gas is at most 1,565,190, far below 30M; none is near-full. K512 is a fixture cap, not a protocol bound, a 30M maximum, a global capacity measurement or a measured larger-footprint projection. Zero user gas is 21,067 and selected user gas is 21,067+2,689K; Anchor costs 167,355. Refunds are zero. For each K, the common signed envelope is selected gross gas+1,000 across both operations and levels. Fixture 15M envelope, EIP-7825 16,777,216 envelope, 31M header-prefix admission and 30M charged-block budget are checked separately. No full+1 claim is made.

The exact retained c10 quote provenance binds effective Alethia revision `e01c43ff2e79ef75ac9089bb8692093e9c5cbda3`. Selected host/declared guest resolution is revm43.0.3, interpreter43.0.2 and precompile43.0.3; declarations are not an ELF build attestation. The quote's override-resolved lock and native workspace resolution are separate sources.

EXTCODESIZE loads the account with code; BALANCE loads without code. Although the 0x3b opcode observer emits only Account, `TaikoEvmWrapper::transact_raw` finalizes the journal after either plain or inspected execution. It iterates returned accounts, including materialized read-only accounts, and emits `StateAccess::Code` for `account.info.code = Some(code)`. The common accumulator deduplicates by code hash. Shadow block execution commits this collector's transaction scope. Thus opcode hooks are one source of code observations, not the complete source.

Literal Node and Prove exact unique code bytes are equal here: 6,873 for all zero/BALANCE cases, then 400,089 / 3,152,601 / 12,589,785 for size-selected K16/128/512. These equal `6873 + K*24576`; unique hashes grow from 3 to 3+K. All twelve feature-error scaled selectors are zero. K16 selected size-minus-BALANCE code resources alone add `16*24576*64 + 16*2971 = 25,213,360` PGU, confirming actual resource charging under existing 64 PGU/byte and code-hash tariffs. Other resource/event differences remain separate.

The frozen host quote obtains NodeKnown from the committed report and separately completes Prove from exact resources; completion does not backfill Node selectors. The narrower opcode-hook observation was correct, but its global missing-node-code conclusion was false. The supported boundary is retained-source Shadow observability, including ordinary non-inspected execution. Shadow is non-enforcing; this is not a measurement of live node admission or an attested clean production build.

## Twelve signed exact-scaled diagnostics

Scale S=10^18. Signed deficit is `Δactual*S − Δestimate_scaled`; positive-gap ceiling is `max(0, ceil(deficit/S))`. Both exact signed estimate differences are retained below without rounding. These are six own-zero→selected and six size-minus-BALANCE whole-input contrasts, not isolated byte/opcode costs. Full signed transaction objects and selected support maps are retained in the raw diagnostic audit.

Eight flags in order: same prestate target count / visits / target code bytes / target code pattern / operation / charged gas / full signed envelope / authenticated selected storage support. Their true counts are 12/6/12/12/6/6/12/12. Support means exactly one authenticated primary owner mapped to an empty selected persistent-storage set; it excludes every target, account/code and global witness domain. Equal envelopes and storage flags do not imply equal runtime or complete GuestInput.

| Contrast (to minus from) | Δactual PGU | Δnode_scaled | Δcomplete_scaled | Signed node deficit_scaled | Signed complete deficit_scaled | Positive gap N/P PGU | Covered N/P | Eight flags |
| --- | ---: | ---: | ---: | ---: | ---: | --- | --- | --- |
| BALANCE K128 selected−own zero | +4723102 | +114544626426741295979657856 | +118480546426741295979657856 | -109821524426741295979657856 | -113757444426741295979657856 | 0/0 | true/true | `10111011` |
| BALANCE K16 selected−own zero | +481298 | +14318078303342661997457232 | +14644574303342661997457232 | -13836780303342661997457232 | -14163276303342661997457232 | 0/0 | true/true | `10111011` |
| BALANCE K512 selected−own zero | +20176145 | +458178505706965183918631424 | +476105001706965183918631424 | -438002360706965183918631424 | -455928856706965183918631424 | 0/0 | true/true | `10111011` |
| SIZE K128 selected−own zero | +180028916 | +331931397987811632220605056 | +335867317987811632220605056 | -151902481987811632220605056 | -155838401987811632220605056 | 0/0 | true/true | `10111011` |
| SIZE K16 selected−own zero | +22104371 | +41491424748476454027575632 | +41817920748476454027575632 | -19387053748476454027575632 | -19713549748476454027575632 | 0/0 | true/true | `10111011` |
| SIZE K512 selected−own zero | +742362402 | +1327725591951246528882420224 | +1345652087951246528882420224 | -585363189951246528882420224 | -603289685951246528882420224 | 0/0 | true/true | `10111011` |
| K16 zero SIZE−BALANCE | +1000 | +0 | +0 | +1000000000000000000000 | +1000000000000000000000 | 1000/1000 | false/false | `11110111` |
| K16 selected SIZE−BALANCE | +21624073 | +27173346445133792030118400 | +27173346445133792030118400 | -5549273445133792030118400 | -5549273445133792030118400 | 0/0 | true/true | `11110111` |
| K128 zero SIZE−BALANCE | +423 | +0 | +0 | +423000000000000000000 | +423000000000000000000 | 423/423 | false/false | `11110111` |
| K128 selected SIZE−BALANCE | +175306237 | +217386771561070336240947200 | +217386771561070336240947200 | -42080534561070336240947200 | -42080534561070336240947200 | 0/0 | true/true | `11110111` |
| K512 zero SIZE−BALANCE | -3188 | +0 | +0 | -3188000000000000000000 | -3188000000000000000000 | 0/0 | true/true | `11110111` |
| K512 selected SIZE−BALANCE | +722183069 | +869547086244281344963788800 | +869547086244281344963788800 | -147364017244281344963788800 | -147364017244281344963788800 | 0/0 | true/true | `11110111` |

All six own-zero increments are covered by both exact estimates. Two zero-mode contrasts remain uncovered by both, by 1,000 PGU at K16 and 423 PGU at K128; the signed negative K512 zero contrast also remains visible. Neither the covered increments nor small mode deficits select a cause, startup-only interpretation, per-byte actual PGU or global bound. The ten absolute node deficits remain explicit TODOs.

## Frozen identities and retained evidence

Producer/source revision is `2798cb7db65b3543f07b72e44d638ed43632067b`. The [binding spec](../superpowers/specs/2026-11-04-cold-code-reads.md), [plan](../superpowers/plans/2026-11-04-cold-code-reads.md), [maintained tool usage](../../bin/zkgas/bench/README.md) and [coverage inventory](zkgas-workload-coverage.md) define scope. Independent source/spec/quality review passed without findings. Source closure checks all 184 inventory files against current files and exact committed blobs. Source, tools, canonical inputs and all-six-zero then all-six-selected order were frozen before actuals.

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
| Source inventory | `d374f531d510a8b7201983d616929e5ab0b92d5f46227ef7170ac1c6d9fc7aa2` |
| Policy freeze | `f52e4878d5945e41bc1a1e9ce67e8e4220690910bd45b9a4a52cb2ee754b0a02` |
| Config | `e8302aa5e16e85ec975e0ea2c7cda15003698db411a25a2bb940851776031ed1` |
| Original qualified input index | `493a545e0e5f4c29fc20b6c87493d8e846b55bfb0aa38faeb2e474363a0486c0` |
| Original qualification manifest (369 unchanged) | `6b280aa46f6f3132112af46a1d80b562e65427dfa5c012f4dcf2b10c84b951c6` |
| Original quote supplement | `8fbbd0aa86135543f83993191b6e158a7ed32d7029255434dc7c11d45aa839df` |
| Quote-index addendum | `bba57314bec62a4fa6c7d33e21c97fe009bea452fa61a9ecf7fcbae2b19f1572` |
| Analysis | `d9e51162c70255b1114bf2d94749cf611cef1bd2ff6583ed303e2c2d4713bcb7` |
| Root source closure | `63e63e6ae68a5aec026f954432cdc622ec4c87492276b668962742a9529dd7a8` |
| Root real-host receipt | `caf08d2add8325f4ea879a8dfa599feaa1baee7d80187ed60acecbea13ace385` |
| Final qualification manifest (384 files) | `5332c63315d307e0ea042fe12bd0bc418f085e5e6257c906c3ff67bc5dc30bae` |
| Independent raw12/exact12/eight-flag audit | `5cfb9b1e1ee15146afd940e07dfa282a15c0b6c90d2cdebfbdf51ef4f948c998` |
| Independent raw final closure | `d9fec8cbb3bce2f65fb9e00fa19447141ca093d7afe33cac283edffdd939aeca` |
| Independent committed source closure | `e5b1435613a9a42380dc2e90a82a767e2a9349efbe4fb229160eb7121a7f8eff` |
| Independent real-host audit | `75231028b6dfbf3137a64965018b279873aad006aa80eb12273606ea6e5b37fc` |
| Retained selector-source explanation | `1ee398cdf0f2224ff7d593555354092415b232f8d4f2f75ca0081cb0f147da75` |

The original 369-file qualification manifest and original indices remain immutable. A private guard-import error occurred before the first native invocation and was fixed additively. The expected missing-code quote raised the guard's normal nonzero-exit exception; additive closure read the retained exit-2 evidence without repeating the child. A root controller v1 `KeyError` on an omitted quote field stopped before any SP1 child. An immutable original-quote supplement and addendum bind controller v2 to the already measured original quotes. All twelve actuals then ran exactly once. These bookkeeping corrections are not external retries or new quote measurements.

Guard limits remain 6,144 MiB sampled process-tree PSS, 600-second actual, 180-second native and 1,800-second quote, 192 MiB combined growing output, 16 MiB log and 256 MiB runtime disk plus 320 MiB retention reserve. Request/canonical/benchmark uncompressed caps remain 32/128/64 MiB. Verified identical request/native-input/original-intent bytes use immutable hardlinks rather than triple copies; compression does not bypass caps. Peak sampled PSS is 3,539,834,880 bytes and maximum actual wall time is 16.33281580172479 seconds. Sampling is not a continuous peak guarantee.

The first authoritative real-host receipt matches all 48 unique original runtime-owned resource names to recorded device 27 and checks exact-name lstat absence without deletion. This campaign has no corrected sandbox-host receipt; earlier cohorts retain their own histories.

Independent raw12/exact12/eight-flag, committed-source184 and real-host48 audits passed. Final qualification manifest contains 384 files; the original 369 members are unchanged. The private recovery archive `cold-code-reads-evidence-20261104.tar.gz` is SHA256 `c231c35a15b5e0af631a87017291dbb5a1198f29bbb2c99951443015c27a4a7b`, 4,707,008 compressed bytes, 596 regular members and 209 restorable duplicate aliases. Root streamed and verified all member hashes without extraction. Four runtime artifacts remain external by identity; this private recovery archive is not a published standalone Atlas fixture.

Prior ten cohorts remain separate and unchanged: persistent236, transient8, pairing18, SHA18, linearSHA18, LOG30, MCOPY12, division32, KECCAK27 and modular16. Modular16 analysis remains `e7a922f0e24336cf19bf808e14886f12d5d211aba0ce90a49100173afa6ab2b0`. No prior actual is rerun or counted into a new union.

This documentation pass independently reads retained raw reports and checks all twelve raw-report hashes and normalized actuals, signed literal rows and twelve exact-scaled diagnostics/eight flags against the independent raw audit. The retained-data arithmetic/hash checker and `git diff --check` pass; relative documentation links and path hygiene were checked. It runs no producer tests, native, quote, SP1 or build. The five focused source groups and external gates above describe retained executions. Pre-increment CI run 41728128513 at `0a8d3f4cd5c8e3b0d283b2be4642bc91cbb44ae5` passed formatting, clippy, foundation, pipeline, runtime, model and macOS lanes; prover lacked the SDK6.8.1 runner binary and xtask had a stale RISC0 fingerprint. This is historical CI evidence, not green CI for the new head; no lane was rerun or build performed.

These are finite execute-only SDK6.3/experimental-ELF observations. They establish no STARK proof, aggregation, final PLONK/SNARK wrapping, proof/GPU performance, production parity, reproducible build, ordinary historical usability, global price sufficiency or component bound. Larger/different code footprints, content and lengths, copy/zero behavior, failures, rollback, fork changes, heterogeneous composition/order and production promotion remain open. The retained selectors correct the hypothesis; no tariff adjustment is selected.
