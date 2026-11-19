# Transient footprint, journaling and transaction reset

All eight new execute-only observations completed once without failure or retry: four own-R0 controls and four generated full points across root STOP/REVERT and K16/4096. Two K16 controls have literal node gaps, with maximum 144,259,073 PGU; no full point has a node gap, no input has a complete-proposal gap and none exceeds the frozen >2B actual-minus-node threshold within 30M charged gas. Maximum actual is 795,396,957 PGU. Full node/actual ratios are 7.16462–20.16436 and complete-proposal/actual ratios 7.36519–20.43321. These conservative synthetic quotes warrant separate usability work; they do not select a tariff cut.

This transient panel is separate from the [236-input persistent-storage cohort](2026-11-03-storage-warm-rollback.md). Its eight inputs are not a newly verified 244-input union. See the [coverage inventory](zkgas-workload-coverage.md) for remaining interactions.

## Finite programs and gas domain

Four signed ordinary top-level CALL users execute one common code/storage owner, with empty persistent initial storage. Each transaction checks transient sentinel slot 0=0, stores 1, seeds keys 1..K from 0 to 5, executes R guarded dirty 5→7→5 pairs per key, cleans 5→0, and verifies sentinel 1 and the body/round/marker counters. STOP leaves the sentinel 1 until transaction finalization; the next transaction's zero guard verifies reset. Root REVERT rolls transient changes back and preserves attempted observations. There are K+1 logical owner/slot pairs across four independent reset transactions, not four owners.

Per user, reads total 2+6K including two sentinel reads, stores total 1+K(2+2R), and memory reaches 192 bytes. TLOAD/TSTORE each cost 100 gas with no cold surcharge, storage refund or SSTORE 2300-gas sentry. Persistent aggregate_state_size and state_size_per_account both equal 0; peak_transient_state_entries_per_transaction explicitly equals K+1. Own-R0 retains seed/cleanup and its full point's signed envelope; it is not an empty proposal.

| Terminal | K | Full R/key | Gross=charged gas/user | Signed envelope/user | Charged block gas | Own-R0 block gas |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| STOP | 16 | 2117 | 7455820 | 7456820 | 29990635 | 338091 |
| REVERT | 16 | 2117 | 7455826 | 7456826 | 29990659 | 338115 |
| STOP | 4096 | 2 | 7201900 | 7202900 | 28974955 | 21831531 |
| REVERT | 4096 | 2 | 7201906 | 7202906 | 28974979 | 21831555 |

Anchor succeeds and contributes 167355 gas; user receipts succeed for STOP and fail for REVERT. Every block has a 31M header. The finite solver checks <=30M charged block gas, gross+1000 signed envelopes, <=15M/user fixture cap, 16,777,216 EIP-7825 cap, every ordered admission prefix and 24576-byte code cap. The 1000 reserve is a fixture policy, not an SSTORE rule. Independent full+1 execution uses sufficient gas before testing its own constraints. Both K4096 R0 controls fit; full R2 is below99% occupancy because uniform-R granularity makes the next candidate fail. Only the two K16 full points are near-full. Uniform maximal R is not a global packing maximum.

## Literal same-proposal results

Actual is the entire proposal's SDK 6.3 `ExecutionReport.gas * 10 // 191`. The node estimate is the literal sum of block known-PGU ceilings, here one block; the complete-proposal estimate separately includes input/proposal terms. Positive gaps are `max(actual-estimate,0)`. No raw scalar or per-operation allocation replaces these three quantities.

### Generated full points

| Terminal | K | R/key | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive full gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| STOP | 16 | 2117 | 576400963 | 11622759080 | 11777719655 | 0 | 0 |
| REVERT | 16 | 2117 | 795396957 | 11642760412 | 11797720986 | 0 | 0 |
| STOP | 4096 | 2 | 689924724 | 5515565636 | 5670526211 | 0 | 0 |
| REVERT | 4096 | 2 | 772625160 | 5535566968 | 5690527543 | 0 | 0 |

### Preserved own-R0 controls

| Terminal | K | R/key | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap | Positive full gap |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| STOP | 16 | 0 | 163323352 | 19064279 | 174024854 | 144259073 | 0 |
| REVERT | 16 | 0 | 163434782 | 39065611 | 194026186 | 124369171 | 0 |
| STOP | 4096 | 0 | 593210792 | 2710626534 | 2865587108 | 0 | 0 |
| REVERT | 4096 | 0 | 622243962 | 2730627865 | 2885588440 | 0 | 0 |

## Whole-proposal contrasts and retained counters

