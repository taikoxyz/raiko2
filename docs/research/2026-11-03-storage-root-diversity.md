# Storage-root diversity at equal block gas

This campaign follows [homogeneous storage fanout](2026-11-02-storage-fanout.md) with 32 new execute-only observations and 16 immutable same-program homogeneous observations reused as references. All 32 new inputs completed. There are 21 positive literal node gaps: all 16 own-N0 controls and five of 16 full-body points. The largest full-body gap is 52,532,202 PGU at disjoint-keys/A64/S16384/update/N84; the largest overall gap is 159,003,703 PGU in that group's own-N0 control. Complete-proposal estimates cover every observation. None exceeds the frozen >2B actual-minus-node priority threshold within 30M charged block gas. These smaller gaps remain recorded as low-priority refinements.

Root diversity raises actual PGU relative to every matched homogeneous reference, but does not establish a global storage bound. Background-value salting preserves substantial internal-node sharing; disjoint raw key domains remove shared selected storage support in these inputs. The observations concern whole proposals, with no actual PGU allocated to a transaction, operation or trie.

## Finite program and gas domain

The panel crosses profiles `background-values` and `disjoint-keys`, A=8/64 accounts, aggregate S=8192/16384 initialized entries, update/delete and own-N0/full levels. Each proposal has one block with 64 ordinary signed top-level CALL users, an unchanged guarded runtime and round-robin recipient schedule. There is no new A1 profile. Setup is outside the measured block. The unused primary runtime remains STOP; user bytecode contains no CALL-family opcode.

Let K=S/A, V=64/A, F=full N and account index i start at zero. Own-N0 and full pairs use exactly the same initialized map within each profile/A/S/mode group:

- `background-values` (B): raw phase slot 0 is 1; raw body slots 1 through V*F are 1; untouched raw slots V*F+1 through K-1 contain i+2. All other slots are absent. The full-N tail boundary applies even to own-N0. Every accessed key and value remains identical to the homogeneous program.
- `disjoint-keys` (D): B_i=(i+1)*65536; raw phase slot 0 contains B_i+1; raw slots B_i+1 through B_i+K-1 contain 1; all other slots are absent. Body keys shift and phase control values differ from the homogeneous program. Body values, runtime and gas remain unchanged. User j starts at B_i+1+floor(j/A)*N and commits B_i+1+(floor(j/A)+1)*N; each account ends at B_i+1+V*N.

Update changes each body value from 1 to 2; delete changes it from 1 to 0. Before-value, count, checksum and phase guards constrain execution. Every row has A distinct authenticated initial storage roots and A distinct authenticated final storage roots. Distinct roots alone do not imply distinct internal nodes.

| Mode/level | Body N/user | Main writes/block | Gross gas/user | Applied refund/user | Charged block gas | Signed envelope/user |
| --- | --- | --- | --- | --- | --- | --- |
| update/full | 84 | 5376 | 465927 | 0 | 29986683 | 466927 |
| delete/full | 106 | 6784 | 581075 | 116215 | 29918395 | 582075 |
| update/own-N0 | 0 | 0 | 23468 | 0 | 1669307 | 466927 |
| delete/own-N0 | 0 | 0 | 23468 | 0 | 1669307 | 582075 |

Each row also executes 64 phase writes. Delete's full refund counter is 508800/user and is capped to one fifth of gross gas. Charged block gas includes the 167355-gas Anchor. The header remains 31M, zero/full pairs retain the full envelope, each ordered prefix reserves the next envelope, and users remain within the 15M tool and 16777216 protocol caps. Full N is the existing fanout producer's maximum for this guarded program under its gas, envelope, present-slot and distinct-pair constraints, not a maximum over arbitrary programs.

## Literal same-proposal results

Actual PGU is SDK 6.3 `ExecutionReport.gas * 10 // 191` for the whole proposal. Node PGU is the literal sum of per-block known-PGU ceilings; with one block here, it equals that block's ceiling. The complete estimate includes proposal-once terms. Raw Unzen values are not compared with actual PGU. Positive gaps are `max(actual-estimate, 0)`; zero denotes coverage, not precision.

