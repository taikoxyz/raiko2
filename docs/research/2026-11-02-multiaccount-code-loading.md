# Multi-account code loading under fixed zkGas pricing

Completed 72 new controlled post-Unzen proposal measurements: 36 CALL and 36 DELEGATECALL inputs, each with 18 full targets and 18 own-N0 controls. Each proposal contains one block. These are synthetic chain 167000 fixtures, not historical Mainnet observations. No runtime tariff, node admission limit, guest, ELF or SDK changed. The host-only constructor gained honest access-derived prestate fixtures.

The comparison retains actual identical-proposal SP1 PGU, each block's node estimate and their literal sum, and the complete proposal estimate separately. Actual PGU is SDK6.3 execute-only `ExecutionReport.gas * 10 // 191`; these are not generated proofs or measured GPU proving time. Raw Unzen difficulty is not used as PGU.

## Findings

| Evidence | Result |
| --- | ---: |
| Primary completed observations | 72/72 |
| Full targets / own-N0 controls | 36 / 36 |
| Full targets with actual > node | 0 / 36 |
| N0 controls with actual > node | 36 / 36 |
| All observations with actual > proposal estimate | 0 / 72 |
| Full targets with actual > proposal estimate | 0 / 36 |
| Strict >2B node gaps, all observations | 0 |
| Maximum actual workload | 1,756,967,656 PGU |
| Maximum node gap including controls | 154,094,882 PGU |

All 36 node shortfalls occur in the N0 controls. They remain observed whole-proposal versus block-observable gaps; this experiment does not allocate actual startup cost to a block or opcode. All absolute deficits and overquotes remain in the data; paired work increments do not replace them. A positive actual-minus-node gap strictly above 2B within the charged whole-block 30M domain is high priority. This threshold is a research prioritization rule, not a proving deadline or a new admission limit.

CALL alone is a weak code-price test because its existing confirmed-spawn wrapper costs 1,000,000 PGU/event plus 64 dispatch. In the 16-account/32-byte/shared full case,108,780 events contribute108,786,961,920 PGU; node quotes110,321,850,843 while actual is1,300,643,989. The fixed wrapper alone is98.6024% of node price. Feature-error charge is0. The runtime skips the ordinary raw-gas CALL route after creating a frame; GAS forwarding is not the cause.

DELEGATECALL uses its existing 6000 PGU wrapper plus 64 dispatch, so the second panel avoids this unrelated reserve without repricing. It still loads the selected code account. ADDRESS returns the parent execution context, and the parent checks that context; independent bytecode target-order, authenticated account and code-hash checks establish code-target identity. ADDRESS is not used as proof of which account supplied the code.

Across the 18 full DELEGATECALL targets, node/actual is 1.566657–1.790509× and complete-proposal quote/actual is 1.669876–1.916299×. These finite ratios are separate from the light N0 gaps and the heavily reserved CALL quotes.

Four of 36 overlapping code-length contrasts have quoted growth below observed whole-proposal growth (three CALL, one DELEGATECALL). The largest own-N0-adjusted gap is3345614 PGU for 16-account unique CALL code 4096→24576 bytes. These are low-priority boundary/precision TODOs under the 2B whole-block rule; every full block remains covered. The independently audited DELEGATECALL contrasts retain one exception:16 accounts/shared code, 32→4096 bytes, actual full-proposal growth 696471 PGU versus quoted growth 260096; after each own-N0, the missing growth is 436798 PGU. The full block remains covered. Seventeen of 18 length contrasts are covered; this is a low-priority boundary/precision TODO, not a per-byte actual coefficient or a high-priority whole-block gap. The512-account unique 32→24576 contrast grows 609528544 actual versus 804257792 quoted PGU.

## Full-target results

