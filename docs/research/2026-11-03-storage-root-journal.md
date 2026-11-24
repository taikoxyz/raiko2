# Many-key restore and root rollback across heterogeneous storage roots

All 36 new execute-only observations completed. Literal node gaps occur in eight inputs: the four `restore-stop` own-N0 controls and all four `restore-stop` small32 inputs. None of the 12 full-body inputs has a node gap; no input has a complete-proposal gap or exceeds the frozen >2B actual-minus-node priority threshold at <=30M charged block gas. The largest node gap is 158,991,334 PGU in restore-stop/A64/S16384/N0. The largest actual is 499,492,021 PGU in restore-stop/A64/S16384/N101. These are finite whole-proposal observations, not a storage or composition bound.

Root REVERT retains attempted collection work, discards effective refunds and restores contract storage. Its 64 terminals also receive a substantial existing model allowance: 5M PGU per REVERT event. Both that allowance and account-range reuse matter when interpreting coverage. The [coverage inventory](zkgas-workload-coverage.md) separates sampled families, still-open interactions and exact-production/historical promotion.

## Finite programs and gas domain

The panel crosses `restore-stop`, `update-revert` and `restore-revert`, A=8/64 accounts, S=8192/16384 aggregate initialized storage entries, and own-N0/small32/full levels: 36 new inputs in 12 own-N0/small/full triples. Every proposal has one 31M-header block and 64 ordinary signed top-level CALL users, round-robin recipients and empty calldata. The unused primary contract is STOP-only. User bytecode contains no CALL-family wrapper; this is top-level root rollback, not caught child rollback.

For account index i, let `B_i=(i+1)*65536`, `K=S/A` and `V=64/A`. Raw phase slot 0 contains `B_i+1`; raw slots `B_i+1..B_i+K-1` contain 1; all others are absent. Each triple retains exactly the same initial maps and signed gas envelope. Every row has A distinct initial and A distinct final storage roots; distinct roots alone do not imply distinct internal nodes. The guarded body reads and checks 1, stores 2, reads 2 into a checksum, and in restore modes stores 1 and reads 1 into that checksum. It checks loop count N, checksum 3N for restore or 2N for update, and the written phase+N before the terminal. Wrong-path guards reach INVALID.

- `restore-stop` commits the phase. User j begins at `B_(j%A)+1+floor(j/A)*N`; each account ends at `B_i+1+V*N`. Main values finish 1. Distinct user (account,rawslot) pairs total `64*N+A`.
- Both REVERT modes attempt and read-check phase+N, then execute zero-length/zero-offset root REVERT. Every visit to an account starts at `B_i+1`; distinct user pairs total `A*(N+1)`. Each account's original complete storage map and storage root are restored. All 64 user receipts have status false, while Anchor succeeds. They are valid included transactions; sender nonces, fees and Anchor still change whole-world state.

Dirty `1->2->1` earns 2800 refund-counter units per body key before the terminal. STOP keeps that counter and applies `min(counter,gross//5)`. Root REVERT discards the refund: final effective counter and applied refund are zero. Before-terminal and effective refunds remain separate observations.

| Mode | Full N/user | Gross gas/user | Refund before terminal/user | Effective counter/user | Applied refund/user | Charged gas/user | Signed envelope/user | Charged block gas |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| restore-stop | 101 | 578135 | 282800 | 282800 | 115627 | 462508 | 580436 | 29767867 |
| update-revert | 84 | 465933 | 0 | 0 | 0 | 465933 | 468234 | 29987067 |
| restore-revert | 80 | 463397 | 224000 | 0 | 0 | 463397 | 465698 | 29824763 |

Full values hold across the A/S choices. Charged block gas includes Anchor 167355. Full N is maximal only for this fixed 64-user program: the solver checks charged budget, present ranges, <=8192 distinct target pairs, 15M/user and protocol caps, every ordered admission prefix, and the SSTORE gas-left sentry. Each triple uses full gross+2301 as its shared signed envelope. The bounded bytecode VM independently executes N+1 to check maximality. All full points exceed 99% of the 30M charged research budget; they are not observations under a 30M header. Small N=32 contrasts across modes can change gross gas, refunds, charged gas, envelopes and unique-key support; they do not isolate terminal or opcode costs at fixed gas.

## Literal same-proposal results

