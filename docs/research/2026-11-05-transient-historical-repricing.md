# Repricing retained synthetic transient-storage workloads

Current model5f covers all four retained full-R transient-storage workloads and all eight
complete-proposal actuals. Two K16/R0 node gaps remain, maximum144,238,337PGU; none meets
the >2B positive-gap-within30M-gas priority screen. This round adds eight current quotes of
the same eight canonical inputs, zero native constructor runs, zero new SP1 actuals and zero
measurement retries. Pricing is unchanged.

## Workload and evidence identity

The [original transient panel](2026-11-03-transient-storage.md) was measured on November3.
It spans root STOP/REVERT, K16/4096 and own-R0/maximal uniform full-R: eight synthetic
post-Unzen inputs, not historical onchain Mainnet proposals or eight additional samples.
Four user transactions share one code owner, with transaction-scoped transient state reset
between users. Each guards sentinel0, seeds body keys1..K from0 to5, runs R dirty5→7→5
pairs per key, cleans body keys to0 and checks the sentinel/counters before root exit.
R0 retains seed/cleanup and its paired full envelope; it is not an empty control. User
persistent-storage reads/writes are zero; transient keys are not persistent MPT targets.

The retained constructor inputs, native receipts, canonical GuestInput bytes, public outputs,
old quotes and raw actuals were preserved byte-for-byte. Current quote1390 decoded and
replayed those eight original bincodes with model5f. That native replay inside the quote
CLI is distinct from a new native fixture-constructor execution or a new SP1 measurement.
No compatibility decoder was added.

| Identity | Retained value |
| --- | --- |
| Old quoted **new-model snapshot**, not Unzen | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Current model | `5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11` |
| Historical SP1 estimator executable SHA256 | `1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b` |
| Historical report `source_revision` | `null`, unchanged |
| Current quote executable SHA256 | `1390ee779fd09e03dacc4234624f01828ae93dc8995099f4ec8272f64ee69e0e` |
| Original quote executable SHA256 | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| Original native constructor SHA256 | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Historical ELF SHA256 | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| Historical program manifest SHA256 | `8d5b4a482d26e3bdca94c5e66b9de7b33dc57bcfbaa9b9c24140bf030c1f93a9` |
| Historical SDK / core executor / runner | `6.3.0` / `6.3.0` / `6.3.0` |
| Reviewed original producer commit | `226795a2db96d0cd11c3341290014f7e799e940c` |
| Producer file SHA256, still matching the original catalog | `f2d50dcc393db7fdc788238126892318a62bc69db5b873d3a0e955f07a56f394` |
| Tools source base at repricing | `8951ebb9f3a4201747448e24fe367379800efc5e` |

The original raw gas is normalized as `floor(ExecutionReport.gas * 10 / 191)` and checked
against each retained `prover_gas`. Input length/hash, public output, successful estimator
exit, SDK/core/runner, ELF, program/SDK manifest and lock identities were checked separately.
Executor options match; retained preset lists have the same unique five members with differing
serialization order. Only a copied comparison object sorts that list; raw reports remain
unchanged. Every other executor option is compared exactly.

The old1998/null reports were not relabelled as the newer register observer, and the strict
current-observer verifier was not relaxed. Fresh quote canonical-input/replay validation and
historical raw-report identity validation are separate gates. The quote executable's SHA and
dirty package metadata are retained; a later source commit is not assigned to that binary
as an embedded revision. These checks validate this historical b76/SDK6.3 program context,
not current production ELF/SDK equivalence, a generated proof or GPU throughput.

## Literal results

Node is the literal per-block known estimate summed over the one-block proposal. Full is
the completed proposal estimate, including proposal-once work exactly once. Both are PGU
estimates under the declared new-model snapshot; raw Unzen difficulty is not used as PGU.
Signed gaps below are `actual - estimate`: positive means underpricing, negative means
conservative overpricing. Block gas includes Anchor. The full-R selectors are not a proof
of optimal packing for arbitrary transactions.

| Root exit | K | R | Charged block gas | Actual PGU | Current node PGU | Current full PGU | Actual − node | Actual − full |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| STOP | 16 | 0 | 338,091 | 163,323,352 | 19,085,015 | 174,045,590 | +144,238,337 | -10,722,238 |
| STOP | 16 | 2117 | 29,990,635 | 576,400,963 | 11,622,779,816 | 11,777,740,391 | -11,046,378,853 | -11,201,339,428 |
| STOP | 4096 | 0 | 21,831,531 | 593,210,792 | 2,710,647,270 | 2,865,607,844 | -2,117,436,478 | -2,272,397,052 |
| STOP | 4096 | 2 | 28,974,955 | 689,924,724 | 5,515,586,372 | 5,670,546,947 | -4,825,661,648 | -4,980,622,223 |
| REVERT | 16 | 0 | 338,115 | 163,434,782 | 39,086,347 | 194,046,922 | +124,348,435 | -30,612,140 |
| REVERT | 16 | 2117 | 29,990,659 | 795,396,957 | 11,642,781,148 | 11,797,741,722 | -10,847,384,191 | -11,002,344,765 |
| REVERT | 4096 | 0 | 21,831,555 | 622,243,962 | 2,730,648,601 | 2,885,609,176 | -2,108,404,639 | -2,263,365,214 |
| REVERT | 4096 | 2 | 28,974,979 | 772,625,160 | 5,535,587,704 | 5,690,548,279 | -4,762,962,544 | -4,917,923,119 |

