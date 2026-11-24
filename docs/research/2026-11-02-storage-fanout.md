# Initialized-storage fanout across accounts

This campaign follows [single-account storage mutation](2026-11-02-storage-mutation.md) with 64 ordinary signed top-level CALL users spread across 1, 8 or 64 initialized accounts. It preserves frozen c893 pricing, literal node-known PGU and complete-proposal estimates beside actual same-proposal SP1 PGU.

All 24 execute-only observations completed. There are 15 positive literal node gaps, including three of 12 full-body points; the largest is 153,919,193 PGU in an own-N0 control. The largest full-body gap is 58,675,269 PGU at A1/S16384/update/N84. The largest control gap is A64/S16384/update/N0 at 1669307 charged block gas. Complete-proposal estimates cover all 24. No actual-minus-node gap exceeds the >2B PGU high-priority threshold within the 30M charged-gas domain. These finite observations establish neither a global storage bound nor ordinary-transaction admission compatibility.

The panel uses **homogeneous storage roots**: equal-size all-one maps produce exactly one initial storage root and one final storage root across configured accounts in every row. Canonical witnesses globally deduplicate identical node hashes. Distinct accounts still have separate per-address sparse storage trie instances, supported by current and source-declared ELF code inspection; that inspection is not a build attestation. This is valid finite account-fanout evidence, but it does not close heterogeneous-root coverage.

## Concrete program and gas domain

The 24 inputs cross A=1/8/64 accounts, aggregate S=8192/16384 entries, update/delete and own-N0/full levels. Setup is outside the measured block. Each account starts with raw slots `0..S/A-1` equal to 1, including phase slot 0; all other slots are absent. Every configured account uses the same short guarded runtime, while the unused primary runtime remains STOP.

There are always 64 signed users with empty calldata. User `j` calls account `j%A`, starts at `1+floor(j/A)*N`, processes N consecutive body slots and commits the next phase to that account. Update writes 2; delete writes 0. Before-value, count, checksum and phase guards constrain the body. The users execute no CALL-family opcode, avoiding a user CALL-wrapper reserve; the fixture Anchor is separate. Final initialized-entry counts are S for update and `S-64*N` for delete. Every account ends at phase `1+(64/A)*N`.

| Mode/level | Body N/user | Main writes/block | Gross gas/user | Applied refund/user | Charged block gas | Signed envelope/user |
| --- | --- | --- | --- | --- | --- | --- |
| update/full | 84 | 5376 | 465927 | 0 | 29986683 | 466927 |
| delete/full | 106 | 6784 | 581075 | 116215 | 29918395 | 582075 |
| update/own-N0 | 0 | 0 | 23468 | 0 | 1669307 | 466927 |
| delete/own-N0 | 0 | 0 | 23468 | 0 | 1669307 | 582075 |

Every row also executes 64 phase writes. Delete's full refund counter is 508800/user, capped to one fifth of gross gas. Charged block gas includes the 167355-gas Anchor. Zero/full pairs share the full-body envelope. The header remains 31M, every ordered prefix can reserve the next envelope, and users stay below the 15M tool and 16777216 protocol caps.

Full N is maximal for this generated guarded program under the charged budget, header/envelope constraints, `64*N+A<=8192` distinct account-slot pairs and `(64/A)*N<=S/A-1` present body slots. It is not a maximum over arbitrary compositions. An independent byte interpreter and native qualification check concrete reads/writes, per-transaction originals/warmth/refunds, all 1536 signed recipients, 65 receipts per input and authenticated complete initial/final roots. Seven mutants are rejected, including two that succeed natively with unintended semantics.

## Literal same-proposal results

Actual PGU is SDK 6.3 `ExecutionReport.gas * 10 // 191` for the whole proposal. Node PGU is the literal sum of per-block known-PGU ceilings; each input has one block, so that sum equals its block ceiling. The complete estimate is the full proposal total, including proposal-once terms. Raw Unzen scores are not compared with actual PGU. Positive gaps are `max(actual-estimate, 0)`; zero does not imply precision. No proposal actual is assigned to a transaction, block component, opcode or trie.

### Full-body points

| Mode | Aggregate S | Accounts A | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive full gap |
| --- | --- | --- | --- | --- | --- | --- | --- |
| delete | 8192 | 1 | 499268557 | 562090825 | 2123091336 | 0 | 0 |
| delete | 8192 | 8 | 386337015 | 562382095 | 1948515678 | 0 | 0 |
| delete | 8192 | 64 | 377854216 | 564712255 | 2042068422 | 0 | 0 |
| delete | 16384 | 1 | 567060545 | 562090825 | 1994414440 | 4969720 | 0 |
| delete | 16384 | 8 | 422485983 | 562382095 | 1813612062 | 0 | 0 |
| delete | 16384 | 64 | 410661250 | 564712255 | 2230633846 | 0 | 0 |
| update | 8192 | 1 | 465113246 | 450298895 | 868510582 | 14814351 | 0 |
| update | 8192 | 8 | 373491458 | 450590165 | 812327300 | 0 | 0 |
| update | 8192 | 64 | 367788797 | 452920325 | 816199804 | 0 | 0 |
| update | 16384 | 1 | 508974164 | 450298895 | 945533606 | 58675269 | 0 |
| update | 16384 | 8 | 400326493 | 450590165 | 876075988 | 0 | 0 |
| update | 16384 | 64 | 391590313 | 452920325 | 876211628 | 0 | 0 |