| Child operation | Accounts | Code bytes/account | Hashes | Charged block gas | Actual PGU | Node PGU | Proposal PGU | Actual/node |
| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| call | 16 | 32 | shared | 29,999,767 | 1,300,643,989 | 110,321,850,843 | 110,477,360,450 | 0.011790 |
| call | 16 | 32 | unique | 29,999,767 | 1,301,300,562 | 110,321,926,128 | 110,477,435,735 | 0.011795 |
| call | 16 | 4096 | shared | 29,999,767 | 1,301,493,798 | 110,322,110,939 | 110,477,620,546 | 0.011797 |
| call | 16 | 4096 | unique | 29,999,767 | 1,303,488,909 | 110,326,087,664 | 110,481,597,271 | 0.011815 |
| call | 16 | 24576 | shared | 29,999,767 | 1,301,787,711 | 110,323,421,659 | 110,478,931,266 | 0.011800 |
| call | 16 | 24576 | unique | 29,999,767 | 1,327,805,858 | 110,347,059,184 | 110,502,568,791 | 0.012033 |
| call | 128 | 32 | shared | 29,999,127 | 1,264,461,836 | 106,145,130,656 | 106,304,745,175 | 0.011913 |
| call | 128 | 32 | unique | 29,999,127 | 1,270,097,951 | 106,145,768,069 | 106,305,382,588 | 0.011966 |
| call | 128 | 4096 | shared | 29,999,127 | 1,264,218,276 | 106,145,390,752 | 106,305,005,271 | 0.011910 |
| call | 128 | 4096 | unique | 29,999,127 | 1,295,208,283 | 106,179,060,357 | 106,338,674,876 | 0.012198 |
| call | 128 | 24576 | shared | 29,999,127 | 1,265,140,937 | 106,146,701,472 | 106,306,315,991 | 0.011919 |
| call | 128 | 24576 | unique | 29,999,127 | 1,420,925,415 | 106,346,832,517 | 106,506,447,036 | 0.013361 |
| call | 512 | 32 | shared | 29,999,575 | 1,138,354,454 | 91,834,798,392 | 92,009,832,607 | 0.012396 |
| call | 512 | 32 | unique | 29,999,575 | 1,144,587,763 | 91,837,363,101 | 92,012,397,316 | 0.012463 |
| call | 512 | 4096 | shared | 29,999,575 | 1,138,570,587 | 91,835,058,488 | 92,010,092,703 | 0.012398 |
| call | 512 | 4096 | unique | 29,999,575 | 1,247,164,657 | 91,970,532,253 | 92,145,566,468 | 0.013560 |
| call | 512 | 24576 | shared | 29,999,575 | 1,139,658,088 | 91,836,369,208 | 92,011,403,423 | 0.012410 |
| call | 512 | 24576 | unique | 29,999,575 | 1,756,967,656 | 92,641,620,893 | 92,816,655,108 | 0.018965 |
| delegatecall | 16 | 32 | shared | 29,999,387 | 1,237,963,681 | 2,213,465,355 | 2,368,974,962 | 0.559288 |
| delegatecall | 16 | 32 | unique | 29,999,387 | 1,236,263,110 | 2,213,540,640 | 2,369,050,247 | 0.558500 |
| delegatecall | 16 | 4096 | shared | 29,999,387 | 1,238,660,152 | 2,213,725,451 | 2,369,235,058 | 0.559536 |
| delegatecall | 16 | 4096 | unique | 29,999,387 | 1,238,719,212 | 2,217,702,176 | 2,373,211,783 | 0.558560 |
| delegatecall | 16 | 24576 | shared | 29,999,387 | 1,238,939,328 | 2,215,036,171 | 2,370,545,778 | 0.559331 |
| delegatecall | 16 | 24576 | unique | 29,999,387 | 1,253,986,993 | 2,238,673,696 | 2,394,183,303 | 0.560147 |
| delegatecall | 128 | 32 | shared | 29,998,967 | 1,201,120,772 | 2,131,323,345 | 2,290,937,864 | 0.563556 |
| delegatecall | 128 | 32 | unique | 29,998,967 | 1,202,195,521 | 2,131,960,758 | 2,291,575,277 | 0.563892 |
| delegatecall | 128 | 4096 | shared | 29,998,967 | 1,200,880,302 | 2,131,583,441 | 2,291,197,960 | 0.563375 |
| delegatecall | 128 | 4096 | unique | 29,998,967 | 1,227,278,492 | 2,165,253,046 | 2,324,867,565 | 0.566806 |
| delegatecall | 128 | 24576 | shared | 29,998,967 | 1,201,782,697 | 2,132,894,161 | 2,292,508,680 | 0.563452 |
| delegatecall | 128 | 24576 | unique | 29,998,967 | 1,355,028,224 | 2,333,025,206 | 2,492,639,725 | 0.580803 |
| delegatecall | 512 | 32 | shared | 29,999,647 | 1,080,100,093 | 1,849,851,598 | 2,024,885,813 | 0.583885 |
| delegatecall | 512 | 32 | unique | 29,999,647 | 1,086,231,113 | 1,852,416,307 | 2,027,450,522 | 0.586386 |
| delegatecall | 512 | 4096 | shared | 29,999,647 | 1,080,308,963 | 1,850,111,694 | 2,025,145,909 | 0.583916 |
| delegatecall | 512 | 4096 | unique | 29,999,647 | 1,187,507,808 | 1,985,585,459 | 2,160,619,674 | 0.598064 |
| delegatecall | 512 | 24576 | shared | 29,999,647 | 1,081,415,935 | 1,851,422,414 | 2,026,456,629 | 0.584100 |
| delegatecall | 512 | 24576 | unique | 29,999,647 | 1,695,759,657 | 2,656,674,099 | 2,831,708,314 | 0.638302 |

## Inputs, cache and semantics

The axes are 16/128/512 predeployed accounts × 32/4096/24576 runtime bytes ×shared/unique hashes ×zero/full, separately for each child operation. Four signed users have the same full-selected envelope within each own-N0 pair. The parent cycles all targets, checks status/return length/context/counter, and the child executes only ADDRESS/MSTORE/RETURN32. Zero padding and a fixed-width unreachable salt change code loading and analysis without changing executed child maps.

