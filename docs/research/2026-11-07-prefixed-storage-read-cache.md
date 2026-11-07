# Read-only storage across transactions: block-local query scope

The retained `3e335de2` baseline misses a high-priority workload even after its
changed-slot transition correction: **5,280,821,062 actual SP1 PGU versus
2,390,442,635 literal node PGU**, at **28,062,336 charged EVM gas**. The full
proposal estimate is 2,770,850,065 PGU. Both deficits exceed the agreed
2-billion-PGU / at-most-30-million-gas priority screen.

These are complete whole-input measurements, including the genuine prefix,
both Anchor transactions and startup. Raw Unzen difficulty is not PGU. The
finite panels below are closed; no third input or same-input measurement retry
was used to obtain this finding.

## Retained inputs and measurements

Each input contains an Anchor-only block followed by an Anchor and ordinary
CALL users. Each user reads its next 128 initially nonzero body slots, verifies
their sum and advances one persistent phase slot. Body slots are not written.
The 215-byte kernel has no calldata, access list, authorization, CREATE or refund.
The prestate setup is outside the measured proposal.

The first panel fixes 64 users. A separate prospective gas-packing panel fixes
89 users using only `floor((30M - 2M) / 312223) = 89`, before observing its actual
PGU. Its conservative two-Anchor gas bound is 29,787,847; the next integer would
require 30,100,070 under those same allowances. This does not claim that 90 users
could never fit with smaller actual Anchor costs.

Each panel permits one fixed reference and at most one successor selected from
that reference's own independently qualified local observation. The new panel
does not reuse the older panel's selection as evidence.

| Input | Charged EVM gas | Actual SP1 PGU | Literal node PGU | Full proposal PGU | Actual minus node | Actual minus full |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 64-user reference |20,256,761|430,578,358|1,643,693,938|1,961,030,144|-1,213,115,580|-1,530,451,786|
| 64-user sole successor |20,256,761|2,954,417,885|1,643,693,938|1,960,992,560|+1,310,723,947|+993,425,325|
| 89-user reference |28,062,336|534,307,327|2,390,442,635|2,769,944,473|-1,856,135,308|-2,235,637,146|
| 89-user sole successor |28,062,336|5,280,821,062|2,390,442,635|2,770,850,065|+2,890,378,427|+2,509,970,997|

Actual PGU is `floor(raw_SP1_execution_gas * 10 / 191)`. The four raw gas values
are 8,224,046,653; 56,429,381,606; 10,205,269,947; and 100,863,682,288.
The literal 89-user node sum is `[7326859,2383115776]`; the aggregate scaled
ceiling is one PGU lower and must not replace that denominator. Full proposal
pricing is a separate metric. The 64-user deficit alone does not grade the
30M-gas priority question; the 89-user result does so without extrapolation.

The two actual Anchor receipts consume 167,355 and 107,134 gas in every input.
There are 66 or 91 signed transactions and 8,193 or 11,393 initialized storage
entries. Independent verification checks signatures, typed transactions, fresh
state roots and witness carry, exact EVM gas, checksum and per-user phase routes.
Within each pair, transaction objects and Kona data sources are equal. Actual
keys, state roots and witnesses differ; second-block prove-MPT pricing can differ.

## Observed mechanism and scope

The observed path is `State::storage` through the block-local `PlainStorage`
map. It is distinct from the earlier forward `TransitionState` observation.
Independent decoding closes 254 local calls per input, of which 245 match body
keys. Both successor inputs have the selected value in their own sampled hash
chains and actual full hash zero for all 245 observed body calls. Incomplete
intervals, including the capped final parent interval, are excluded.

These observations do not identify a transaction or state owner from a pointer,
prove whole-map placements or population, or isolate PGU for one map. Whole
input PGU comes from complete canonical execution. The optional 400M-instruction
observation prefix is separate and does not truncate the canonical measurement.

The retained3e335 `storage_transition` policy uses all finalized physical Journal keys
to maintain block-prefix population, but gives query budget only to changed
slots: `B = 2X`. A successful read-only materialization can grow and query the
same block-local storage cache without contributing to X. Transaction-local
Journal pricing does not cover that cross-transaction population dimension.

For ordinary valid State execution, a finalized owner's physical key set J
bounds successful read-through queries by one per key per transaction. An
occupied Journal entry avoids another database query; caught or root REVERT
restores values/coldness without deleting the loaded key. Zero-valued and no-op
entries remain physical keys. Backend errors and externally preseeded/custom
state paths require separate qualification; this is not a bound on failed DB
queries, table probes, comparisons or PGU.

## Correction and retained-input validation

Reuse the existing independent committed/attempted block-prefix ledgers. For
owner o and transaction t, merge all incoming physical keys before computing:

