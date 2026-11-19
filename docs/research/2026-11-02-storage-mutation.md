# Persistent storage mutation: one initialized account

This campaign follows [unreachable code preparation](2026-11-02-code-analysis-tail.md) with committed many-leaf storage work. It keeps literal node-known PGU, complete-proposal estimates and actual same-proposal SP1 PGU separate. Prices remain frozen at c893.

All 24 execute-only SP1 observations completed: four modes, two background-state sizes and zero/64/maximal-body levels. Six of eight near-full node estimates cover actual; the two update points underquote by 7,842,757 and 54,353,316 PGU. Across all 24, 18 node shortfalls remain visible, maximum 164,072,990 PGU at update/S16384/N64 (3,057,331 charged block gas). Complete-proposal estimates cover all 24. No positive actual-minus-node gap exceeds the 2B high-priority threshold.

The largest whole-proposal actual is 569,314,904 PGU. The eight near-full node/actual ratios span 0.8940–1.6056; complete-proposal/actual spans 1.8834–4.3584. This is finite block-level evidence, not a global workload bound or a normal-transaction admission certification.

## Valid gas domain and concrete state

- Eight signed ordinary CALL transactions execute one shared short runtime in one block. Each user has a disjoint body range, selected by raw slot 0, which starts at 1 and commits the next range start. Phase reads/writes count in gas, opcode/resource observations, proofs and the final storage root.
- Exactly 8192 or 16384 initial raw slots contain 1, including phase slot 0. Update writes 2 to present keys; delete clears them; insert writes 2 to absent keys at offset 1048576; mixed clears each present key and inserts its absent counterpart. These are analogues of ordinary batch mapping operations, not automatically irrelevant because constructed. Setup/deployment is outside the measured block.
- Before-value, counter, checksum and final-phase guards constrain the program. An independent byte interpreter checks ordered actual keys/values, per-transaction original/current storage, cold/warm access and refunds, including mutations that preserve successful STOP/counts but target the wrong keys.
- Per-user gross gas and charged gas remain distinct. Delete refunds are capped at one fifth of that transaction's gross gas. The uniform signed envelope is full-body gross+1000 at every level; each prefix must have enough remaining header gas for the next envelope. All users remain below the existing 15M tool cap and Unzen/Osaka's 16,777,216 protocol cap. Header gas remains 31M, while charged block gas including the 167,355 fixture Anchor is bounded by 30M.

| Mode | Body N/user | Main writes/block | Gross gas/user | Applied refund/user | Charged block gas |
| --- | --- | --- | --- | --- | --- |
| update | 707 | 5656 | 3726709 | 0 | 29981027 |
| delete | 885 | 7080 | 4658361 | 931672 | 29980867 |
| insert | 165 | 1320 | 3714351 | 0 | 29882163 |
| mixed | 163 | 2608 | 4510401 | 782400 | 29991363 |

The full N is maximal for this guarded program, initialized state, proof-slot cap, envelope reserve and ordered admission constraints. It is not a maximum over every bytecode or block composition. Independent qualification covers all 192 signed users, actual receipt gas, full initial/final storage roots and counts, proof targets and mutation support.

Witnesses use the canonical mutation-aware ProofRetainer. It conservatively retains connected siblings for potentially removed descendants and insertion splits; this is not mathematical minimum serialization. The independent audit authenticates every node against the actual initial account/storage tries, records permitted support hashes/reasons and rejects unrelated/disconnected padding. An initially overstrict final-collapse-only verifier was corrected without rerunning completed native executions. All original verifier diagnostics remain retained.

## Literal same-proposal results

| Mode | Background slots | Actual SP1 PGU | Literal node PGU | Complete proposal PGU | Positive node gap PGU | Node / actual |
| --- | --- | --- | --- | --- | --- | --- |
| delete | 8192 | 497185672 | 571482289 | 2166913960 | 0 | 1.1494 |
| delete | 16384 | 569314904 | 571482289 | 2054663256 | 0 | 1.0038 |
| insert | 8192 | 240328252 | 385869508 | 849600299 | 0 | 1.6056 |
| insert | 16384 | 253459281 | 385869508 | 825883395 | 0 | 1.5224 |
| mixed | 8192 | 304826798 | 479431492 | 1196987515 | 0 | 1.5728 |
| mixed | 16384 | 326995546 | 479431492 | 1150967675 | 0 | 1.4662 |
| update | 8192 | 466262494 | 458419737 | 885512272 | 7842757 | 0.9832 |
| update | 16384 | 512773053 | 458419737 | 965770208 | 54353316 | 0.8940 |

All eight zero and eight small points remain in the retained results with their literal denominators. Do not remove the baseline gap from the primary metric. In matched own-N0 diagnostics, 12/16 node work increments and 16/16 complete-proposal increments cover actual increments. The four node increment misses are small-N update/delete contrasts, about 4.62M–10.65M PGU; all eight full-body increments are covered. These proposal differences do not assign actual cost to an opcode, transaction or isolated MPT feature.

| Mode | Actual full S16384−S8192 PGU | Node difference scaled | Complete-proposal difference scaled |
| --- | --- | --- | --- |
| delete | 72129232 | 0 | -112250704000000000000000000 |
| insert | 13131029 | 0 | -23716904000000000000000000 |
| mixed | 22168748 | 0 | -46019840000000000000000000 |
| update | 46510559 | 0 | 80257936000000000000000000 |