Actual PGU is the whole proposal's SDK6.3 `ExecutionReport.gas * 10 // 191`. Literal node PGU is the sum of per-block known-PGU ceilings; each input has one block, so the sum equals that block's ceiling. Complete-proposal PGU additionally includes proposal/input terms unavailable to the node's block-local quote. Raw Unzen scalars are not actual PGU. Positive gaps are `max(actual-estimate,0)` for each surface separately. Own-N0 retains the users, phase program and full envelope; it is not an empty proposal.

### Full-body points

| Mode | S | A | N/user | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive full gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| restore-stop | 8192 | 8 | 101 | 29767867 | 457321106 | 583353345 | 913009360 | 0 | 0 |
| restore-stop | 8192 | 64 | 101 | 29767867 | 458961255 | 585683505 | 916062104 | 0 | 0 |
| restore-stop | 16384 | 8 | 101 | 29767867 | 493736805 | 583353345 | 976927672 | 0 | 0 |
| restore-stop | 16384 | 64 | 101 | 29767867 | 499492021 | 585683505 | 984232488 | 0 | 0 |
| update-revert | 8192 | 8 | 84 | 29987067 | 271881337 | 632080145 | 834299816 | 0 | 0 |
| update-revert | 8192 | 64 | 84 | 29987067 | 401943054 | 772933697 | 1084011872 | 0 | 0 |
| update-revert | 16384 | 8 | 84 | 29987067 | 278768487 | 632080145 | 847240520 | 0 | 0 |
| update-revert | 16384 | 64 | 84 | 29987067 | 440217751 | 772933697 | 1144997104 | 0 | 0 |
| restore-revert | 8192 | 8 | 80 | 29824763 | 291450847 | 655086704 | 856017815 | 0 | 0 |
| restore-revert | 8192 | 64 | 80 | 29824763 | 418387906 | 789344160 | 1096649871 | 0 | 0 |
| restore-revert | 16384 | 8 | 80 | 29824763 | 297971103 | 655086704 | 868460471 | 0 | 0 |
| restore-revert | 16384 | 64 | 80 | 29824763 | 455127538 | 789344160 | 1155992927 | 0 | 0 |

### Small32 points

| Mode | S | A | N/user | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive full gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| restore-stop | 8192 | 8 | 32 | 10464699 | 286022462 | 201049036 | 449686587 | 84973426 | 0 |
| restore-stop | 8192 | 64 | 32 | 10464699 | 291808414 | 203379196 | 459939195 | 88429218 | 0 |
| restore-stop | 16384 | 8 | 32 | 10464699 | 302798722 | 201049036 | 481018899 | 101749686 | 0 |
| restore-stop | 16384 | 64 | 32 | 10464699 | 310645812 | 203379196 | 495007347 | 107266616 | 0 |
| update-revert | 8192 | 8 | 32 | 12568315 | 215256403 | 453596644 | 635543058 | 0 | 0 |
| update-revert | 8192 | 64 | 32 | 12568315 | 283523773 | 508697620 | 760855234 | 0 | 0 |
| update-revert | 16384 | 8 | 32 | 12568315 | 218026776 | 453596644 | 640979138 | 0 | 0 |
| update-revert | 16384 | 64 | 32 | 12568315 | 301642730 | 508697620 | 795573034 | 0 | 0 |
| restore-revert | 8192 | 8 | 32 | 13039355 | 224737817 | 468292104 | 650238519 | 0 | 0 |
| restore-revert | 8192 | 64 | 32 | 13039355 | 292467379 | 523393336 | 775550951 | 0 | 0 |
| restore-revert | 16384 | 8 | 32 | 13039355 | 227494991 | 468292104 | 655674599 | 0 | 0 |
| restore-revert | 16384 | 64 | 32 | 13039355 | 311060645 | 523393336 | 810268751 | 0 | 0 |

### Preserved own-N0 controls