The largest actual is 567,060,545 PGU at A1/S16384/delete/N106. Across full points, node/actual spans 0.8847–1.4945 and complete-proposal/actual spans 1.8577–5.4318. The three full node misses are A1 update at both S values and A1/S16384 delete. All other full node estimates cover actual.

### Preserved own-N0 controls

| Mode | Aggregate S | Accounts A | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive full gap |
| --- | --- | --- | --- | --- | --- | --- | --- |
| delete | 8192 | 1 | 172530137 | 21294510 | 176646620 | 151235627 | 0 |
| delete | 8192 | 8 | 173137074 | 21585780 | 178948690 | 151551294 | 0 |
| delete | 8192 | 64 | 177382665 | 23915940 | 188829034 | 153466725 | 0 |
| delete | 16384 | 1 | 172532451 | 21294510 | 176667620 | 151237941 | 0 |
| delete | 16384 | 8 | 173189829 | 21585780 | 179114690 | 151604049 | 0 |
| delete | 16384 | 64 | 177833875 | 23915940 | 190441034 | 153917935 | 0 |
| update | 8192 | 1 | 172535139 | 21294766 | 176646876 | 151240373 | 0 |
| update | 8192 | 8 | 173140212 | 21586036 | 178948946 | 151554176 | 0 |
| update | 8192 | 64 | 177383269 | 23916196 | 188829290 | 153467073 | 0 |
| update | 16384 | 1 | 172537467 | 21294766 | 176667876 | 151242701 | 0 |
| update | 16384 | 8 | 173189850 | 21586036 | 179114946 | 151603814 | 0 |
| update | 16384 | 64 | 177835389 | 23916196 | 190441290 | 153919193 | 0 |

All 12 controls have positive node gaps. They retain the signed transactions, account schedule, envelope and phase program; N0 does not mean an empty proposal. Their literal denominators remain primary. All 12 matched full-minus-own-N0 node increments and complete-proposal increments cover actual increments, but those diagnostics do not erase any literal gap or allocate work to individual operations.

## Account and background-state contrasts

All 12 full account comparisons (A1→8, A8→64 and A1→64 at each S/mode) reduce actual. For S8192/update, actual falls 465113246→373491458→367788797 PGU. Node rises slightly, 450298895→450590165→452920325, while full proposal moves 868510582→812327300→816199804. The A8→64 full estimate increases while actual decreases, so even covered estimates do not establish correct ranking.

The homogeneous roots matter to interpretation. For S8192/update/full:

| A | Serialized witness nodes | Serialized node bytes | Unique storage nodes | Logical storage-node sum per address | Quoted hydration hash blocks | Quoted materialization work units |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 8367 | 587917 | 8360 | 8360 | 9589 | 19293 |
| 8 | 1026 | 73685 | 1010 | 8080 | 1212 | 18823 |
| 64 | 229 | 20038 | 132 | 8448 | 260 | 19649 |

Serialized witness counts/bytes and authenticated logical sums are diagnostics, not observed runtime allocations or an attribution of actual PGU. Canonical hash dedup explains why distinct account addresses do not imply proportionately distinct serialized storage nodes. The observed reduction does not show that arbitrary account fanout is cheaper.

All six full S16384−S8192 comparisons increase actual at fixed A/body/EVM gas, while node is unchanged:

| Mode | A | Actual difference PGU | Complete-proposal difference PGU |
| --- | --- | --- | --- |
| delete | 1 | 67791988 | -128676896 |
| delete | 8 | 36148968 | -134903616 |
| delete | 64 | 32807034 | 188565424 |
| update | 1 | 43860918 | 77023024 |
| update | 8 | 26835035 | 63748688 |
| update | 64 | 23801516 | 60011824 |

For delete at A1/A8, complete estimates move opposite to actual as conservative quoted structural churn falls (2977→2375 and 2714→2154 units). At A64 churn instead rises 2945→3201. Under the existing 400000-PGU/unit price, A64/S8192/delete quotes 1178000000 PGU for churn alone, versus actual whole-proposal 377854216 PGU and complete estimate 2042068422 PGU. This identifies a dominant quoted allowance; it does not measure actual cost of churn. The retained analysis includes all 12 own-N0 increments, 12 background-state contrasts and 24 account-fanout contrasts, including controls.

## Ordinary-transaction usability and disposition