All four full-R cases are node-covered, all eight full-proposal estimates cover actuals,
and the only two node gaps are K16/R0 STOP144,238,337PGU and REVERT124,348,435PGU.
These deficits remain explicit; proposal-once startup is not multiplied per hypothetical
block to erase or extrapolate them. Synthetic dirty-store full-R quotes are very conservative.
Neither that conservatism nor these finite <=30M-gas observations establishes normal onchain
admission usability or a universal30M-gas bound.

## Exact additive change

For all eight inputs, the real current quotes report node/prove persistent-journal work54,
with proposal-once journal work0:

```text
node journal_work_units = prove block-local journal_work_units = 54
journal charge = 54 * 384 = 20,736 PGU
proposal_once journal_work_units = 0
new node PGU - old node PGU = new full PGU - old full PGU = 20,736
```

Every non-journal quote object is unchanged after removing only model identity, the newly
added journal fields and their aggregate totals. This comparison includes pricing terms,
resource/MPT/payload selectors, counters and public outputs, not just final PGU values.
No hidden opcode tariff, canonical-input or actual denominator change explains the delta.
The54 units are the persistent/system journal term in these quotes, not a measurement of
transient-map visits and not evidence that TLOAD/TSTORE are persistent journal operations.

The existing c893→5f comparison therefore changes this store-heavy panel only slightly.
It supplies no TLOAD-dominant map-distribution/capacity or caught-child rollback evidence.
The new direct-write/TLOAD family, account-journal correction, L1 frequency sampling and
other compositions remain separate work. No tariff adjustment follows from this repricing.

## Verification and retained-data boundary

All eight quotes ran once under frozen1390/model5f, `evm-gas-only`, with the existing process-tree
guard:2GiB PSS,180s,16MiB output,2MiB log and576MiB free-disk floor. Each stage exited
successfully with no failure; observed quote PSS was17,161,216–29,944,832bytes and wall time
0.088–0.926s. No new SP1 execution or shared-memory cleanup measurement is claimed.

A preserved prelaunch qualification failure compared the historical preset arrays in order.
It occurred before any quote/native/SP1 launch. The correction compared duplicate-free
preset membership while keeping other options exact; it was not a measurement retry or a
change to the strict current observer report contract.

Independent readback checked all72 source/input/tool/model/guard bindings and62 final-manifest
files before/after, all eight old raw reports/native inputs/new quotes, canonical gzip-to-bincode
hash/length equality, original quotes, normalized raw gas, public outputs, guard outcomes,
closed tariff arithmetic, once-only composition and complete non-journal object equality.
It ran no producer gate, native, quote, SP1 or build command. Retained evidence identities:

| Evidence artifact | SHA256 |
| --- | --- |
| Repricing `summary.json` | `02b9fb7bc0e7b080334da97a91e7aedca0dbc6f292a97781d495741bf58d286b` |
| Repricing `final-manifest.json` | `f893b1d17a1e9e76090d9bb80ca17e2618165e586a0f6e1f968b5338b615e1aa` |
| Repricing `source-input-bindings.json` | `d5fdf461ff736c2cea5ee56747cd617a6e20cbe226921e8d6f3e767e62691a57` |
| Independent `REPORT.md` | `3c1cc6c641b6784fc3176e1b3fce4aa381094d364390ee4fdbe3da9c569dfdc0` |
| Independent `readback.json` | `65a6115e15274093c06f306b34d710604d941b581a1147473703ebcf259f4999` |

Canonical inputs, original raw/native reports, both quote snapshots and their per-file
hashes remain retained locally. The Atlas supplement for this repricing has **not been
published**. The hashes above identify evidence, not public download locations; without the
retained inputs they do not provide turnkey public reproduction. Tool source remains in
Raiko, immutable data belongs in Atlas, and no private controller paths are exported here.

The original eight observations remain one opened synthetic cohort, not IID/unopened holdouts
or a new combined coverage denominator. No model/guest/SDK/pin change, new price fit, current
production certificate, arbitrary-gas workload bound, merge or deployment follows.