| Mode | S | A | N/user | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive full gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| restore-stop | 8192 | 8 | 0 | 1669307 | 173781732 | 21587252 | 179480930 | 152194480 | 0 |
| restore-stop | 8192 | 64 | 0 | 1669307 | 181964832 | 23917412 | 193559442 | 158047420 | 0 |
| restore-stop | 16384 | 8 | 0 | 1669307 | 173849653 | 21587252 | 179639930 | 152262401 | 0 |
| restore-stop | 16384 | 64 | 0 | 1669307 | 182908746 | 23917412 | 195488794 | 158991334 | 0 |
| update-revert | 8192 | 8 | 0 | 1669691 | 173797888 | 341599408 | 499493086 | 0 | 0 |
| update-revert | 8192 | 64 | 0 | 1669691 | 181779983 | 343929568 | 513571598 | 0 | 0 |
| update-revert | 16384 | 8 | 0 | 1669691 | 173854292 | 341599408 | 499652086 | 0 | 0 |
| update-revert | 16384 | 64 | 0 | 1669691 | 182732207 | 343929568 | 515500950 | 0 | 0 |
| restore-revert | 8192 | 8 | 0 | 1669691 | 173790097 | 341601136 | 499494814 | 0 | 0 |
| restore-revert | 8192 | 64 | 0 | 1669691 | 181788014 | 343931552 | 513573582 | 0 | 0 |
| restore-revert | 16384 | 8 | 0 | 1669691 | 173850427 | 341601136 | 499653814 | 0 | 0 |
| restore-revert | 16384 | 64 | 0 | 1669691 | 182722293 | 343931552 | 515502934 | 0 | 0 |

Every table row identifies `storage-root-journal-{mode}-a{A}-s{S}-n{N}`. All 36 actuals are new; no older actual is reused in this dataset. Across the 12 full points, node/actual is 1.1726–2.3248 and complete-proposal/actual is 1.9705–3.0686. All four small32 STOP node gaps and all four STOP control gaps remain literal observations; the covered full points do not supersede them.

## Matched work, state, accounts and rollback

All 24 small/full-minus-own-N0 actual increments are positive, ranging from 41,458,515 to 319,887,152 PGU, and covered by their matched node and complete-proposal increments. All 12 small-to-full actual differences, 56,624,934–190,938,083 PGU, are also covered by both quote differences. This is a diagnostic about the measured whole programs, not a linear extrapolation or an isolated storage tariff estimate.

All 18 S16384−S8192 contrasts increase actual, by 56,404–40,530,766 PGU, while literal node quotes stay identical. Complete-proposal differences cover all 18 actual differences. At full body, the storage-size increases are 36.416M/40.531M PGU for restore-stop A8/A64, 6.887M/38.275M for update-revert and 6.520M/36.740M for restore-revert. The smaller A8 root-REVERT support repeats across eight users per account; STOP advances through fresh ranges.

All 18 A8→A64 contrasts also increase actual, by 1,640,149–161,449,264 PGU, at the same mode/N/charged gas. Full STOP actual increases only 1,640,149 PGU at S8192 and 5,755,216 at S16384. Full update-revert increases 130,061,717/161,449,264; full restore-revert increases 126,937,059/157,156,435. For REVERT, changing A also changes distinct support from `8*(N+1)` to `64*(N+1)` and removes repeated visits. Node quote differences cover only three of the 18 account-growth differences, while complete-proposal differences cover all 18. These incremental ranking limits remain visible despite all full absolute node quotes covering actual.

The following small32 comparisons hold body N constant; they do not hold charged gas or envelope constant across modes. Charged block gas is 10,464,699 for STOP restore, 12,568,315 for REVERT update and 13,039,355 for REVERT restore.

| S | A | STOP restore actual PGU | REVERT update actual PGU | REVERT restore actual PGU | Restore REVERT minus STOP PGU | Restore minus update REVERT PGU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 8192 | 8 | 286022462 | 215256403 | 224737817 | -61284645 | 9481414 |
| 8192 | 64 | 291808414 | 283523773 | 292467379 | 658965 | 8943606 |
| 16384 | 8 | 302798722 | 218026776 | 227494991 | -75303731 | 9468215 |
| 16384 | 64 | 310645812 | 301642730 | 311060645 | 414833 | 9417915 |

At A8, restore REVERT is 61.285M–75.304M PGU below STOP restore; at A64 it is 0.415M–0.659M above. This contrast changes committed phases, refunds and repeated-key support as well as the terminal. Within root REVERT, explicit restoration raises small32 actual by 8.944M–9.481M PGU and node quote by about 14.696M. Neither contrast identifies an actual cost per REVERT or SSTORE.