The retained eight diagnostics comprise four full-minus-own-R0 and four REVERT-minus-STOP contrasts, with exact scaled node/proposal differences before rounding. All four own-control increments are covered by both estimate increments: actual increases are 413,077,611/631,962,175 PGU at STOP/REVERT K16 and 96,713,932/150,381,198 at K4096. Both estimate increments equal scaled `11603694800803346208439276608` at K16 and `2804939102924043352474583040` at K4096, each divided by 10^18 for PGU.

Each terminal estimate difference is scaled `20001331754694528277460392`/10^18 PGU, approximately 20.001M. Actual terminal differences are 111,430 at K16/R0, 218,995,994 at K16/full, 29,033,170 at K4096/R0 and 82,700,436 at K4096/full. The last three exceed both estimate differences. Absolute full coverage does not erase these incremental ranking deficits, nor do covered own-control increments erase the two absolute control gaps. The corresponding upward-rounded exact increment gaps are 198,994,663 PGU at K16/full, 9,031,839 at K4096/R0 and 62,699,105 at K4096/full. These are complete-program differences, not isolated REVERT or transient-operation costs.

Across the eight comparisons, equality flags total 4 for R, 8 for K, 0 for charged gas, 4 for signed envelope and 8 for authenticated selected persistent-storage support. Support is one authenticated owner mapped to an empty set, with SHA256(empty), not an empty owner dictionary. Equal empty selected storage support does not make account/code/global witness or whole-world state identical. User transient slots are not persistent MPT targets.

Independent tracing confirms user persistent reads/writes are zero. Observed persistent access/write selectors remain 4 system slots, independent of K/R; source inspection confines that resource slot domain to SLOAD/SSTORE (`0x54`/`0x55`), separate from transient (`0x5c`/`0x5d`). Exposed dispatch agrees with attempted user work plus the observed 12363 Anchor allowance, including through REVERT. Three counter negatives reject dropped dispatch or phantom persistent targets. The system allowances are observed fixture values, not independently derived Anchor opcode counts. Aggregate/unique counters do not prove per-opcode occurrence charging.

## Qualification and evidence boundary

See the [producer](../../bin/zkgas/bench/cases/transient_storage.py), [tool README](../../bin/zkgas/bench/README.md) and [binding specification](../superpowers/specs/2026-11-03-transient-storage.md). Final source review binds producer commit `226795a2db96d0cd11c3341290014f7e799e940c`. A catalogue regression first failed because generic persistent size metadata included transient entries; the metadata correction passed while all eight requests/runtime bytes and gas/selection functions remained unchanged. The source bridge retains those identities. Existing helpers and oracles were not modified.

Qualification independently checked eight canonical inputs, 32 signed users, native receipt/gas behavior, authenticated account/code/complete-world/storage-root/witness support, original real quotes and literal transient semantics. It includes seven fresh native calls plus one retained original STOP/K16/full positive smoke, not eight fresh native calls after review. All eight original real quotes are fresh. The original smoke raw JSON hash `86b6f65487431232de25f0aedd7b6b0d49c6058a8d7c051861fcd3af98c23343` differs only in JSON encoding from canonical request hash `b0a51d60d447027e53ad6b2f662f1052f290d546a3ba0cfc59057019c7322552`; object/runtime and canonical input `f41161ef156572e9da6f927e89d579aecdf6c80c0a05098fd35b20c98c5f7a9e` agree. Original source phase and command remain original. Missing original per-call start/elapsed/cap fields were not backfilled; the retained author's combined two-smoke-test duration 8.929s is not individual process timing. Native-accepted dirty9 remains a retained negative rejected by literal intent, not another canonical measurement.

The separate byte oracle checks all old→new values, owner/reset/frame behavior, STOP finalization, REVERT/exception rollback with retained attempts, K16 R0/1/63/64/65, sufficient-gas full+1 and TSTORE at 100/99 gas. Fourteen semantic and nine strict-domain negatives pass. The qualification manifest freezes 156 files. Fresh native bounds are 180s; original quote bounds 1800s with at most two concurrent quote workers.

All eight actuals ran sequentially once under 6GiB sampled process-tree PSS and 600s bounds, with 192MiB output and 16MiB logs. Disk preflight requires 576MiB, comprising 256MiB free-space floor and 320MiB retention reserve. Peak sampled PSS was 4571047936 bytes. No failure or retry occurred. Independent final verification passed all eight raw actuals, eight diagnostics, retained-native/source/serialization bridge and the unchanged persistent 236 identity-only context. It rejoined original raw SDK conversion, canonical input/public output, literal node sums/proposal ceilings, original quote/artifact/source identities, receipts, gas and resource closures without rerunning a measurement. Maximum execution wall time was 22.98740362s. A separate host-context receipt confirms all 32 unique owned shared-memory names absent; raw file verification alone does not establish live host absence. Post-actual closure confirms 160 tracked source files unchanged at the producer commit before these documentation edits. The original 156-file qualification manifest remains unchanged; a separate 169-file final manifest includes the new audit evidence.