Each additional account starts with nonce 1, zero balance and empty storage. Deployment/setup is outside this measured block and is not free across its lifecycle. Distinct addresses retain separate account state and EVM warmth; same-hash code shares bytecode and its precomputed jump table while each call still executes. Warmth resets for each transaction, while code cache entries can survive within a block.

The host builds a complete authenticated prestate, records actual backing-database account/storage/code reads including reverted reads, exports mutation-aware target multiproofs and hash-deduplicated fetched code, and replays the minimized witness. It requires equal block, receipts, poststate, opcode maps and read identities. Unused N0 target code is absent. Proof siblings can commit to background accounts; proof-node counts need not decrease because absence/update paths also require nodes.

Charged full gas is at least 99% of 30M including Anchor 167355; the native header remains 31M. Every remaining-header prefix admits its user envelope. The experimental evm-gas-only profile bypasses legacy Unzen limits. No future node cap or 30M-header admission claim is made.

CALL per-user gas is `21054 + 272N + 2500 min(N,A) + 9[N>0]`; DELEGATECALL uses 265 instead of272. Integer full bounds and four-user warmth reset were independently checked from actual bytecode and receipts. Children have no storage operations, refunds or value transfers.

## Verification and retained evidence

- Host package: `cargo test --locked -p zkgas --features fixture` passed 157 tests; package Clippy with tests and `-D warnings`, format and diff checks passed. Only the host fixture was rebuilt. Five unchanged request profiles matched the older frozen host's JSON/bincode/report/stdout byte-for-byte.
- Final integrated stdlib: `MULTIACCOUNT_CALL_BASELINE=<old-call-cases> ~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests` passed 214 tests in 61.833s, 175 executed and 39 optional skips. Native/quote representative tests and focused actual missing-witness replay checks passed separately.
- Independent source review covered the complete host and generator diffs. Independent behavior qualification checked 72 canonical inputs, 288 signed users, code bytes/hashes, authenticated full prestate roots, minimal witness coverage, exact maps/gas and 36 own-N0 pairs.
- Missing code/proof, wrong child context, failed/short child output, and native-pass order/code-salt mutations remain as explicit negatives without fabricated PGU. An initial negative command failed CLI argument validation; it is labeled invalid invocation and excluded from replay coverage. Corrected commands assert failed proposal replay rows, IDs, raw-input hashes and actual database failure reasons.
- Independent actual audits checked raw SDK normalization, canonical inputs/public outputs, literal and aggregate ceilings, full quotes, exact scaled matched increments, failures/resources and final host-namespace SHM absence.

The sampled SP1 process-tree PSS peaks and owned-resource closure are recorded per panel below. These bounded execute-only resource observations are not GPU throughput measurements.

| Panel | Peak sampled PSS bytes | Owned SHM names verified absent |
| --- | ---: | ---: |
| call | 4,669,043,712 | 144 |
| delegatecall | 4,670,002,176 | 144 |

## Frozen panels

| Panel | Producer source | Configuration SHA256 | Policy SHA256 |
| --- | --- | --- |
| call | `a0b26969b5c5bd26c20ae7effd15139e2096512d` | `a8bd510e98976beb7aa37bc2232e9be3a452ae51b8da9ad08901589914054c54` | `2864e5d62e11cf11bbb4e101dadd40f6d70d3db995236f94cd6a8878c1b2d307` |
| delegatecall | `f2219c25b9193759fa0f46b1d130a0133bd3811a` | `460a6767648d8ffb67c1622041594231d8ffdbf5af02540183c28104b4c35853` | `b087fa51019df3b14896124d72c41eadfa91c4a4b52753e76f34831fe040a457` |

Both panels use native `1d555010…`, quote `c10eae23…`, model `c8936124…`, SDK 6.3.0, sampler `1998fb85…` and ELF `b76baf2e…`. Source manifests are provenance, not build attestations or certification of deployed-program parity. One earlier small ABI-compatibility actual is retained separately and excluded from the 72 primary observations.

Tools and usage remain in [bin/zkgas/bench](../../bin/zkgas/bench/README.md) and pending [Raiko #296](https://github.com/taikoxyz/raiko2/pull/296). Immutable data/provenance are retained in [Atlas fixture `unzen-multiaccount-code-loading-v1`](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-multiaccount-code-loading-v1); its sample indices locate canonical requests/inputs, raw native/quote/SP1 reports, qualifications and all attempts. No runner, executable, ELF or pricing implementation belongs in Atlas.

## Remaining scope

This is finite coverage of the specified code-loading domains, not a universal workload bound. The512-account fixture cap is a tool bound, not a protocol bound or the maximum unique-code footprint possible within 30M gas. Larger cold-account/code footprints, unreachable JUMPDEST/PUSH-data tail patterns, the small code-length boundary observation, cross-account storage mutation/deletion and ancestor rollback remain future tests. Only zero padding plus fixed salts was sampled here. The code-only children do not test delegated storage behavior. Synthetic conservative CALL overquotes remain visible as a precision TODO; no historical Mainnet conclusion or tariff was rewritten.