The independent selected-route audit confirms that the frozen model's accepted-absolute `opcode:0xfd` route charges 5M PGU per terminal event. All root-REVERT inputs have 64 such events, adding 320M PGU to the literal node quote. Their own-N0 node quotes are approximately 341.6M/343.9M at A8/A64, versus approximately 21.6M/23.9M for STOP. Thus root-REVERT control coverage is strongly affected by a fixed event-count allowance; it cannot be attributed solely to accurate storage accounting.

As a read-only sensitivity diagnostic, subtracting exactly 320M PGU from each of the eight full REVERT literal node quotes still leaves all eight above actual; the smallest remaining margin is 12,715,946 PGU at update-revert/A64/S16384/N84. This arithmetic does not create a new quote or model, recommend tariff removal, or change the primary metrics. Packing 64 failures supplies 64 terminal allowances; a few longer failed transactions remain a distinct untested composition.

The retained analysis keeps 96 diagnostics: 24 small/full-minus-own-N0 increments, 12 small-to-full contrasts, 18 background-state contrasts, 18 account-fanout contrasts and 24 mode contrasts (three mode pairs at each A/S and N0/small). The four contrast sets retain smaller/larger sample IDs, actual differences, scaled node/full differences and `same_body_n`/`same_charged_gas` flags. Own-N0 increments retain sample/baseline IDs, actual/scaled quote increments and coverage flags. Scaled quote differences precede rounding. No linear N0 projection or actual allocation to a transaction, opcode, hash or trie is inferred. An increment being covered does not remove an absolute node deficit.

## Attempted work retention and observability

The independent native/private-VM instruction traces preserve attempted and completed user events through root REVERT. Exposed node/prove aggregate `normal_dispatch_count` matches the independent 64-user instruction totals plus a fixed observed Anchor allowance of 12363. Unique storage access/write counts match `64*N+A+4` for STOP or `A*(N+1)+4` for root REVERT; the fixed four observed Anchor slots are separate. Two deliberately counter-dropping mutants are rejected.

For example, update-revert/A8/N84 attempts 5440 SSTORE events across all users, but exposes 684 unique storage pairs including Anchor. These are different intended quantities, not missing occurrences. The quote API exposes no per-opcode/per-occurrence pricing counters. This audit observes no collection drop in the exposed aggregate/unique surfaces; it cannot prove that each opcode occurrence received an independent charge. Counter loss on rollback would be an accounting-correctness finding distinct from a tariff being low.

## Disposition and remaining coverage

Keep all eight node misses as low-priority refinements under the frozen >2B/30M rule. There is no new high-priority gap in these 36 observations. Covered full root-REVERT points supply finite evidence under the current tariff and packing; they do not close fewer-longer-failure programs, arbitrary nested rollback or high warm-toggle counts.

Keep prices unchanged. Storage reads/writes and reverted transactions also occur in ordinary workloads; synthetic construction does not make conservative overquote harmless. Large quotes can reduce usable admission capacity. No replacement node limit is selected and natural historical usability is not certified here. See the linked [coverage inventory](zkgas-workload-coverage.md) for sampled families and open interactions. Many-key single-toggle restore/root rollback does not close many-key × high warm-toggle-count × nested/caught rollback, deliberate hash-path skew/depth, arbitrary mutation topology, broader trie/cache/key-lookup distributions or heterogeneous workload order. Uniform sequential raw keys do not test deliberately skewed internal lookup/hash-table distributions; further source review and bounded tests must establish which distributions matter. No exploitable hash-collision behavior is established. No fit on this opened panel, global bound, generated proof, GPU throughput or production-program claim follows.

## Tools, qualification and retained evidence