Scaled values use 10^18 units per PGU. At equal body and EVM gas, larger background state increases actual work in all four full comparisons. Node cannot distinguish that background topology in its current surface. Complete-proposal estimates move in the opposite direction for delete, insert and mixed: their conservative structural-churn term decreases while actual rises. Thus coverage alone does not certify workload ranking or precision.

For delete/S8192/full, 3062 structural branch/extension churn units at the existing 400,000 PGU price contribute 1,224,800,000 quoted PGU, out of 2,166,913,960 complete-proposal PGU; actual whole-proposal PGU is 497,185,672. This identifies a dominant quote allowance, not actual attribution to churn. Node for that case is 571,482,289 PGU (about 1.15x actual). Structural-churn precision and state-size ordering remain refinement TODOs.

## Ordinary transaction usability

The current evidence does not justify saying every overquote is harmless. The generated shapes resemble usable storage batching, and a future admission limit is not selected here. Their node quotes range from roughly 0.89x to 1.61x actual at near-full gas; this panel found no severe node overquote of these batches. The complete-proposal estimate can overquote by 4.36x and must not be substituted into a per-transaction node filter. If later used for a proposal budget, that conservative term can reduce capacity and needs separate calibration. Existing historical compatibility regression remains necessary; these synthetic cases do not replace it. No price is changed, so this round introduces no new tariff-induced historical quote change. Raw Unzen values remain their own score, never PGU.

## Retained tools and observation cost

Usage is in [the tool README](../../bin/zkgas/bench/README.md), including `cases/storage_mutation.py`, its semantic/native tests and composable `workload.py measure --quote-report`. The host fixture now permits storage_state with 1–8 CALL users in one block and exposes optional committed storage-root/count diagnostics. Default/legacy native outputs remain byte-identical. GuestInput, model, SDK, ELF and production pins are unchanged.

Native construction initially collected unused MPT snapshots after every mutation, through Cargo feature union, then repeated a full pricing replay solely to obtain public output. The full update point was deliberately cancelled after 462.95s, with its request/log/cancellation evidence retained. Per-instance unobserved validation now skips unused observation while preserving the canonical execution and proposal validation pipeline. The same full construction completed in 3.52s in the author probe; independent native qualification completed each point within 6.57s. These timings are host instrumentation measurements, not PGU or proving speed.

Real full quotes still collect unchanged MPT selectors and prices. Their native diagnostic collector remains expensive (maximum 392.77s in this panel); each was executed once, bounded at 1800s and with four independent processes. Actual measurement reuses original quote bytes only after exact key/input/model/profile/executable/public-output validation, with a retained-source receipt and no fabricated quote command/outcome. Further measured-collector scaling is a tooling TODO.

The source changes passed 161 zkgas tests, 55 stateless tests and 230 Python tests (42 explicit optional skips), scoped production-library Clippy, zkgas test Clippy, formatting and feature-off host compilation. The broader three-package test-target Clippy lane surfaced nine preexisting stateless test-style warnings; those were not suppressed or presented as passing. No guest build or packaged-artifact fingerprint refresh was performed. The retained workload ELF remains tied to its declared older source, not a new build attestation or deployed-program accuracy certification.

Independent source review, native/quote qualification and all 24 raw actual audits passed. Actuals use SDK 6.3 and `ExecutionReport.gas * 10 // 191`; maximum SP1 wall time was 14.39s and sampled process-tree PSS 3240607744 bytes under the 6GiB/600s sampler guards. A separate read-only host-namespace check confirmed all 96 owned shared-memory names absent. These are execute-only observations, not generated proofs or GPU-throughput measurements.

| Identity | Value |
| --- | --- |
| Frozen source | `2f6892382521dec2bfe4cdf22283a19475f367c0` |
| Model | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Native | `da7365a71b17c9eac306c56d6c7470e3b3105a8bccdbf99236b55d2c1bb31477` |
| Quote | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| ELF | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| Frozen config | `37d52f7222f6d8f36bcb792665bbf34c7718cf1942d209473ba7419540a59c10` |
| Pre-actual policy freeze | `25496b714e6ffb868aa78aa29b912fc62304f5f3a0d40d240601799fa7781f96` |
| Actual results | `c02813a944ac3127ad09e01939ff9f0c35b23072c0565cd733b64b8653dfbe88` |
| Independent actual audit | `6d6f28e8aa1ceec9c4adf253f3ca0c2d83eec5b0ebf824ef37ca471dbe3383db` |

## Disposition

Keep pricing unchanged. Record the literal node gaps, small increment misses, full-proposal churn overquote/ranking and diagnostic scaling as low-priority refinement work under the current >2B/30M priority rule. A finite absence of a high-priority gap is not a global closure claim.

The next distinct coverage target is initialized storage spread across independent account tries. Separate or avoid CALL-wrapper reserve masking (seen in the earlier multi-account code campaign); top-level multi-recipient transactions are a useful candidate if the fixture gains that capability. Include many-key dirty restore/rollback and hash-path-skew composition later. Keep the same literal three-way metrics and normal-transaction compatibility requirement.