```text
K[o] = union of finalized physical J[o,t] in this block prefix
N = |K[o]|; M = largest semantic raw-key low32 group in K[o]
C = ceil((N + 7*M) / 8)
B[o,t] = |J[o,t]| + 2*|X[o,t]|
U += C * B[o,t]
storage reserve PGU = 384 * U
```

The added J term covers read-through query scope; the existing 2X accounts for
the two forward changed-slot passes. The empirical 384 tariff, concentration
proxy, domain and inherited opcode prices remain unchanged. This is a simple
conservative policy, not an implementation-independent upper bound.

Candidate model
`eea6f1da87ee129d2d9f759ae6cef01cb8ac93988376a86efb290398e6db5da6`
passes independent source/model review and12 independently authored behavioral
tests. The author suite passes282 Rust and62 Python tests, generation checks,
formatting and strict production clippy. The consumer passes37 trace and140 CLI
tests, five strict host production lint targets and the existing192-block replay.
The latter's storage component grows from1,622 to3,699units, adding797,568PGU;
its canonical execution output and all unrelated price fields stay equal.

Fresh host quotes reuse the four original actuals without SP1 resampling:

| Input | Original actual PGU | New literal node PGU | New full proposal PGU |
| --- | ---: | ---: | ---: |
|64-user reference|430,578,358|14,835,335,026|15,152,671,232|
|64-user sole successor|2,954,417,885|14,835,335,026|15,152,633,648|
|89-user reference|534,307,327|27,788,992,523|28,168,494,361|
|89-user sole successor|5,280,821,062|27,788,992,523|28,169,399,953|

The near-budget successor is covered at5.262x node/actual and5.334x full/actual.
The artificial reference's52.009x node overquote remains visible. The tariff is
kept conservative for this constructed concentration; these ratios describe
this family rather than normal proposal accuracy.

All86 retained inputs/9,642blocks pass independent arithmetic and field-invariance
verification. Of33 numeric actuals, all full estimates and29 literal node
estimates cover actual. Four smaller node deficits remain; the largest is
153,343,319PGU. No measured remaining deficit exceeds2B. Three controlled actuals
and50 historical numeric actuals remain absent, rather than being filled in.

The50-proposal/9,600-block historical cohort gives fresh price sensitivity:

| Metric |3e335 total PGU |eea6 total PGU | Added PGU | Increase |
| --- | ---: | ---: | ---: | ---: |
|Literal integer-block node sum|92,887,659,232|92,925,858,400|38,199,168|0.041124%|
|Full proposal sum|160,031,170,253|160,069,369,421|38,199,168|0.023870%|

All50 proposals/9,600blocks gain some price. Maximum proposal-node uplift is
0.082066%; maximum block-node uplift is0.102877%. Median/p95 block additions are
3,840PGU and the maximum is36,096PGU. These small changes do not materially
change this cohort's pricing scale. Its older416f program/SDK6.1 inputs supply
price sensitivity, without numeric actuals or current-b76 accuracy certification.
The previous approximately2x natural-proposal observations are not a new
production-SDK parity or universal admission guarantee.

## Artifact and execution boundaries

All four actuals use the frozen `b76baf2e…` ELF and SDK 6.3 streaming estimator,
not a newly compiled guest or production-SDK-6.8 parity run. Original quote
`c9ba7310…` was built from Raiko `08fbc3eb` dirty / Reth `182d5e57` dirty;
later clean source checkpoints do not change that build provenance. Native
`1368d40d…` supplies constructor/schema/execution checks and retains its older
Shadow dependencies. Original3e335 pricing comes from the separate c9 quote;
the repaired EEA quotes use the new401a host executable identified below.

The 89-user successor peaks at 4,575,120,384 process-tree PSS bytes and takes
146.10 seconds, within its 6GiB/600-second guard. Independent original-host
readback confirms the exact producer PID and four owned shared-memory objects
are absent. No blanket process termination or shared-memory deletion was used.

A preparation precondition failed before its marker/helper because a manifest
used label keys where absolute-path identity keys were expected. The failure is
retained; an explicit mechanical metadata projection fixed the contract before
the sole helper invocation. No input or measurement was rerun to repair it.