See [the tool README](../../bin/zkgas/bench/README.md#heterogeneous-storage-restore-and-root-rollback), [producer](../../bin/zkgas/bench/cases/storage_root_journal.py) and [short specification](../superpowers/specs/2026-11-03-storage-root-journal.md). The tools change adds the separate producer and tests; the shared bounded test VM adds only zero-range root REVERT, preserves attempted traces/preterminal refunds, restores the caller's state dictionary in place and rejects unsupported nonzero return ranges. Existing STOP behavior is preserved. Pricing/model, host, GuestInput, guest, SDK, ELF, dependencies and production pins remain unchanged.

Final journal validation ran 11 tests successfully with no skips, including two canonical and two native-accepted wrong-semantics specimens. The earlier nine-module lane ran 106 tests: 99 passed and seven existing optional skips. The final focused lane includes one test added after the broader run; these counts describe separate runs. Native success alone is not the semantic oracle, and valid failed REVERT receipts are expected.

Independent source/specification review and all-36 native, original real-quote, 2304 signed-user, authenticated original/final storage-map/root and conservative witness gates passed before actuals. Original quote command/start/outcome provenance is retained. Each actual uses only its exact canonical-input/key/model/profile/executable/public-output-qualified original quote, with a retained-source receipt; this is retained quote reuse, not a second invocation.

Source/artifact/input/original-quote identities and policy were frozen before 36 sequential execute-only attempts, each under 6GiB sampled process-tree PSS and 600s guards, a 256MiB minimum-free check plus 320MiB retention reserve, and bounded logs/output. Every attempt completed once; no failed or censored actual and no implicit retry occurred. Maximum guard wall time was 13.8726s and peak sampled process-tree PSS was 2966707200 bytes.

Independent final raw verification passed all 36 actuals, all 96 diagnostic contrasts, all 2304 signed users, counter retention, the 116-distinct-input storage-cohort join and the REVERT allowance diagnostic. It rejoined raw SDK gas conversion, canonical input/public-output identities, original quote provenance, literal node sums/full totals, expected failed/success receipts, gas/refunds/phases, complete initial/final storage roots and resource guards. A separate read-only host-namespace audit confirmed all 144 exact owned shared-memory names absent; the raw actual audit did not independently certify live host absence. These observations execute the guest; they do not generate proofs or measure production/GPU proving throughput.

The retained ELF declares source revision `4580faf7dc009714b310fd625b5810667019d683`. That declaration and source inspection are not a build attestation or exact production-program/SDK parity evidence. No guest or Rust build occurred. These Markdown changes need no Rust checks; old-head CI is not new-head validation.

| Identity | Value |
| --- | --- |
| Producer source | `e1726e11d73bc11da43255e8690a267d6b3c73f0` |
| Model | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Retained ELF / SDK | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` / `6.3.0` |
| Frozen config | `5e14abd075a8c7fc0168541c75f2025fbea4c79807646956860e7295c7c1caca` |
| Pre-actual policy | `1f151337ea363243fbb75eb0922c42fedf557eb7729ed777f3d71746b157dac3` |
| Input/quote qualification | `541a274fb054634d4d8d6009637cdfd123e2ba5ea80fd93825ca00ab5f4a3fd8` |
| Counter retention audit | `0aa3d2a95f3cfb76ace70a188b039b0933eba8ed896f88e63d17c316108e9c97` |
| Actual analysis | `5cc320d2e8323ac38dab1df9469d7ee305461c1e9cb062dd7d0c8511b5c60e52` |
| Gas-budget report | `3c5e7f81973ce18540ff9150aeec60a5679cf8d62811656ba218f7ebce59208d` |
| Independent final verification | `20777747465f707ca16c67658ab2ba9c7851a02aee6010cedea7db8ba7020ea4` |
| Independent analysis comparison | `89fd5c83265cf97d08f37f57eb361248a879c0dd69faf7c3b6f0bb4fff59a64c` |
| REVERT selected-route diagnostic | `56219ec4fde6a7f6c9a4a174a1dc5fd161b62d9fe6d81c851d41a9a6addf6b44` |
| Host shared-memory audit | `ae02974c712a76b52c18639f0dd544238cb4248af9d5b366a4aad3b129b0d5a3` |
| Four-panel storage-cohort summary | `8a33d90c27df8f8fd9953c38b74ef88087aa01359b0ecb5d1f6863f0e498fb02` |
| Independent 499-file evidence manifest | `9e06d6899f36b626898945b7d506bc0dfd87afd02068db2c0dc6d42f233a5875` |

Raw requests/canonical inputs, original quotes and invocation provenance, native diagnostics, all 36 actual attempts, controls, source/helper/artifact identities and resource/audit receipts are retained for local recovery and separate immutable data-only Atlas publication. The compressed local recovery archive is SHA256 `25b93a7a36ba2bd4028fc34398410d63f8e6a930e8a3fabf0f895ee42128ce05`, 51813427 bytes, with 1031 regular members carrying 138728698 payload bytes, 216 restorable duplicate aliases and four external runtime artifacts identified by hash. Streamed member hashes were verified without extraction. The archive records helper dependencies and external artifacts; it is recovery evidence, not a standalone auditor runner or a published Atlas fixture. No merge, deployment, repricing or on-chain action is part of this campaign.
