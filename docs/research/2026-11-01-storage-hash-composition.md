# Dirty storage, hashing and caught frame rollback

This45-input validation panel keeps frozen model `c8936124…` and the same
experimental SDK6.3/ELFb76 program. It exposes a repeatable dirty-storage
rollback deficit. Own-N0 growth increments for direct execution, successful
child return and mixed/hash work are covered by both estimate surfaces in
this panel; their literal node deficits remain and node closure is not established.
No coefficient, admission limit, guest, SDK or proposal allocation is changed.

## Fixed inputs and independent gates

The [producer and usage](../../bin/zkgas/bench/README.md#storagehash-composition-and-caught-rollback)
retain27 base cases: dirty writes,32-zero-byte KECCAK, or their mixture;
direct, self-DELEGATECALL RETURN32 or caught REVERT32; N0/2048/32768.
Twelve packing cases keep mixed bulk rounds at32768 per block with4 or16
users, each with its own matching N0. Six mixed cases add B4 growth.
All45 inputs and18 controls remain distinct. Equal bulk rounds do not imply
equal signature, setup, frame or total proposal work.

Each transaction starts slot2 at its original zero, seeds5, enters the kernel
at5, writes7, optionally alternates11↔7, restores5 and clears0 in the parent.
Bulk dirty writes stay nonoriginal/nonzero. The parent prewarms the slot and
self target. This is not cold-first-child-access or warm-set rollback coverage.
Disjoint memory words hold hash input, counter, computed return marker and
child input tag; marker, exact returned length and child status are checked.

Independent native/quote qualification covers all45. The concrete oracle
checks every storage read and mutation, journal reversal, hash bytes, frame
status/data, packing nonces, final empty user storage, completed opcode maps,
normal dispatch and zero code preparation. Four negative probes remain:
wrong marker and child out-of-gas reject without publishing an input; wrong
initial read key and wrong dirty-write slot can pass actual native receipt
and opcode gates but fail the independent operand oracle.

Source review caught the missing ordered-read assertion in the original test
oracle. Its three-layout wrong-read-key regression first failed and then
passed after the test fix. Producer/request bytes did not change. Mandatory
hash and MPT oracles use a standard-library Ethereum Keccak implementation,
with known vectors, rate-boundary checks and separate optional external
cross-checks; CI does not acquire a new package dependency.

## Three quantities and measured results

All45 actual executions completed once. Actuals are whole-proposal execute-only
normalized SP1 PGU (`ExecutionReport.gas *10//191`), not proofs or GPU timings.
Native `b0497f45…`, quote `c10eae23…`, sampler `1998fb85…` and program manifest
`8d5b4a48…` remain unchanged. Manifest source binding is not a build attestation
or a production-program certification.

| Case | Node literal sum PGU | Full proposal estimate PGU | Actual proposal SP1 PGU |
| --- | ---: | ---: | ---: |
| Dirty,direct,N32768,B1 |167,215,774|322,157,861|278,566,745|
| Dirty,RETURN,N32768,B1 |167,248,024|322,190,110|278,584,276|
| Dirty,caught REVERT,N32768,B1 |172,242,891|327,184,977|328,498,441|
| Mixed,direct,N32768 per block,B4 |1,080,708,127|1,236,900,323|991,078,575|
| Mixed,RETURN,N32768 per block,B4 |1,080,837,123|1,237,029,320|991,157,626|
| Mixed,caught REVERT,N32768 per block,B4 |1,100,816,591|1,257,008,788|1,205,181,436|

Literal node deficits are43/45; full deficits are1/45. The two node-covered
inputs are the high-count mixed direct/RETURN B4 proposals. Every N0 control
retains a node deficit. The one full deficit is dirty REVERT N32768:1,313,464
PGU, about0.4014% above its full quote. Its node shortfall is156,255,550PGU,
and its literal actual/node ratio is about1.9072. These literal outcomes remain
primary even when a matched increment is covered.

Of27 own-N0 work comparisons,25 have covered node and full increments. The
two exceptions are dirty REVERT N2048 and N32768. At N32768, actual growth is
167,414,774PGU versus159,328,756.998 node/full quoted PGU: an8,086,017.002
increment shortfall. At N2048 it is9,406,528 versus8,834,220.212PGU.
All six selected packing comparisons and six B1→B4 comparisons have covered
node/full increments. These are differences of whole-proposal workloads,
not invented actual workloads assigned to individual transactions or blocks.

## Why the repeated rollback matters

Subtracting each layout's ownN0, caught REVERT adds49,912,634 more actual PGU
than RETURN at N32768; corresponding node/full price increments are identical.
This is an explanatory difference, not the entire unpriced amount: the direct
path's existing headroom absorbs about41.83M, leaving the8.09M growth deficit.
Do not label the whole49.91M as missing price or an isolated rollback tariff.

The selected REVM43 source adds a `StorageChanged` journal entry for each
successful value change and reverses entries individually on checkpoint revert.
Repeating changes on a single slot therefore grows journal work even though
the unique-write selector increases only once. REVERT's fixed5M reserve does
not grow with journal length. This source behavior and the controlled frame
difference support the attribution; dividing a PGU difference by writes is
not an isolated physical-cost measurement.

The current node hook already captures original/current/new SSTORE values
and the completed storage branch. A changed-SSTORE mutation selector can be
observed without an extra database read. A journal entry is consumed when
reverted; nesting does not replay that same drained entry repeatedly. A
conservative mutation allowance therefore deserves testing. Exact reverted
entry counts would need earlier checkpoint/frame capture, since the journal
has already been drained by the existing end hook.

The warm dirty typed value is approximately2145.594280PGU, with a raw100-gas
times12 floor and64-PGU normal dispatch. Actual route pricing first compares
accepted and incremental **route totals**, then adds dispatch. A correction
must respect that ownership; increasing one typed coefficient can be hidden
by the other route total. A completed changed-mutation allowance applied once
after the route maximum is a candidate design, not an implemented price here.
Freeze its rule and independently test fresh counts, branches and nested
rollback before claiming the deficit corrected; inspect historical sensitivity.

## Rounding and retained evidence

The old workload harness mislabeled aggregate-ceil node metrics as a sum.
For these six B4 measurements, literal sum of the four individually rounded
block prices is exactly2PGU higher. Raw statuses remain byte-exact. The tables,
ratios and deficits above use the actual literal sum, retain aggregate-ceil
separately, and record the original stored value and correction. No deficit
flag changes. Fixing future report/ranking semantics requires no SP1 rerun.

All180 owned shared-memory objects were verified absent after measurement.
Peak sampled process-tree PSS was4,583,143,424bytes, below the6GiB guard.
Keep both this panel and the separate
[cold-storage growth finding](2026-11-01-storage-access-growth.md) in immutable
fixture data, including controls, native-only negatives, raw reports and
source identities. Neither finding refits the frozen model in place.