The reusable [prefixed-read helper](../../bin/zkgas/bench/README.md#phase-advancing-readonly-storage-bodies)
and tests remain in Raiko. Immutable fixture packaging belongs to Atlas; runners,
decoders and pricing implementations do not belong in the fixture repository.
The four original inputs/3e335 quotes/actuals are available in the
[immutable read-cache fixture](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-prefixed-storage-read-cache-v1).
Its9,672,438-byte archive hashes to
`d2a96ab9ba9093fb3d98770d87303e76e23deeddd6fc54222c1266e55151f0e2`.
Independent genuine remote download and README/CI integrity verification pass;
published earlier revisions are unchanged. The repaired quotes above are separate
consumer evidence and do not overwrite that fixture's original model binding.
The following fixed-layout follow-up records the common-byte-prefix boundary.
Absent-zero reads have source-level coverage only; bulk zero/proof-shape actuals
remain a separate low-priority TODO.

The new quote executable is`401a7e3d…`, built from actual Raiko`200ccc8e` dirty /
Reth`5d6e281` dirty. Its production source is unchanged by the separately reviewed
test-only domain/golden updates. Later commits retain this original build label.
The cached host checks do not constitute a new guest or SDK build.


## Fixed-layout follow-up under unchanged EEA

Three separate, prospectively fixed panels use the same genuine two-block
construction: Anchor-only prefix, then Anchor and89 users each reading128
initialized body slots and writing only its phase. Each reference has word0;
each sole successor uses that reference's qualified process-shared G. Each
panel is closed after those two inputs, with no measurement retry or further
count, seed, layout or observation-window adaptation.

For raw body keys `(i << ordinal_shift) | (word << 64)`, all six inputs consume
28,062,336 charged EVM gas:167,355 and27,894,981 by block. With11,393 physical
keys including the phase, the geometry is:

| Layout | Common leading LE bytes | Largest raw low32 group M | C |
| --- | ---: | ---: | ---: |
|shift240|30|11,393|11,393|
|shift0|0|1|1,425|
|shift24, the32−8 bit boundary|3|45|1,464|

This geometry is the pricing proxy, not a measurement of a complete physical
hash-table population, probe count or comparison work.

| Layout/input | Actual SP1 PGU | Literal node PGU | Full proposal PGU |
| --- | ---: | ---: | ---: |
|shift240 reference|534,935,515|27,788,992,523|28,169,446,953|
|shift240 successor|13,774,743,290|27,788,992,523|28,168,681,753|
|shift0 reference|535,182,243|3,731,822,603|4,112,318,681|
|shift0 successor|3,158,837,743|3,731,822,603|4,112,413,505|
|shift24 reference|535,323,577|3,827,450,123|4,208,110,769|
|shift24 successor|4,219,423,570|3,827,450,123|4,207,560,689|

The long-prefix and dispersed successors are covered. The three-byte-prefix
successor is **underquoted**: actual minus literal node is391,973,447PGU, and
actual minus full proposal is11,862,881PGU. Actual/node is1.1024x. At this
measured28.062M-gas point, the node deficit remains below the agreed high
priority screen of strictly more than2BPGU at no more than30M actual charged
gas. Keep this deliberate layout residual as a low-priority TODO and hold
EEA/J+2X/384 unchanged. This classification is a complexity-versus-cost choice,
not a proof that another input cannot exceed that screen.

At the readonly follow-up checkpoint, this storage/account subset has92 unique
inputs/9,654blocks and39 numeric actuals:34 node quotes and38 full quotes cover
actual. Five positive node gaps remain; the largest is391,973,447PGU. One full
quote is below actual by11,862,881PGU. No measured node gap exceeds2B. The three
unmeasured controls and50 historical proposals/9,600blocks without current
numeric actuals remain excluded from accuracy claims. The latter still provide
only price sensitivity; this count does not summarize the whole opcode campaign.

All six new inputs passed independent canonical signatures, state/root, VM,
witness/Kona and quote checks, followed by independent actual/resource review.
Each successor independently qualified245 retained declared-key-member local
chains with matching ownG and fullH0. This does not qualify semantic owner,
transaction, physical target-body count, whole-map placement or isolated-map
PGU. In the shift0 reference, six earlier key aliases are retained by the
original minimum-member selector; its251 members are not251 target-body calls.
The generic G qualification is for that one process/input; a later input must
qualify its own observed G again.

The shift24 actual's raw gas is80,590,990,195, normalized by `*10//191` to
4,219,423,570PGU. Its complete canonical execution has4,977,089,097instructions;
the400M optional observation is incomplete and sample-capped. The whole
measurement takes120.078seconds and peaks at4,578,564,096 process-tree PSS bytes,
within the fixed6GiB/1,200-second guard. Independent original-host readback finds
the exact producer and four owned shared-memory objects absent, with no signals
or unlink operations. The raw report SHA256 is
`cf2560972717784e8f009ad71ebe8691992d9821c8c9572623ea9354e3f8a246`.

The permanent readonly recipe now strictly accepts shifts0/24/48/240; default48
and existing0/240 requests remain byte-identical. The focused module passes33
tests with one existing native-binary skip. Independent literal VM verification
covers42 tiny recipes/342transactions,21 manual ordinal boundaries and207
failure guards. No guest, SDK, ELF, observer or pricing implementation was
rebuilt or changed for these fixed-layout follow-ups. Quote401a, native1368 and
b76/SDK6.3 retain the artifact scopes and dirty-build labels above.

The [long-prefix fixture](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-prefixed-storage-long-read-v1)
and [dispersed fixture](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-dispersed-storage-read-v1)
retain immutable original data and documented integrity checks. The shift24
pair is retained locally pending the next data-only Atlas fixture checkpoint;
it is not yet represented by either published asset. Existing release assets
are not rewritten. Bulk-zero/proof-shape and custom/preseeded/failed-backend
paths remain explicit TODOs rather than inferred coverage.


## Dense writes and final state construction

A separate closed64×64 pair combines30 common LE bytes with initialized1→2
writes. It reuses the two genuine blocks with user counts[0,64]. Every user
changes64 body keys and the phase; each transaction has J=X=65 for the target
owner. The phase advances1→4097 and4,096 body keys become2. The target-only
storage reserve is25,970,880units×384=9,972,817,920PGU; other components and
Anchors remain in the complete quoted totals below.

| Input | Actual SP1 PGU | Literal node PGU | Full proposal PGU |
| --- | ---: | ---: | ---: |
|dirty240 reference|363,071,660|10,971,432,738|11,274,506,921|
|dirty240 sole successor|9,001,709,767|10,971,432,738|11,274,773,545|

Both consume23,541,753 charged EVM gas, split167,355/23,374,398; this fixed
point does not fill30M gas. The successor is covered at1.218817x node/actual
and1.252515x full/actual, with signed gaps−1,969,722,971/−2,273,063,778PGU.
EEA/J+2X/384 stays unchanged. The reference's approximately30x synthetic
node overquote is retained rather than reported as natural-traffic precision.

All independent prepared/data/actual/resource/profile gates pass. The successor
completes11,410,625,627 canonical instructions in242.8134seconds, with
4,563,782,656 process-tree PSS bytes. Raw gas171,932,656,563 normalizes by
`*10//191` to9,001,709,767PGU; raw report SHA256 is
`84024f2b5da675a442ad7921d355509f2d805e8610e9205905384d99437fc7ed`.
Its400M observation is incomplete and capped. The reference completes below
400M but still has capped samples. Independent reconstruction qualifies176
same-call declared-key members with actual matchingG/fullH0, followed by
comparison with all185 root local rows. It excludes71 incomplete intervals
and proves no owner/TX, physical-body, final-Bundle or whole-placement attribution.
Actual code-pool ordering differs between the two inputs; G was observed afresh.
No measurement, seed, layout, count or observation window was retried or extended.

The storage/account checkpoint now has94 unique inputs/9,658blocks and41
numeric actuals:36 node quotes and40 full quotes cover actual. The five positive
node gaps still peak at391,973,447PGU; the sole positive full gap is11,862,881PGU.
No measured node gap exceeds2B. The unmeasured controls/historical-only rows and
all frozen-ELF/SDK/native/dirty-build limits above continue to apply. This is
finite evidence, not a universal cost bound or a whole-opcode accuracy audit.

Two source checks clarify scope without changing the model. Each ordinary or
lab block replay creates a fresh local revm State/CacheState/TransitionState.
The immutable witness-node pool, headers and semantic state-root results may
be shared across blocks; raw PlainStorage map contents are not inherited.
`take_bundle()` extracts the bundle; the local State drops when that block's
execution returns. Thus a previous block's large physical raw cache does not
silently survive into the next block on these checked paths. Custom/preseeded
backends remain separately qualified TODOs.

The quote CLI's MPT observer has a different host-only cost: each dirty-slot
mutation exports/rebuilds a complete trie snapshot and compares the snapshots.
The4,097-dirty-slot quote takes108.8seconds; retained dirty48 quotes also take
about108–109seconds, while readonly240 takes about0.57seconds. This existing
observer work is not a demonstrated node-pricing-ledger regression or actual
SP1 PGU. No profiling split or metrics-on/off ablation was run. Optimizing that
observer is a separate tool TODO, with no source or guest change in this panel.

The bounded structural matrix has readonly common-byte prefixes0/3/6/30 and
dirty prefixes6/30 measured. Dirty0/3 remain unmeasured. The next distinct
question is fixed64×64 dirty24: the raw-low32 discount combined with all changed
slots and final state construction. This is a new finite cell, not another point
in a closed panel. No actual, price fix or30M worst-case bound is inferred from
the target-only reserve arithmetic. These two dirty inputs remain locally
retained for the next data-only Atlas checkpoint.