| Identity | Value |
| --- | --- |
| Producer source | `226795a2db96d0cd11c3341290014f7e799e940c` |
| Frozen model | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Retained ELF / SDK | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` / `6.3.0` |
| Native executable | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Quote executable | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| Frozen config | `d75e9ebea3a9ce4ac024cd377436702588ea00c4faef39734245899906251ff4` |
| Qualification index | `366d011c0cb0d2c5cc268f9e9218f9ff4d396ea2c6155ea4cf6ab30346e80e70` |
| Retained-smoke bridge | `6678241dc92666541185e1584a750db8acea5c6761dbdbc6448071b2e4d38596` |
| Actual analysis | `c322c0957334d51882668febbfa7e2c090319f440653523ac7b541ba966ac8c0` |
| Frozen pre-actual policy | `6613bac41b6203fbac34dd3811af0052a491a59274ea1eefe7df8d5fede5fc98` |
| Original qualification manifest | `c26aedd8f467a261bd30228c2bb7dac607a0dfbe09d77d518f3e2288742612e4` |
| Counter retention audit | `31b0d6ce9df77d911003e199fa1bbca9dc424138ec3168c0af81327631f7d315` |
| Transient semantics audit | `7077e79c8094adb264467cea83e1c75e92b5b3e18bdc3313ee94f8ef5aac8028` |
| Gas-budget report | `5586dfa41e85ced42c3bd815dce4b21400250abee5e2a449b7560b01ac99f161` |
| Independent raw final audit | `4222eb109751156fd3dedd30cc08ad1364bc1f59517440b3eeefe13ab2d05cef` |
| Independent diagnostic/context checks | `4b5a454677ac77ba49e8d53825d3f612fc2b806138062eedf13261a84e2d639b` |
| Independent actual closure | `eff1dec7e36264389a4fed8285554d103e7a0c4bd896131f8d18623f94a8e0f5` |
| Independent final audit report | `7ce778df2d4c75936fd22cc27934b3088a2c38742564be2492cdc1f2c8f1e0ab` |
| Independent 169-file final manifest | `0c42c08649e2ec60b9587d5c54637fd380e72e7ee52360484443ff0097f4f2e3` |
| Host shared-memory receipt | `dba85dc06cc03466af3ff692552578833e50b7f706c28748b1c8de87da2fbac5` |
| Post-actual source closure | `0912c58626f5b602797ae7a7938b1c2b1e1cce9223e61aa8d6fdb0c94e5a84cb` |

The verified local recovery archive is SHA256 `c2b9a2b5934389a7f3fb1715c9fa9cd0fe7dd2d517448cdf1f6e737a408eafdf`, 1,170,507 compressed bytes, with 308 regular members carrying 12,286,540 payload bytes, 48 restorable duplicate aliases and four external runtime artifacts identified by hash. All streamed member hashes passed verification without extraction. Requests/canonical inputs, original native/quote provenance and smoke bridge, raw actuals, source/helper/artifact identities, controls, resources and independent audits are retained. This is recovery evidence, not a standalone auditor runner or published Atlas fixture.

## Interpretation and remaining work

The frozen model composition helps explain the large quotes: its accepted sealed raw-gas floors imply approximately 8437.508 PGU per 100-gas TLOAD and 42187.541 per 100-gas TSTORE, before ordinary dispatch. For STOP/full, the TSTORE floor supplies 98.405% of the K16 node scalar and 75.194% at K4096. This describes estimate composition, not an allocation of actual PGU to opcodes or a measured replacement coefficient.

Keep prices unchanged and preserve both controls and incremental deficits. Positive actual-minus-node >2B PGU at <=30M charged gas remains the frozen priority; smaller misses remain explicit refinements. Large conservative quotes can reduce ordinary admission capacity. This rare synthetic panel neither establishes a historical two-times overquote revision nor certifies natural-transaction usability. No new admission limit is selected.

The selected transient footprint, journal and transaction-reset cases add finite evidence. General nested transient ownership/rollback, exceptional call graphs, internal key/cache/hash distributions, cross-family composition and order remain open. The persistent 236 snapshot remains immutable and separate. Exact production ELF/SDK parity and representative historical usability require a separate promotion lane. The ELF's declared source revision `4580faf7dc009714b310fd625b5810667019d683` is not a production source attestation. No model fitting, tariff change, Rust/guest rebuild, current-ELF promotion, global/IID bound, proof generation, GPU throughput, merge, deployment or on-chain action follows.