Keep prices unchanged. The batches resemble ordinary repeated storage updates/deletes, so synthetic construction alone does not make overquote harmless. Full node overquote reaches about 1.49x; full-proposal overquote reaches 5.43x and can reduce usable proposal capacity if used for admission. Do not substitute the complete-proposal quote into a per-transaction node filter. No new node limit is selected here. Historical normal-transaction compatibility remains a separate required regression before tariff or admission changes.

Record the three full node misses, all control gaps, full-proposal churn conservatism and ranking inversions as low-priority refinements under the frozen >2B/30M priority rule. The absence of a high-priority point in this panel is not a universal closure claim.

The next separate priority is heterogeneous storage roots: salt untouched tail values by account while preserving accessed values, body and gas. Freeze that new panel separately; do not silently add variants or refit this opened holdout. Many-key dirty restore/rollback and hash-path-skew composition remain later targets.

## Tools, qualification and retained evidence

See [the tool README](../../bin/zkgas/bench/README.md#initialized-storage-fanout) and [producer](../../bin/zkgas/bench/cases/storage_fanout.py) for usage, schema and constraints. The host extends `request.program.prestate_contracts[]` with optional `initial_storage` and adds `request.program.transactions.recipients`. This does not change GuestInput, EVM rules, SDK, guest sources, retained ELF or the model. New output directories and configurable paths are required by the producer.

The host passed 168 zkgas tests, scoped Clippy and formatting. Independent qualification checked five legacy profiles byte-exact, distinct A/B/A authenticated roots, acceptance at 16384 aggregate entries and 15 invalid inputs. Producer portable discovery found seven tests (six passed, one optional native skip); the earlier two-case native smoke passed all seven without skips. Broad Python discovery found 236 tests (193 passed, 43 explicit optional skips), followed by focused final CLI/assertion checks. Independent all-24 native/real-quote qualification and seven negative mutants passed before actuals. No guest rebuild was performed; source inspection does not attest the retained ELF build or production SDK parity.

Original real quote bytes retain their command/start/outcome provenance. Actual attempts reuse those bytes only after exact input/key/model/profile/executable/public-output checks, with a retained-source receipt; reuse is not a new quote invocation. Harness corrections and failed qualification diagnostics are retained; no measured input was retried. Every SP1 actual ran sequentially with 6GiB sampled process-tree PSS and 600s guards, after a disk preflight including the existing 320MiB retention reserve. Maximum guarded SP1 process wall time was 14.9067s and peak sampled PSS 3250564096 bytes. These are execute-only observations, not proof generation, GPU throughput or production execution parity.

Independent raw actual auditing passed all 24 observations and all 48 diagnostic contrasts, matching the controller analysis, literal flags, ratios and resource totals. A separate read-only host-namespace audit confirmed all 96 owned shared-memory names absent. The audited joins include raw gas conversion, canonical input/public-output identities, original quote provenance, literal node sums, complete proposal totals, receipts and gas/refunds.

| Identity | Value |
| --- | --- |
| Producer source | `ab6fe10c60a79dabfa1f453c28df1b0856ea67e1` |
| Host source | `2966320fc236adcc50020aaa899037c15b7fd4cb` |
| Model | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Native executable | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Quote executable | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| SP1 executable | `1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b` |
| ELF | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| Frozen config | `a02bd480b417e8b93792adbc21c313e71c984af8183beaaff531cff1659c2016` |
| Pre-actual policy freeze | `ce0dcc557e7bf9ccc01960fa8879f5c28b18d41ea824752c24a0db80cd9a95cf` |
| Input/quote qualification | `86d6cad5432687212293891e56879fe095d1713cd709b4fd18331de646fe408c` |
| Homogeneous-root/dedup audit | `eece0e2a54b70d36006f6265b5068bc42e7c7616ac7e6492dd5efdc60066d2c6` |
| Actual analysis | `0c6d8acf2ab5c84a1ed992888641c2cd316f3054d29ad8b16ab61113b78ad8a5` |
| Independent actual audit | `feb67677f38da8067e65572cd8d0ce115d2dc4cf8ad338f2b0f575d1c89d5f91` |
| Host shared-memory audit | `d55521fb42ef27cd6eca1106925a0775ec32fcdd850da208984b0438d9833e35` |
| Gas-budget report | `26d0cf59b303b4c5d02be710d65afc3e979a022125131b7c86a0e8298ebd3ed8` |
| Local recovery archive | `69ccaebcee4fd2fb419d0bccedef0c3416b7df6bfc685009563671b50bfc69cc` |

Raw requests, canonical inputs, original quote provenance, native diagnostics, actual attempts, resource receipts, controls, failures and hashes are retained locally for recovery and separate immutable Atlas publication. The compressed recovery archive is 20347745 bytes and preserves 872 regular members plus 144 restorable hardlink aliases; four external runtime binaries are identified by hash. Local recovery evidence is not a publicly published fixture. No merge, deployment, price fit or on-chain action is part of this campaign.