B/D abbreviate the two profiles above. Each final column subtracts the old homogeneous actual at the same A/S/mode/N, as printed in [the preceding panel](2026-11-02-storage-fanout.md#literal-same-proposal-results). Those are 16 unique original observations reused twice, not 32 new reference measurements. Their raw inputs, original native/quote/actual reports, receipts and frozen tool identities are bound by the reference index.

### Full-body points

| Profile | Mode | S | A | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive full gap | Actual minus old reference PGU |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| B | delete | 8192 | 8 | 413722808 | 562382095 | 1966281678 | 0 | 0 | 27385793 |
| B | delete | 8192 | 64 | 409939627 | 564712255 | 2062102422 | 0 | 0 | 32085411 |
| B | delete | 16384 | 8 | 487400448 | 562382095 | 1857208062 | 0 | 0 | 64914465 |
| B | delete | 16384 | 64 | 486785384 | 564712255 | 2280151846 | 0 | 0 | 76124134 |
| B | update | 8192 | 8 | 400273974 | 450590165 | 829967300 | 0 | 0 | 26782516 |
| B | update | 8192 | 64 | 399953486 | 452920325 | 835855804 | 0 | 0 | 32164689 |
| B | update | 16384 | 8 | 450491129 | 450590165 | 909591988 | 0 | 0 | 50164636 |
| B | update | 16384 | 64 | 446510287 | 452920325 | 911743628 | 0 | 0 | 54919974 |
| D | delete | 8192 | 8 | 493046462 | 562382095 | 2025901710 | 0 | 0 | 106709447 |
| D | delete | 8192 | 64 | 489509708 | 564712255 | 1858684590 | 0 | 0 | 111655492 |
| D | delete | 16384 | 8 | 555102208 | 562382095 | 1870704686 | 0 | 0 | 132616225 |
| D | delete | 16384 | 64 | 565814338 | 564712255 | 2187673342 | 1102083 | 0 | 155153088 |
| D | update | 8192 | 8 | 461453045 | 450590165 | 863930092 | 10862880 | 0 | 87961587 |
| D | update | 8192 | 64 | 460911240 | 452920325 | 861602028 | 7990915 | 0 | 93122443 |
| D | update | 16384 | 8 | 502532432 | 450590165 | 939364484 | 51942267 | 0 | 102205939 |
| D | update | 16384 | 64 | 505452527 | 452920325 | 942452492 | 52532202 | 0 | 113862214 |

All eight B full points are covered by literal node estimates. D misses all four update points and A64/S16384/delete. The maximum actual is 565,814,338 PGU at D/A64/S16384/delete/N106. Across full-body points, node/actual is 0.8961–1.3775 and complete-proposal/actual is 1.8646–5.0303. The absence of a >2B point leaves all five full node deficits visible.

### Preserved own-N0 controls

| Profile | Mode | S | A | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive full gap | Actual minus old reference PGU |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| B | delete | 8192 | 8 | 173604963 | 21585780 | 179284690 | 152019183 | 0 | 467889 |
| B | delete | 8192 | 64 | 179970532 | 23915940 | 190719034 | 156054592 | 0 | 2587867 |
| B | delete | 16384 | 8 | 173776947 | 21585780 | 179576690 | 152191167 | 0 | 587118 |
| B | delete | 16384 | 64 | 181163105 | 23915940 | 193087034 | 157247165 | 0 | 3329230 |
| B | update | 8192 | 8 | 173697521 | 21586036 | 179368946 | 152111485 | 0 | 557309 |
| B | update | 8192 | 64 | 179980134 | 23916196 | 190719290 | 156063938 | 0 | 2596865 |
| B | update | 16384 | 8 | 173782762 | 21586036 | 179576946 | 152196726 | 0 | 592912 |
| B | update | 16384 | 64 | 181171230 | 23916196 | 193087290 | 157255034 | 0 | 3335841 |
| D | delete | 8192 | 8 | 173803206 | 21585780 | 179479458 | 152217426 | 0 | 666132 |
| D | delete | 8192 | 64 | 181961200 | 23915940 | 193557970 | 158045260 | 0 | 4578535 |
| D | delete | 16384 | 8 | 173871328 | 21585780 | 179638458 | 152285548 | 0 | 681499 |
| D | delete | 16384 | 64 | 182911642 | 23915940 | 195487322 | 158995702 | 0 | 5077767 |
| D | update | 8192 | 8 | 173806999 | 21586036 | 179479714 | 152220963 | 0 | 666787 |
| D | update | 8192 | 64 | 181964059 | 23916196 | 193558226 | 158047863 | 0 | 4580790 |
| D | update | 16384 | 8 | 173874842 | 21586036 | 179638714 | 152288806 | 0 | 684992 |
| D | update | 16384 | 64 | 182919899 | 23916196 | 195487578 | 159003703 | 0 | 5084510 |

N0 retains 64 signed users, the recipient schedule, full envelope and phase program; it is not an empty proposal. All 16 controls have literal node deficits. All 16 matched full-minus-own-N0 node increments and complete-proposal increments cover actual increments, but those diagnostics neither erase the literal gaps nor isolate operation costs.

## Root/hash sharing and matched contrasts

All 32 new actuals exceed their matched old homogeneous actuals: differences range from 467,889 to 155,153,088 PGU. The largest full-body difference is D/A64/S16384/delete. All 16 B→D contrasts also increase actual, by 92,080–79,570,081 PGU overall; the eight full-body differences are 52,041,303–79,570,081 PGU. Node quotes stay identical in every B→D and new-minus-homogeneous matched contrast because the literal node selectors are unchanged.

For S8192/update/full, authenticated selected storage support and frozen quoted MPT selectors show the difference between roots and internal sharing:

| Profile | A | Serialized nodes / bytes | Unique storage support nodes | Logical support sum per address | Reused support hashes | Quoted hydration hash blocks | Quoted block materialization units |
| --- | --- | --- | --- | --- | --- | --- | --- |
| B | 8 | 2769 / 357731 | 2753 | 8080 | 5327 | 4152 | 18823 |
| B | 64 | 2308 / 342976 | 2211 | 8448 | 6237 | 3536 | 19649 |
| D | 8 | 8168 / 581214 | 8152 | 8152 | 0 | 9590 | 18956 |
| D | 64 | 8051 / 581971 | 7954 | 7954 | 0 | 9492 | 18702 |

For comparison, the old homogeneous A8/A64 inputs serialize 1026/229 nodes and 73685/20038 bytes, with only 1010/132 unique storage nodes but logical per-address sums 8080/8448. The new B roots are distinct while retaining 5327/6237 repeated support hashes across addresses. D has zero repeated selected storage-support hashes in these examples. Across the complete new panel, D has no repeated selected storage-support hash; this finite support observation is not a theorem about arbitrary disjoint raw keys or all trie nodes. Serialized counts/bytes, authenticated logical support and quote selectors are diagnostics, not observed runtime allocations.

Quoted hydration for the same A8/A64 update examples rises from homogeneous 1212/260 to B 4152/3536 and D 9590/9492 hash blocks. B's block materialization remains 18823/19649 units, matching the homogeneous program, while D quotes 18956/18702. Higher actual therefore does not imply every quoted work selector must rise.

All 16 S16384−S8192 contrasts increase actual at fixed profile/A/body/gas. The eight full-body differences range from 41,079,387 to 76,845,757 PGU. Node is unchanged. The complete estimate moves opposite to actual for full delete at A8 in both profiles: B falls 109,073,616 PGU while actual rises 73,677,640; D falls 155,197,024 while actual rises 62,055,746. All four B A8→64 full contrasts decrease actual while node and complete proposal estimates increase. D A8→64 full actual decreases at S8192 but increases at S16384; account fanout is not uniformly cheaper.

A profile ranking inversion is especially visible at A64/S8192/delete: B→D actual rises 409,939,627→489,509,708 PGU, while the complete estimate falls 2,062,102,422→1,858,684,590. Quoted block hydration rises 3638→11731 hash blocks, materialization falls 22529→21191 units, and conservative structural churn falls 2945→2339 units. At the unchanged 400000-PGU/unit price, that quoted churn allowance falls 1,178,000,000→935,600,000 PGU; each exceeds the corresponding whole-proposal actual. This identifies quote conservatism and ranking limits, not actual churn cost. The retained analysis preserves all 96 diagnostics: 16 own-N0, 16 state, 16 account, 16 profile and 32 new-minus-homogeneous contrasts, including controls.

## Ordinary-transaction usability and disposition

Keep prices unchanged. Repeated storage updates and deletes resemble ordinary transaction work, so synthetic construction alone does not make overquote harmless. Full node overquote reaches 1.38x and complete-proposal overquote reaches 5.03x; an admission policy using those estimates can lose usable capacity. Do not substitute a complete-proposal quote into a per-transaction node filter. No new node limit is selected, and historical normal-transaction admission compatibility is not certified by this panel.

Record the five full node misses, all control gaps, quote ranking inversions and conservative full-proposal churn as low-priority refinements under the frozen >2B/30M rule. This finite root-diversity panel does not close arbitrary storage compositions, dirty restore/rollback or hash-path-skew coverage. No fit is performed on this opened holdout, and no universal bound or production accuracy claim follows.

## Tools, qualification and retained evidence

See [the tool README](../../bin/zkgas/bench/README.md#storage-root-diversity), [producer](../../bin/zkgas/bench/cases/storage_root_diversity.py) and [short specification](../superpowers/specs/2026-11-03-storage-root-diversity.md). This tools-only extension reuses authoritative fanout/runtime/gas helpers. It changes no host, GuestInput, guest, SDK, ELF, model, price, dependency or production pin. Existing producers and old data remain byte-exact.

Producer discovery ran eight tests: seven passed and one explicit optional native skip; the separate native smoke passed its one test over two full cases. Nine relevant modules ran 96 tests: 90 passed and six explicit optional skips. Independent source/specification review and all-32 native/real-quote gates passed before actuals. Qualification independently checked all 2048 signed recipients, 65 receipts/input, concrete reads/writes/originals/warmth/refunds/gas, complete canonical initial/final states, distinct per-account roots and conservative mutation proofs. Nine mutants were rejected, including three native-success cases with unintended key/value semantics. Native success alone was not the semantic oracle.

Original real quote bytes retain command/start/outcome provenance. Each actual reused only its exact same-input/key/model/profile/executable/public-output-qualified quote with a retained-source receipt; reuse is not a fresh quote invocation. The 16 old homogeneous actuals were not rerun. The source, inputs, quotes, tools, reference index and policy were frozen before 32 sequential execute-only attempts, each with 6GiB sampled process-tree PSS and 600s guards, a 256MiB minimum-free check and the 320MiB retention reserve. There was no implicit retry. Maximum SP1 guard wall time was 14.8261s and peak sampled process-tree PSS was 3238534144 bytes. These are execute-only observations, not proof generation, GPU throughput or production parity.

Independent raw actual auditing passed all 32 observations, all 96 diagnostic contrasts and all 16 old reference identities, matching controller values, directions and deficit flags. The audited joins cover raw SDK gas conversion, canonical input/public-output identities, original retained-quote provenance, native signed receipts/roots/gas/refunds, literal node sums, complete proposal totals and resource receipts. A separate read-only host-namespace check confirmed all 128 owned shared-memory names absent; the raw actual audit itself did not certify live host absence.

The retained ELF has source-declared revision `4580faf7dc009714b310fd625b5810667019d683`; that declaration and source inspection do not attest its build or production SDK parity. No guest build occurred. The new research Markdown requires no Rust checks; previously reported old-head CI is not evidence of new-head CI.

| Identity | Value |
| --- | --- |
| Producer source | `b88d3457d26257bfb5e31d79e770fe0af7bc5e4c` |
| Model | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Retained ELF / SDK | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` / `6.3.0` |
| Frozen config | `414874175cda020e015c1c66114cc8aa31f99a80176c0c582d734f8cbb9bbaaa` |
| Pre-actual policy freeze | `c72b80ac9816b118dbb70c7ac6bfc0379ede391e82eff3d0842ebd833437ba8c` |
| Old reference index | `2ae9a8c0cc29fc9e0fd379c7277aafca3871f0bfd09577031c083bdc2f01cb1d` |
| Input/quote qualification | `9774b9f6abee5444d2171fa4183720a591238f5fbdaabae1404b9959d0b4ad40` |
| Root/hash/quote-selector audit | `ad804ed0f42f7d4b62baa4b877bcda01bc4e24e226d5f0bf9f2a9c253ccd17d2` |
| Actual analysis | `28f5f1afff9a676c4861ab0bf8b8deef0afca880e4b52bf9d5b194736d30643d` |
| Gas-budget report | `cf9e4d02a2a467a1b4e1fac6b753887436520cd2cdef054d52af9de66ebf0835` |
| Independent actual audit | `d96b34743cd22f7bf79e27a50e13b1482c59d11332ebb8c6eec73f8ea1165855` |
| Host shared-memory audit | `7b981d4817a713b62f0677545ac1b439d92ecc87293597bcd950dcb4bb60e6e9` |
| Independent evidence manifest | `1ac12c9169844b559610954af16e59eb03fb07bb61c56cb7b694b2368853918e` |
| Local recovery archive | `626b364d5d073c2c825145d6f2d0bbdc828eedf53244e91bc5fae6ed7259c358` |

Raw new requests/inputs, original quotes and provenance, native diagnostics, all 32 actual attempts, controls, resource receipts and the 16 old references/core evidence are retained for recovery and separate immutable data-only Atlas publication. The compressed local recovery archive is 51669916 bytes, with 1157 regular members and 192 restorable hardlink aliases; four external runtime artifacts are identified by hash. Its manifest explicitly records prior helper dependencies, so it is not a standalone independent-auditor runner. Local recovery evidence is not a published Atlas fixture. No merge, deployment, repricing or on-chain action is part of this campaign.
